//! Zhipu GLM-5 Reasoning (Thinking) Example
//!
//! Demonstrates the `enable_thinking` feature with GLM-5 on cn/global endpoints.
//!
//! Run: cargo run --example zhipu_thinking
//! Recommended real-world verification: run without local proxy interference.

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Zhipu GLM-5 Reasoning (Thinking) Example\n");

    let api_key = env::var("ZHIPU_API_KEY").expect("ZHIPU_API_KEY not set");
    let region = env::var("ZHIPU_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "global".to_string());
    let model = env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-5".to_string());

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

    // 2. Prepare request with enable_thinking
    println!("--- Step 1: Request with thinking enabled ---");
    let request = ChatRequest::new(&model)
        .add_message(Message::user(
            "Solve this math problem step by step: What is the derivative of x^2 * sin(x)?",
        ))
        .with_enable_thinking(true);

    let response = client.chat(&request).await?;

    // 3. Show reasoning content
    if let Some(reasoning) = response
        .choices
        .first()
        .and_then(|c| c.message.reasoning_any())
    {
        println!("🧠 Thinking Process:\n{}\n", reasoning);
    } else {
        println!(
            "⚠️ No explicit reasoning content returned (maybe the model integrates it into the response or doesn't support the 'thinking' parameter yet)."
        );
    }

    println!("🏁 Final Answer:\n{}\n", response.content);

    Ok(())
}
