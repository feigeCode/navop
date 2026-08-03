//! Moonshot (Kimi) Reasoning (Thinking) Example
//!
//! Demonstrates the reasoning/thinking capabilities of Kimi models on cn/global endpoints.
//! Note: cn/global keys may differ. Use a key that is valid for the selected region.
//!
//! Run: cargo run --example moonshot_thinking
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
    println!("🌙 Moonshot (Kimi) Reasoning (Thinking) Example\n");

    let api_key = env::var("MOONSHOT_API_KEY").expect("MOONSHOT_API_KEY not set");
    let region = env::var("MOONSHOT_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "cn".to_string());
    let model = env::var("MOONSHOT_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string());

    let endpoint_id = format!("moonshot:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    println!("📍 Region: {} ({})", endpoint.label, endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::builder()
        .moonshot(&api_key)
        .base_url(endpoint.base_url)
        .build()?;

    // 1. Request with thinking (if supported by model/endpoint)
    // For Kimi K2.5, thinking is often enabled by default or via model selection.
    // We also use enable_thinking=true as a hint for protocols that support it.
    println!("--- Step 1: Request with thinking enabled ---");
    let request = ChatRequest::new(&model)
        .add_message(Message::user("Solve this logic puzzle: If a farmer has 17 sheep and all but 9 run away, how many are left? Explain your reasoning."))
        .with_enable_thinking(true);

    let response = client.chat(&request).await?;

    // 2. Show reasoning content
    if let Some(reasoning) = response
        .choices
        .first()
        .and_then(|c| c.message.reasoning_any())
    {
        println!("🧠 Thinking Process:\n{}\n", reasoning);
    } else {
        println!(
            "⚠️ No explicit reasoning content returned in a separate field (the model might integrate reasoning directly into the response content)."
        );
    }

    println!("🏁 Final Answer:\n{}\n", response.content);

    Ok(())
}
