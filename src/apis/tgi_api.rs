use std::collections::BTreeMap;

use reqwest::Response;

use super::{LLMApi, RequestError, METRIC_PERCENTILES};
use std::time::Duration;
pub struct TGIApi;

const DEFAULT_PERCENTILES: [u32; 3] = [90, 95, 99];

impl Copy for TGIApi {}

impl Clone for TGIApi {
    fn clone(&self) -> Self {
        *self
    }
}

fn normalize_ms(value: &str) -> String {
    value
        .parse::<f64>()
        .map(|v| format!("{v:.3}"))
        .unwrap_or_else(|_| value.to_string())
}

#[async_trait::async_trait]
impl LLMApi for TGIApi {
    const AIBRIX_PRIVATE_HEADER: bool = false;

    fn request_json_body(prompt: String, output_length: u64, _stream: bool) -> String {
        let json_body =
            serde_json::json!({"inputs":prompt,"parameters":{"max_new_tokens":output_length}});
        json_body.to_string()
    }

    async fn parse_response(
        response: Response,
        _stream: bool,
        _timeout_duration: Duration,
    ) -> Result<BTreeMap<String, String>, RequestError> {
        let mut map = BTreeMap::new();
        map.insert("status".to_string(), response.status().as_str().to_string());
        if response.status().is_success() {
            let request_id = response
                .headers()
                .get("x-request-id")
                .map_or("nil".to_string(), |hv| hv.to_str().unwrap().to_string());
            map.insert("request_id".to_string(), request_id);

            let first_token_time = response
                .headers()
                .get("x-first-token-time")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert(
                "first_token_time".to_string(),
                normalize_ms(first_token_time),
            );

            let total_time = response
                .headers()
                .get("x-total-time")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert("total_time".to_string(), normalize_ms(total_time));

            let inference_time = response
                .headers()
                .get("x-inference-time")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert("inference_time".to_string(), normalize_ms(inference_time));

            let queue_time = response
                .headers()
                .get("x-queue-time")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert("queue_time".to_string(), normalize_ms(queue_time));

            let first_decode_token_time = response
                .headers()
                .get("x-first-decode-token-time")
                .map_or("nil".to_string(), |hv| hv.to_str().unwrap().to_string());
            map.insert(
                "first_decode_token_time".to_string(),
                if first_decode_token_time == "nil" {
                    first_decode_token_time
                } else {
                    normalize_ms(&first_decode_token_time)
                },
            );

            let max_time_between_tokens_except_first = response
                .headers()
                .get("x-max-time-between-tokens-except-first")
                .map_or("nil".to_string(), |hv| hv.to_str().unwrap().to_string());
            map.insert(
                "max_time_between_tokens_except_first".to_string(),
                if max_time_between_tokens_except_first == "nil" {
                    max_time_between_tokens_except_first
                } else {
                    normalize_ms(&max_time_between_tokens_except_first)
                },
            );

            let max_time_between_tokens = response
                .headers()
                .get("x-max-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert(
                "max_time_between_tokens".to_string(),
                normalize_ms(max_time_between_tokens),
            );

            let avg_time_between_tokens = response
                .headers()
                .get("x-avg-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap();
            map.insert(
                "avg_time_between_tokens".to_string(),
                normalize_ms(avg_time_between_tokens),
            );

            let percentiles = METRIC_PERCENTILES
                .get()
                .map(|v| v.as_slice())
                .unwrap_or(&DEFAULT_PERCENTILES);
            for percentile in percentiles {
                let header_name = match percentile {
                    90 => "x-p90-time-between-tokens",
                    95 => "x-p95-time-between-tokens",
                    99 => "x-p99-time-between-tokens",
                    _ => continue,
                };
                if let Some(value) = response.headers().get(header_name) {
                    let value_str = value.to_str().unwrap();
                    map.insert(
                        format!("p{percentile}_time_between_tokens"),
                        normalize_ms(value_str),
                    );
                }
            }

            let output_length = response
                .headers()
                .get("x-output-length")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert("output_length".to_string(), output_length);
        }
        Ok(map)
    }
}
