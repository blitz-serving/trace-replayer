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
    task::JoinHandle,
    time::sleep,
};

use crate::{
    apis::{LLMApi, RequestError, AIBRIX_ROUTE_STRATEGY},
    dataset::LLMTrace,
    timeout_secs_upon_slo,
    token_sampler::TokenSampler,
};
use crate::apis::METRIC_PERCENTILES;

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
    stream: bool,
) -> Result<BTreeMap<String, String>, RequestError> {
    let mut req = client
        .post(endpoint)
        .timeout(timeout)
        .body(json_body)
        .header("Content-Type", "application/json");

    if A::AIBRIX_PRIVATE_HEADER {
        req = req.header(
            "routing-strategy",
            AIBRIX_ROUTE_STRATEGY.get().unwrap().as_str(),
        );
    }

    let response = req.send().await.map_err(|e| RequestError::Other(e))?;

    A::parse_response(response, stream, timeout).await
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
    stream: bool,
) -> JoinHandle<Result<(), i32>> {
    static BASETIME: OnceLock<Instant> = OnceLock::new();
    static RETURNCODE: AtomicI32 = AtomicI32::new(0);
    BASETIME.get_or_init(|| Instant::now());
    fn get_timestamp() -> f64 {
        BASETIME.get().unwrap().elapsed().as_secs_f64() * 1000.0
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

            let curr_timestamp = get_timestamp() as u64;
            let next_timestamp = ((*dataset).timestamp(data_index) as f64 / scale_factor) as u64;

            if next_timestamp > curr_timestamp + 1 {
                sleep(Duration::from_millis(next_timestamp - curr_timestamp)).await;
            }

            // Do not parse in another coroutine to avoid sync/async lock contention
            let (prompt, input_length, output_length) =
                dataset.inflate(data_index, token_sampler.as_ref());

            let request_handle = spawn(async move {
                let json_body = A::request_json_body(prompt, output_length, stream);
                let s_time = get_timestamp();
                let s_time_drift = s_time - next_timestamp as f64;
                match post_with_timeout::<A>(
                    client,
                    endpoint.as_str(),
                    json_body.to_string(),
                    Duration::from_secs(timeout_secs_upon_slo(output_length, ttft_slo, tpot_slo)),
                    stream,
                )
                .await
                {
                    Ok(mut metrics) => {
                        let e_time = get_timestamp();

                        metrics.insert("s_time".to_string(), format!("{s_time:.3}"));
                        metrics.insert("s_time_drift".to_string(), format!("{s_time_drift:.3}"));
                        metrics.insert("e_time".to_string(), format!("{e_time:.3}"));
                        metrics.insert("input_length".to_string(), input_length.to_string());
                        metrics.insert("output_length".to_string(), output_length.to_string());

                        let span_time = e_time - s_time;
                        metrics.insert(
                            "span_time".to_string(),
                            format!("{span_time:.3}"),
                        );
                        response_sender.send(metrics).unwrap();
                    }
                    Err(RequestError::Timeout) => {
                        let e_time = get_timestamp();

                        let mut metrics = BTreeMap::<String, String>::from([(
                            "status".to_owned(),
                            "timeout".to_owned(),
                        )]);
                        metrics.insert("s_time".to_string(), format!("{s_time:.3}"));
                        metrics.insert("s_time_drift".to_string(), format!("{s_time_drift:.3}"));
                        metrics.insert("e_time".to_string(), format!("{e_time:.3}"));
                        metrics.insert("input_length".to_string(), input_length.to_string());
                        metrics.insert("output_length".to_string(), output_length.to_string());

                        let span_time = e_time - s_time;
                        metrics.insert(
                            "span_time".to_string(),
                            format!("{span_time:.3}"),
                        );
                        response_sender.send(metrics).unwrap();
                    }
                    Err(RequestError::Other(error)) => {
                        tracing::error!(
                            "Request#{data_index}::({input_length}|{output_length}) error: {error}",
                        );
                    }
                    Err(RequestError::StreamErr(error)) => {
                        tracing::error!(
                            "Request#{data_index}::({input_length}|{output_length}) stream error: {error}",
                        );
                    }
                }
            });

            tx.send_async(request_handle).await.unwrap();
        }
        tracing::debug!("Requester exited.");
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

    fn get_timestamp() -> f64 {
        BASETIME.get().unwrap().elapsed().as_secs_f64() * 1000.0
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

            let curr_timestamp = get_timestamp() as u64;
            // milisecond
            let next_timestamp = ((*dataset).timestamp(data_index) as f64 / scale_factor) as u64;

            if next_timestamp > curr_timestamp + 1 {
                sleep(Duration::from_millis(next_timestamp - curr_timestamp)).await;
            }

            let (sample, input_length, output_length) =
                dataset.inflate(data_index, token_sampler.as_ref());

            let request_handle = spawn(async move {
                let s_time = get_timestamp();
                let s_time_drift = s_time - next_timestamp as f64;

                let validate_len = tokenizer
                    .encode(sample.clone(), false)
                    .unwrap()
                    .get_ids()
                    .len();
                if validate_len != input_length as usize {
                    tracing::error!("Validation error: {input_length} :> {validate_len}");
                }

                let mut metrics = BTreeMap::new();
                metrics.insert("chat_id".to_string(), data_index.to_string());
                metrics.insert("input_length".to_string(), input_length.to_string());
                metrics.insert("output_length".to_string(), output_length.to_string());
                metrics.insert("s_time".to_string(), format!("{s_time:.3}"));
                metrics.insert("s_time_drift".to_string(), format!("{s_time_drift:.3}"));

                response_sender.send(metrics).unwrap();
            });

            tx.send_async(request_handle).await.unwrap();
        }
        tracing::debug!("Requester exited.");
    });

    handle
}

