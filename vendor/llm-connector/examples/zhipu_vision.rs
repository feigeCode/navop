//! Zhipu Vision Example
//!
//! Demonstrates multi-modal (image) support using Zhipu GLM-4V or GLM-5 on cn/global endpoints.
//!
//! Run: cargo run --example zhipu_vision
//! Recommended real-world verification: run without local proxy interference.

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message, MessageBlock},
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Zhipu Vision/Multi-modal Example\n");

    let api_key = env::var("ZHIPU_API_KEY").expect("ZHIPU_API_KEY not set");
    let region = env::var("ZHIPU_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "global".to_string());

    // For vision, GLM-4V is standard, but GLM-5 should support it too.
    let model = env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-4v".to_string());

    // 1. Get endpoint from llm-providers
    let endpoint_id = format!("zhipu:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    println!("📍 Region: {} ({})", endpoint.label, endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::builder()
        .zhipu(&api_key)
        .base_url(endpoint.base_url)
        .build()?;

    // 2. Prepare multi-modal message
    // Using a sample image from the internet
    let image_url = "https://img1.baidu.com/it/u=1361506484,475200373&fm=253&fmt=auto&app=138&f=JPEG?w=800&h=500";

    println!("🖼️ Analyzing image: {}", image_url);

    let message = Message::new(
        llm_connector::types::Role::User,
        vec![
            MessageBlock::text("What do you see in this image? Describe it briefly."),
            MessageBlock::image_url(image_url),
        ],
    );

    let request = ChatRequest::new(&model).add_message(message);

    // 3. Send request
    let response = client.chat(&request).await?;

    println!("\n🏁 Assistant Response:\n{}\n", response.content);

    Ok(())
}
