//! DeepSeek Example (V2)
//!
//! Run: cargo run --example deepseek

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 DeepSeek Multi-Region Example\n");

    let api_key = env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY not set");
    let region = env::var("DEEPSEEK_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "global".to_string());

    let model = env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());

    // Fetch endpoint from llm-providers
    let endpoint_id = format!("deepseek:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found in llm-providers", endpoint_id))?;

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::openai(&api_key, endpoint.base_url)?;

    println!("--- 1. Basic Chat ---");
    let request =
        ChatRequest::new(&model).add_message(Message::user("Hello DeepSeek, what's new today?"));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    Ok(())
}
