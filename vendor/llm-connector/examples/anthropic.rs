//! Anthropic Claude Example (V2)
//!
//! Demonstrates streaming and system message handling.
//!
//! Run: cargo run --example anthropic

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Anthropic Claude Comprehensive Example\n");

    let api_key = env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY not set");
    let base_url =
        env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let client = LlmClient::anthropic(&api_key, &base_url)?;

    println!("--- 1. Chat with System Message ---");
    let request = ChatRequest::new("claude-opus-4-5-20251101")
        .add_message(Message::system(
            "You are a helpful assistant that speaks like a pirate.",
        ))
        .add_message(Message::user("How's the weather today?"));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    #[cfg(feature = "streaming")]
    {
        println!("--- 2. Streaming Chat ---");
        let request = ChatRequest::new("claude-opus-4-5-20251101")
            .add_message(Message::user(
                "Explain the concept of monads in functional programming.",
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
