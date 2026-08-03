//! Google Gemini Example (V2)
//!
//! Demonstrates basic chat and streaming using Google Gemini.
//!
//! Run: cargo run --example google

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
use std::env;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Google Gemini Comprehensive Example\n");

    let api_key = env::var("GOOGLE_API_KEY").expect("GOOGLE_API_KEY not set");
    let region = env::var("GOOGLE_REGION").unwrap_or_else(|_| "global".to_string());

    // Fetch endpoint and default model from llm-providers
    let endpoint_id = format!("google:{}", region);
    let (provider_id, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    let base_url = env::var("GOOGLE_BASE_URL").unwrap_or_else(|_| endpoint.base_url.to_string());
    let model = env::var("GOOGLE_MODEL").unwrap_or_else(|_| {
        llm_providers::list_models(provider_id)
            .and_then(|m| m.first().cloned())
            .unwrap_or_else(|| "gemini-1.5-flash".to_string())
    });

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::google(&api_key, &base_url)?;

    // 1. Basic Chat
    println!("--- 1. Basic Chat ({}) ---", model);
    let request = ChatRequest::new(&model)
        .add_message(Message::user("What's interesting about the Big Bang?"));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    // 2. Streaming Chat
    #[cfg(feature = "streaming")]
    {
        println!("--- 2. Streaming Chat ---");
        let stream_request = ChatRequest::new(&model)
            .add_message(Message::user(
                "Describe the aurora borealis in one sentence.",
            ))
            .with_stream(true);

        let mut stream = client.chat_stream(&stream_request).await?;
        print!("Streaming: ");
        io::stdout().flush()?;

        while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
            let chunk = chunk?;
            print!("{}", chunk.content);
            io::stdout().flush()?;
        }
        println!("\n");
    }

    // 3. Reasoning (Thinking) with Gemini 2.0
    println!("--- 3. Reasoning (Thinking) with gemini-2.0-flash-thinking-exp ---");
    let thinking_model = "gemini-2.0-flash-thinking-exp";
    let thinking_request = ChatRequest::new(thinking_model)
        .add_message(Message::user(
            "Which is larger, 9.11 or 9.9? Explain with thinking process.",
        ))
        .with_enable_thinking(true);

    match client.chat(&thinking_request).await {
        Ok(response) => {
            if let Some(reasoning) = response.reasoning_content {
                println!("🧠 Thinking process:\n{}\n", reasoning);
            } else {
                println!("⚠️ No separate reasoning content returned.\n");
            }
            println!("Final Answer: {}\n", response.content);
        }
        Err(e) => println!("⚠️ Reasoning test failed: {}\n", e),
    }

    Ok(())
}
