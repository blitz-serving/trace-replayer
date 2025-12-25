use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use reqwest::Response;
use tokio::{
    fs::File,
    io::{AsyncWriteExt, BufWriter},
    spawn,
    task::{yield_now, JoinHandle},
    time::sleep,
};

use crate::{
    apis::{LLMApi, AIBRIX_ROUTE_STRATEGY},
    dataset::LLMTrace,
    timeout_secs_upon_slo,
    token_sampler::TokenSampler,
};

#[allow(dead_code)]
async fn request(endpoint: &str, json_body: String) -> Result<Response, reqwest::Error> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .build()?
        .post(endpoint)
        .body(json_body)
        .header("Content-Type", "application/json")
        .send()
        .await?)
}

async fn post_with_timeout<A: 'static + LLMApi + Send>(
    client: reqwest::Client,
    endpoint: &str,
    json_body: String,
    timeout: Duration,
) -> Result<Response, reqwest::Error> {
    if A::AIBRIX_PRIVATE_HEADER {
        Ok(client
            .post(endpoint)
            .timeout(timeout)
            .body(json_body)
            .header("Content-Type", "application/json")
            .header("routing-strategy", AIBRIX_ROUTE_STRATEGY.get().unwrap().as_str())
            .send()
            .await?)
    } else {
        Ok(client
            .post(endpoint)
            .timeout(timeout)
            .body(json_body)
            .header("Content-Type", "application/json")
            .send()
            .await?)
    }
}

async fn wait_all(handle_rx: flume::Receiver<JoinHandle<()>>, interrupt_flag: Arc<AtomicBool>) {
    while let Ok(handle) = handle_rx.recv_async().await {
        handle.await.unwrap();
        if interrupt_flag.load(Ordering::Relaxed) {
            tracing::info!("{} requests has not yet finished!", handle_rx.len());
        }
    }
}