/// The report loop writes the metrics to a file in JSONL format.
///
/// Report loop exits when the response receiver is closed.
pub async fn report_loop(
    mut output_jsonl_file: File,
    mut summary_json_file: File,
    response_receiver: flume::Receiver<BTreeMap<String, String>>,
) {
    let mut buf_writer = BufWriter::new(&mut output_jsonl_file);
    let mut summary = SummaryStats::new();
    while let Ok(metrics) = response_receiver.recv_async().await {
        summary.record(&metrics);
        let line = serde_json::to_string(&metrics).unwrap();
        buf_writer.write_all(line.as_bytes()).await.unwrap();
        buf_writer.write_all(b"\n").await.unwrap();
        buf_writer.flush().await.unwrap();
    }
    if let Some(metrics) = summary.finalize() {
        let line = serde_json::to_string_pretty(&metrics).unwrap();
        summary_json_file.write_all(line.as_bytes()).await.unwrap();
        summary_json_file.write_all(b"\n").await.unwrap();
        summary_json_file.flush().await.unwrap();
    }
}

struct SummaryStats {
    total_requests: u64,
    success_requests: u64,
    total_output_tokens: u64,
    min_s_time: Option<f64>,
    max_e_time: Option<f64>,
    ttft_values: Vec<f64>,
    tpot_values: Vec<f64>,
    e2e_values: Vec<f64>,
}

impl SummaryStats {
    fn new() -> Self {
        Self {
            total_requests: 0,
            success_requests: 0,
            total_output_tokens: 0,
            min_s_time: None,
            max_e_time: None,
            ttft_values: Vec::new(),
            tpot_values: Vec::new(),
            e2e_values: Vec::new(),
        }
    }

