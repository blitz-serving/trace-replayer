use std::collections::BTreeMap;
use std::sync::OnceLock;
use reqwest::Response;
use serde_json::json;

use super::{LLMApi, MODEL_NAME};

#[derive(Copy, Clone)]
pub struct AIBrixApi;

pub static AIBRIX_ROUTE_STRATEGY: OnceLock<String> = OnceLock::new();

impl LLMApi for AIBrixApi {
    const AIBRIX_PRIVATE_HEADER: bool = true;

    /// 构造一个 OpenAI Chat Completions 风格的 JSON body
    fn request_json_body(prompt: String, output_length: u64) -> String {
        let json_body = json!({
            "model": MODEL_NAME.get().unwrap().as_str(), // 可按需修改
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "stream": false,
            "min_tokens": output_length,
            "max_tokens": output_length,
        });

        json_body.to_string()
    }

    /// 解析 OpenAI API 的响应
    /// 返回格式为 { "output": "...", "raw": "..." }
    fn parse_response(response: Response) -> BTreeMap<String, String> {
        let mut result = BTreeMap::new();

        result.insert("status".to_string(), response.status().as_str().to_string());

        result
    }
}
