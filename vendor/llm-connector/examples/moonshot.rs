//! Moonshot (Kimi) Example
//!
//! Demonstrates multi-region (cn/global) testing for Moonshot using llm-providers.
//!
//! # Environment Variables
//! - MOONSHOT_API_KEY: Your Moonshot API key
//! - MOONSHOT_REGION: "cn" (api.moonshot.cn) or "global" (api.moonshot.ai). Falls back to REGION. Default is "cn".
//! - MOONSHOT_MODEL: Model ID (e.g., "kimi-k2.5"). Default is "kimi-k2.5".
//! - Note: cn/global keys may differ. Use a key that is valid for the selected region.
//!
//! Run: cargo run --example moonshot
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
    println!("🌙 Moonshot (Kimi) Multi-Region Example\n");

    let api_key = env::var("MOONSHOT_API_KEY").expect("MOONSHOT_API_KEY not set");
    let region = env::var("MOONSHOT_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "cn".to_string());
    let model = env::var("MOONSHOT_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string());

    // 1. Get region configuration from llm-providers
    let endpoint_id = format!("moonshot:{}", region);
    let (provider_id, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found in llm-providers", endpoint_id))?;

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    // 2. Build client using LlmClient::builder()
    let client = LlmClient::builder()
        .moonshot(&api_key)
        .base_url(endpoint.base_url)
        .build()?;

    println!("--- 1. Basic Chat ({}) ---", provider_id);
    let request =
        ChatRequest::new(&model).add_message(Message::user("Introduce yourself in a few words."));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    #[cfg(feature = "streaming")]
    {
        println!("--- 2. Streaming Chat ---");
        let request = ChatRequest::new(&model)
            .add_message(Message::user(
                "Tell a very short story about Kimi the explorer.",
            ))
            .with_stream(true);

        let mut stream = client.chat_stream(&request).await?;
        print!("Streaming: ");
        while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
            let chunk = chunk?;
            print!("{}", chunk.content);
            std::io::Write::flush(&mut std::io::stdout())?;
        }
        println!("\n");
    }

    Ok(())
}