pub fn spawn_request_loop_with_timestamp<A: 'static + LLMApi + Send>(
    endpoint: String,
    dataset: Arc<Pin<Box<dyn LLMTrace>>>,
    token_sampler: Arc<TokenSampler>,
    scale_factor: f64,
    response_sender: flume::Sender<BTreeMap<String, String>>,
    interrupt_flag: Arc<AtomicBool>,
    ttft_slo: f32,
    tpot_slo: f32,
) -> JoinHandle<Result<(), i32>> {
    static BASETIME: OnceLock<Instant> = OnceLock::new();
    static RETURNCODE: AtomicI32 = AtomicI32::new(0);
    BASETIME.get_or_init(|| Instant::now());
    fn get_timestamp() -> u64 {
        BASETIME.get().unwrap().elapsed().as_millis() as u64
    }

    let rr = dataset.rps();
    println!("Origin request rate: {:.3} req/s", rr);
    println!("Scaled request rate: {:.3} req/s", rr * scale_factor);

    let (tx, rx) = flume::unbounded();
    let flag = Arc::clone(&interrupt_flag);
    let handle = spawn(async move {
        wait_all(rx, flag).await;
        let a = RETURNCODE.load(Ordering::Relaxed);
        if a == 0 {
            Ok(())
        } else {
            Err(a)
        }
    });

    spawn(async move {
        let data_iter = dataset.iter();
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(32)
            .pool_idle_timeout(Duration::from_secs(30))
            .no_proxy()
            .timeout(Duration::from_secs(15)) // default timeout, can be overrided
            .build()
            .unwrap();
        let endpoint = Arc::new(endpoint);
        for data_index in data_iter {
            if interrupt_flag.load(Ordering::Relaxed) {
                break;
            }
            let client = http_client.clone();
            let endpoint = endpoint.clone();
            let response_sender = response_sender.clone();

            let curr_timestamp = get_timestamp();
            let next_timestamp = ((*dataset).timestamp(data_index) as f64 / scale_factor) as u64;

            if next_timestamp > curr_timestamp + 1 {
                sleep(Duration::from_millis(next_timestamp - curr_timestamp)).await;
            }

            // Do not parse in another coroutine to avoid sync/async lock contention
            let (prompt, input_length, output_length) =
                dataset.inflate(data_index, token_sampler.as_ref());

            let request_handle = spawn(async move {
                let json_body = A::request_json_body(prompt, output_length);
                let s_time = get_timestamp();
                let s_time_drift = s_time.saturating_sub(next_timestamp);
                match post_with_timeout::<A>(
                    client,
                    endpoint.as_str(),
                    json_body.to_string(),
                    Duration::from_secs(timeout_secs_upon_slo(output_length, ttft_slo, tpot_slo)),
                )
                .await
                {
                    Ok(response) => {
                        let e_time = get_timestamp();

                        let mut metrics = A::parse_response(response);
                        metrics.insert("s_time".to_string(), s_time.to_string());
                        metrics.insert("s_time_drift".to_string(), s_time_drift.to_string());
                        metrics.insert("e_time".to_string(), e_time.to_string());
                        metrics.insert("input_length".to_string(), input_length.to_string());
                        metrics.insert("output_length".to_string(), output_length.to_string());

                        let span_time = e_time - s_time;
                        metrics.insert("span_time".to_string(), span_time.to_string());
                        response_sender.send(metrics).unwrap();
                    }
                    Err(error) if error.is_timeout() => {
                        let e_time = get_timestamp();

                        let mut metrics = BTreeMap::<String, String>::from([(
                            "status".to_owned(),
                            "timeout".to_owned(),
                        )]);
                        metrics.insert("s_time".to_string(), s_time.to_string());
                        metrics.insert("s_time_drift".to_string(), s_time_drift.to_string());
                        metrics.insert("e_time".to_string(), e_time.to_string());
                        metrics.insert("input_length".to_string(), input_length.to_string());
                        metrics.insert("output_length".to_string(), output_length.to_string());

                        let span_time = e_time - s_time;
                        metrics.insert("span_time".to_string(), span_time.to_string());
                        response_sender.send(metrics).unwrap();
                    }
                    Err(error) => {
                        tracing::error!(
                            "Request#{data_index}::({input_length}|{output_length}) error: {error}",
                        );
                    }
                }
            });

            tx.send_async(request_handle).await.unwrap();
        }
        tracing::info!("Requester exited.");
    });
    handle
}

pub fn spawn_request_loop_debug<A: 'static + LLMApi + Send>(
    _endpoint: String, // 保留参数，为了接口一致
    dataset: Arc<Pin<Box<dyn LLMTrace>>>,
    token_sampler: Arc<TokenSampler>,
    scale_factor: f64,
    response_sender: flume::Sender<BTreeMap<String, String>>,
    interrupt_flag: Arc<AtomicBool>,
) -> JoinHandle<Result<(), i32>> {
    use std::time::Instant;
    static BASETIME: OnceLock<Instant> = OnceLock::new();
    static RETURNCODE: AtomicI32 = AtomicI32::new(0);
    BASETIME.get_or_init(|| Instant::now());

    fn get_timestamp() -> u64 {
        BASETIME.get().unwrap().elapsed().as_millis() as u64
    }

    let rr = dataset.rps();
    println!("Origin request rate: {:.3} req/s", rr);
    println!(
        "Scaled request rate (release-with-debug mode, no HTTP): {:.3} req/s",
        rr * scale_factor
    );

    let (tx, rx) = flume::unbounded();
    let flag = Arc::clone(&interrupt_flag);
    let handle = spawn(async move {
        wait_all(rx, flag).await;
        let a = RETURNCODE.load(Ordering::Relaxed);
        if a == 0 {
            Ok(())
        } else {
            Err(a)
        }
    });

    let validate_tokenizer = Arc::new(token_sampler.get_tokenizer());

    spawn(async move {
        let data_iter = dataset.iter();
        for data_index in data_iter {
            if interrupt_flag.load(Ordering::Relaxed) {
                break;
            }
            let tokenizer = validate_tokenizer.clone();
            let response_sender = response_sender.clone();

            let curr_timestamp = get_timestamp();
            // milisecond
            let next_timestamp = ((*dataset).timestamp(data_index) as f64 / scale_factor) as u64;

            if next_timestamp > curr_timestamp + 1 {
                sleep(Duration::from_millis(next_timestamp - curr_timestamp)).await;
            }

            let (sample, input_length, output_length) =
                dataset.inflate(data_index, token_sampler.as_ref());

            let request_handle = spawn(async move {
                let s_time = get_timestamp();
                let s_time_drift = s_time.saturating_sub(next_timestamp);

                let validate_len = tokenizer.encode(sample.clone(), false).unwrap().get_ids().len();
                if validate_len != input_length as usize {
                    tracing::error!("Validation error: {input_length} :> {validate_len}");
                }

                let mut metrics = BTreeMap::new();
                metrics.insert("chat_id".to_string(), data_index.to_string());
                metrics.insert("input_length".to_string(), input_length.to_string());
                metrics.insert("output_length".to_string(), output_length.to_string());
                metrics.insert("s_time".to_string(), s_time.to_string());
                metrics.insert("s_time_drift".to_string(), s_time_drift.to_string());

                response_sender.send(metrics).unwrap();
            });

            tx.send_async(request_handle).await.unwrap();
        }
        tracing::info!("Requester exited.");
    });

    handle
}

