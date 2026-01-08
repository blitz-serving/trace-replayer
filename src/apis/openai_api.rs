use super::{LLMApi, RequestError, MODEL_NAME};
use futures_util::TryStreamExt;
use reqwest::Response;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    time::{timeout as tokio_timeout, Instant as TokioInstant},
};
use tokio_util::io::StreamReader;

#[derive(Copy, Clone)]
pub struct OpenAIApi;

#[async_trait::async_trait]
impl LLMApi for OpenAIApi {
    const AIBRIX_PRIVATE_HEADER: bool = false;

    fn request_json_body(prompt: String, output_length: u64, stream: bool) -> String {
        let json_body = json!({
            "model": MODEL_NAME.get().unwrap().as_str(), // 可按需修改
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": stream,
            "min_tokens": output_length,
            "max_tokens": output_length,
        });

        json_body.to_string()
    }

    async fn parse_response(
        response: Response,
        stream: bool,
        timeout_duration: Duration,
    ) -> Result<BTreeMap<String, String>, RequestError> {
        let mut result = BTreeMap::new();
        result.insert("status".to_string(), response.status().as_str().to_string());

        if !stream {
            return Ok(result);
        }

        // 流式响应处理
        if !response.status().is_success() {
            return Ok(result);
        }

        let stream = response.bytes_stream();
        let stream_reader = StreamReader::new(
            stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
        );
        let mut reader = BufReader::new(stream_reader);
        let mut line = String::new();
        let mut first_token_time: Option<TokioInstant> = None;
        let mut last_token_time: Option<TokioInstant> = None;
        let mut token_count = 0;
        let mut tbt_values = Vec::new();
        let mut tbt_except_first = Vec::new();
        let start_time = TokioInstant::now();

        loop {
            if start_time.elapsed() > timeout_duration {
                return Err(RequestError::Timeout);
            }
            let remaining_duration = timeout_duration - start_time.elapsed();

            let read_future = reader.read_line(&mut line);
            match tokio_timeout(remaining_duration, read_future).await {
                Ok(Ok(0)) => break, // EOF
                Ok(Ok(_)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        line.clear();
                        continue;
                    }

                    if trimmed.starts_with("data: ") {
                        let data_str = &trimmed[6..];

                        if data_str == "[DONE]" {
                            break;
                        }
                        if data_str.contains(r#""delta""#) {
                            let now = TokioInstant::now();
                            token_count += 1;

                            if first_token_time.is_none() {
                                first_token_time = Some(now);
                                let first_token_duration =
                                    now.duration_since(start_time).as_millis() as u64;
                                result.insert(
                                    "first_token_time".to_string(),
                                    first_token_duration.to_string(),
                                );
                            } else if let Some(last) = last_token_time {
                                let tbt = now.duration_since(last).as_millis() as u64;
                                tbt_values.push(tbt);
                                if token_count > 2 {
                                    tbt_except_first.push(tbt);
                                }
                            }

                            last_token_time = Some(now);
                        }
                    }
                    line.clear();
                }
                Ok(Err(e)) => return Err(RequestError::StreamErr(e)),
                Err(_) => return Err(RequestError::Timeout),
            }
        }

        if let Some(first) = first_token_time {
            if let Some(last) = last_token_time {
                let total_time = last.duration_since(first).as_millis() as u64;
                result.insert("total_time".to_string(), total_time.to_string());
            }
        }

        if !tbt_except_first.is_empty() {
            let max_tbt_except_first = *tbt_except_first.iter().max().unwrap();
            result.insert(
                "max_time_between_tokens_except_first".to_string(),
                max_tbt_except_first.to_string(),
            );
        }

        if !tbt_values.is_empty() {
            let max_tbt = *tbt_values.iter().max().unwrap();
            result.insert("max_time_between_tokens".to_string(), max_tbt.to_string());
        }

        if !tbt_values.is_empty() {
            let avg_tbt = (tbt_values.iter().sum::<u64>() as f64 / tbt_values.len() as f64) as u32;
            result.insert("avg_time_between_tokens".to_string(), avg_tbt.to_string());
        }

        // p90_time_between_tokens, p95_time_between_tokens, p99_time_between_tokens
        // need to sort for computing percentage
        if !tbt_values.is_empty() {
            let mut sorted_tbt = tbt_values.clone();
            sorted_tbt.sort();

            let len = sorted_tbt.len();
            if len > 0 {
                // p90
                let p90_idx = (len as f64 * 0.9).ceil() as usize - 1;
                let p90_idx = p90_idx.min(len - 1);
                result.insert(
                    "p90_time_between_tokens".to_string(),
                    sorted_tbt[p90_idx].to_string(),
                );

                // p95
                let p95_idx = (len as f64 * 0.95).ceil() as usize - 1;
                let p95_idx = p95_idx.min(len - 1);
                result.insert(
                    "p95_time_between_tokens".to_string(),
                    sorted_tbt[p95_idx].to_string(),
                );

                // p99
                let p99_idx = (len as f64 * 0.99).ceil() as usize - 1;
                let p99_idx = p99_idx.min(len - 1);
                result.insert(
                    "p99_time_between_tokens".to_string(),
                    sorted_tbt[p99_idx].to_string(),
                );
            }
        }

        Ok(result)
    }
}
