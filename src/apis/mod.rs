use reqwest::Response;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub static MODEL_NAME: OnceLock<String> = OnceLock::new();

pub mod aibrix_api;
pub mod openai_api;
pub mod tgi_api;

pub use aibrix_api::{AIBrixApi, AIBRIX_ROUTE_STRATEGY};
pub use openai_api::OpenAIApi;
pub use tgi_api::TGIApi;

pub trait LLMApi: Copy + Clone {
    const AIBRIX_PRIVATE_HEADER: bool;
    fn request_json_body(prompt: String, output_length: u64) -> String;
    fn parse_response(response: Response) -> BTreeMap<String, String>;
}
