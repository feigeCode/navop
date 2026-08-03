//! Xinference Example
//!
//! Demonstrates chat and model listing against Xinference OpenAI-compatible API.
//!
//! Run: cargo run --example xinference

use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Xinference Example\n");

    // Typical local Xinference OpenAI-compatible endpoint
    let client = LlmClient::xinference("http://127.0.0.1:9997/v1")?;

    println!("--- 1. List Models ---");
    let models = match client.models().await {
        Ok(models) => {
            println!("Available models: {:?}", models);
            models
        }
        Err(e) => {
            println!("Could not list models: {}", e);
            return Ok(());
        }
    };

    let model = match models.first() {
        Some(m) => m.clone(),
        None => {
            println!("No model found in Xinference instance.");
            return Ok(());
        }
    };

    println!("Using model: {}\n", model);

    println!("--- 2. Basic Chat ---");
    let request = ChatRequest::new(model).add_message(Message::user("Reply with one word: pong"));

    let response = client.chat(&request).await?;
    println!("Response: {}", response.content);

    Ok(())
}
