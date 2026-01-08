use reqwest::Response;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use super::{LLMApi, RequestError, MODEL_NAME};

#[derive(Copy, Clone)]
pub struct AIBrixApi;

pub static AIBRIX_ROUTE_STRATEGY: OnceLock<String> = OnceLock::new();

#[async_trait::async_trait]
impl LLMApi for AIBrixApi {
    const AIBRIX_PRIVATE_HEADER: bool = true;

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
        _stream: bool,
        _timeout_duration: Duration,
    ) -> Result<BTreeMap<String, String>, RequestError> {
        let mut result = BTreeMap::new();

        result.insert("status".to_string(), response.status().as_str().to_string());

        Ok(result)
    }
}