    fn record(&mut self, metrics: &BTreeMap<String, String>) {
        self.total_requests += 1;

        if let Some(status) = metrics.get("status") {
            if status
                .parse::<u16>()
                .map(|code| (200..300).contains(&code))
                .unwrap_or(false)
            {
                self.success_requests += 1;
            }
        }

        if let Some(output_length) = metrics.get("output_length").and_then(|v| v.parse().ok()) {
            self.total_output_tokens = self.total_output_tokens.saturating_add(output_length);
        }

        if let Some(s_time) = metrics.get("s_time").and_then(|v| v.parse().ok()) {
            self.min_s_time = Some(self.min_s_time.map_or(s_time, |min| min.min(s_time)));
        }
        if let Some(e_time) = metrics.get("e_time").and_then(|v| v.parse().ok()) {
            self.max_e_time = Some(self.max_e_time.map_or(e_time, |max| max.max(e_time)));
        }

        if let Some(ttft) = metrics.get("first_token_time").and_then(|v| v.parse().ok()) {
            self.ttft_values.push(ttft);
        }
        if let (Some(total_time), Some(output_length)) = (
            metrics.get("total_time").and_then(|v| v.parse::<f64>().ok()),
            metrics
                .get("output_length")
                .and_then(|v| v.parse::<f64>().ok()),
        ) {
            if output_length > 0.0 {
                self.tpot_values.push(total_time / output_length);
            }
        }
        if let Some(e2e) = metrics.get("span_time").and_then(|v| v.parse().ok()) {
            self.e2e_values.push(e2e);
        }
    }

    fn finalize(&mut self) -> Option<BTreeMap<String, String>> {
        if self.total_requests == 0 {
            return None;
        }

        let percentiles = METRIC_PERCENTILES
            .get()
            .map(|v| v.as_slice())
            .unwrap_or(&[90, 95, 99]);

        let mut summary = BTreeMap::new();
        summary.insert(
            "requests_total".to_string(),
            self.total_requests.to_string(),
        );
        summary.insert(
            "requests_success".to_string(),
            self.success_requests.to_string(),
        );
        summary.insert(
            "output_tokens_total".to_string(),
            self.total_output_tokens.to_string(),
        );

        let duration_ms = match (self.min_s_time, self.max_e_time) {
            (Some(start), Some(end)) if end >= start => end - start,
            _ => 0.0,
        };
        summary.insert("duration_ms".to_string(), format!("{duration_ms:.3}"));
        if duration_ms > 0.0 {
            let duration_secs = duration_ms / 1000.0;
            summary.insert(
                "throughput_rps".to_string(),
                format!("{:.3}", self.total_requests as f64 / duration_secs),
            );
            summary.insert(
                "throughput_tps".to_string(),
                format!("{:.3}", self.total_output_tokens as f64 / duration_secs),
            );
        }

        let ttft = compute_percentiles(&mut self.ttft_values, percentiles);
        for (percentile, value) in ttft {
            summary.insert(format!("ttft_p{percentile}_ms"), format_ms(value));
        }
        let tpot = compute_percentiles(&mut self.tpot_values, percentiles);
        for (percentile, value) in tpot {
            summary.insert(format!("tpot_p{percentile}_ms"), format_ms(value));
        }
        let e2e = compute_percentiles(&mut self.e2e_values, percentiles);
        for (percentile, value) in e2e {
            summary.insert(format!("e2e_p{percentile}_ms"), format_ms(value));
        }

        summary.insert(
            "ttft_mean_ms".to_string(),
            format_ms(mean(&self.ttft_values)),
        );
        summary.insert(
            "tpot_mean_ms".to_string(),
            format_ms(mean(&self.tpot_values)),
        );
        summary.insert(
            "e2e_mean_ms".to_string(),
            format_ms(mean(&self.e2e_values)),
        );

        Some(summary)
    }
}

fn compute_percentiles(values: &mut Vec<f64>, percentiles: &[u32]) -> Vec<(u32, f64)> {
    if values.is_empty() {
        return Vec::new();
    }
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let len = values.len();
    percentiles
        .iter()
        .map(|percentile| {
            let idx = (len as f64 * (*percentile as f64 / 100.0)).ceil() as isize - 1;
            let idx = idx.max(0) as usize;
            let idx = idx.min(len - 1);
            (*percentile, values[idx])
        })
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn format_ms(value: f64) -> String {
    format!("{:.3}", value)
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
        let summary_file = File::create("tmp/summary.json").await.unwrap();
        let reporter = tokio::spawn(report_loop(output_file, summary_file, rx));

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
