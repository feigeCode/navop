//! Zhipu GLM Example (V2)
//!
//! Demonstrates multi-region (cn/global) testing using llm-providers.
//!
//! # Environment Variables
//! - ZHIPU_API_KEY: Your Zhipu API key
//! - ZHIPU_REGION: "cn" (BigModel) or "global" (z.ai). Falls back to REGION. Default is "cn".
//! - ZHIPU_MODEL: Model ID (e.g., "glm-4.5-flash", "glm-4.5"). Default is "glm-4.5-flash".
//! - ZHIPU_PROXY: Optional HTTP proxy (e.g., "http://127.0.0.1:7890")
//!
//! Run: cargo run --example zhipu
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
    println!("🤖 Zhipu GLM Comprehensive Multi-Region Example\n");

    let api_key = env::var("ZHIPU_API_KEY").expect("ZHIPU_API_KEY not set");
    let region = env::var("ZHIPU_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "cn".to_string());
    let model = env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-4.5-flash".to_string());
    let proxy = env::var("ZHIPU_PROXY").ok();

    // 1. Get region configuration from llm-providers
    let endpoint_id = format!("zhipu:{}", region);
    let (provider_id, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found in llm-providers", endpoint_id))?;

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", endpoint.base_url);
    println!("🤖 Model: {}", model);
    if let Some(ref p) = proxy {
        println!("🌐 Using Proxy: {}", p);
    }
    println!();

    // 2. Build client using LlmClient::builder()
    let mut builder = LlmClient::builder()
        .zhipu(&api_key)
        .base_url(endpoint.base_url);

    if let Some(p) = proxy {
        builder = builder.proxy(&p);
    }

    let client = builder.build()?;

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
                "Tell a very short story about a robot learning to cook.",
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
