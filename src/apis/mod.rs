use reqwest::Response;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub static MODEL_NAME: OnceLock<String> = OnceLock::new();
pub static METRIC_PERCENTILES: OnceLock<Vec<u32>> = OnceLock::new();

pub mod aibrix_api;
pub mod openai_api;
pub mod tgi_api;

pub use aibrix_api::{AIBrixApi, AIBRIX_ROUTE_STRATEGY};
pub use openai_api::OpenAIApi;
pub use tgi_api::TGIApi;

use std::time::Duration;

pub enum RequestError {
    Timeout,
    StreamErr(std::io::Error),
    Other(reqwest::Error),
}

#[async_trait::async_trait]
pub trait LLMApi: Copy + Clone + Send + Sync {
    const AIBRIX_PRIVATE_HEADER: bool;
    fn request_json_body(prompt: String, output_length: u64, stream: bool) -> String;
    async fn parse_response(
        response: Response,
        stream: bool,
        timeout_duration: Duration,
    ) -> Result<BTreeMap<String, String>, RequestError>;
}
