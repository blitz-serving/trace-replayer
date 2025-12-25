use std::collections::BTreeMap;

use reqwest::Response;

use super::LLMApi;

pub struct TGIApi;

impl Copy for TGIApi {}

impl Clone for TGIApi {
    fn clone(&self) -> Self {
        *self
    }
}

impl LLMApi for TGIApi {
    const AIBRIX_PRIVATE_HEADER: bool = false;

    fn request_json_body(prompt: String, output_length: u64) -> String {
        let json_body =
            serde_json::json!({"inputs":prompt,"parameters":{"max_new_tokens":output_length}});
        json_body.to_string()
    }

    fn parse_response(response: Response) -> BTreeMap<String, String> {
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
                .unwrap()
                .to_string();
            map.insert("first_token_time".to_string(), first_token_time);

            let total_time = response
                .headers()
                .get("x-total-time")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert("total_time".to_string(), total_time);

            let inference_time = response
                .headers()
                .get("x-inference-time")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert("inference_time".to_string(), inference_time);

            let queue_time = response
                .headers()
                .get("x-queue-time")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert("queue_time".to_string(), queue_time);

            let first_decode_token_time = response
                .headers()
                .get("x-first-decode-token-time")
                .map_or("nil".to_string(), |hv| hv.to_str().unwrap().to_string());
            map.insert(
                "first_decode_token_time".to_string(),
                first_decode_token_time,
            );

            let max_time_between_tokens_except_first = response
                .headers()
                .get("x-max-time-between-tokens-except-first")
                .map_or("nil".to_string(), |hv| hv.to_str().unwrap().to_string());
            map.insert(
                "max_time_between_tokens_except_first".to_string(),
                max_time_between_tokens_except_first,
            );

            let max_time_between_tokens = response
                .headers()
                .get("x-max-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert(
                "max_time_between_tokens".to_string(),
                max_time_between_tokens,
            );

            let avg_time_between_tokens = response
                .headers()
                .get("x-avg-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert(
                "avg_time_between_tokens".to_string(),
                avg_time_between_tokens,
            );

            let p90_time_between_tokens = response
                .headers()
                .get("x-p90-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert(
                "p90_time_between_tokens".to_string(),
                p90_time_between_tokens,
            );

            let p95_time_between_tokens = response
                .headers()
                .get("x-p95-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert(
                "p95_time_between_tokens".to_string(),
                p95_time_between_tokens,
            );

            let p99_time_between_tokens = response
                .headers()
                .get("x-p99-time-between-tokens")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert(
                "p99_time_between_tokens".to_string(),
                p99_time_between_tokens,
            );

            let output_length = response
                .headers()
                .get("x-output-length")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            map.insert("output_length".to_string(), output_length);
        }
        map
    }
}