/// The report loop writes the metrics to a file in JSONL format.
///
/// Report loop exits when the response receiver is closed.
pub async fn report_loop(
    mut output_jsonl_file: File,
    response_receiver: flume::Receiver<BTreeMap<String, String>>,
) {
    let mut buf_writer = BufWriter::new(&mut output_jsonl_file);
    while let Ok(metrics) = response_receiver.recv_async().await {
        let line = serde_json::to_string(&metrics).unwrap();
        buf_writer.write_all(line.as_bytes()).await.unwrap();
        buf_writer.write_all(b"\n").await.unwrap();
        buf_writer.flush().await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dataset::{BailianDataset, LLMTrace},
        token_sampler::TokenSampler,
    };
    use std::sync::Arc;
    use tokenizers::Tokenizer;
    use tokio::fs::File;

    #[tokio::test]
    async fn test_inflate_latency() {
        // 初始化 tracing 输出
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);

        // ====== 准备 dataset ======
        let mut dataset = BailianDataset::new();
        dataset.load("/Users/zdy/Workspace/Rust/request-sim/data/qwen-bailian-usagetraces-anon-main/qwen_traceA_blksz_16.jsonl"); // 你要准备一个小的测试文件

        let dataset = Arc::new(Box::pin(dataset) as Pin<Box<dyn LLMTrace>>);

        // ====== 准备 TokenSampler ======
        let token_sampler = Arc::new(TokenSampler::new(
            Tokenizer::from_file("/Users/zdy/Workspace/Rust/request-sim/data/tokenizer.json")
                .unwrap(),
            "/Users/zdy/Workspace/Rust/request-sim/data/tokenizer_config.json".to_string(),
            4,   // num_producer
            128, // capacity
            16,  // block size
        ));

        // ====== 准备输出通道 ======
        let (tx, rx) = flume::unbounded();
        let output_file = File::create("tmp/inflate_latency.jsonl").await.unwrap();
        let reporter = tokio::spawn(report_loop(output_file, rx));

        // ====== 测试循环 ======
        let iter = dataset.iter();
        for index in iter.take(10) {
            // 只测前10条
            let start = std::time::Instant::now();
            let (_prompt, input_len, output_len) = dataset.inflate(index, &token_sampler);
            let elapsed_us = start.elapsed().as_micros() as u64;

            let mut metrics = std::collections::BTreeMap::new();
            metrics.insert("index".to_string(), index.to_string());
            metrics.insert("input_length".to_string(), input_len.to_string());
            metrics.insert("output_length".to_string(), output_len.to_string());
            metrics.insert("inflate_time_us".to_string(), elapsed_us.to_string());
            tx.send_async(metrics).await.unwrap();
        }

        drop(tx);
        reporter.await.unwrap();

        tracing::info!("Inflate latency test completed");
    }
}
