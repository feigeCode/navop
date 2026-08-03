//! Minimax Multi-Region Testing Example
//!
//! Demonstrates connectivity and streaming with Minimax models
//! using endpoints from `llm-providers`.
//!
//! Run: cargo run --example minimax

use dotenvy::dotenv;
use futures_util::StreamExt;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
use std::env;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Minimax Multi-Region Example\n");

    let api_key = env::var("MINIMAX_API_KEY").expect("MINIMAX_API_KEY not set");
    let region = env::var("MINIMAX_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "global".to_string());
    let model = env::var("MINIMAX_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string());

    // Fetch endpoint from llm-providers
    let endpoint_id = format!("minimax:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    // Minimax uses OpenAI-compatible v1 API
    let client = LlmClient::builder()
        .openai_compatible(&api_key, "minimax")
        .base_url(endpoint.base_url)
        .build()?;

    // 1. Basic Chat
    println!("--- 1. Basic Chat (minimax) ---");
    let request = ChatRequest::new(&model)
        .add_message(Message::user("Hello! Who are you and which model are you?"));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    // 2. Streaming Chat
    println!("--- 2. Streaming Chat ---");
    let stream_request = ChatRequest::new(&model)
        .add_message(Message::user("Tell me a very short joke about AI."))
        .with_stream(true);

    let mut stream = client.chat_stream(&stream_request).await?;
    print!("Streaming: ");
    io::stdout().flush()?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(content) = chunk.get_content() {
            print!("{}", content);
            io::stdout().flush()?;
        }
    }
    println!("\n\nDone.");

    Ok(())
}
