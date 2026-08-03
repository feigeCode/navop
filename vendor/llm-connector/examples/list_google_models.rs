//! List Google Gemini Models
//!
//! Run: cargo run --example list_google_models

use dotenvy::dotenv;
use llm_connector::LlmClient;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🔍 Listing Google Gemini Models...\n");

    let api_key = env::var("GOOGLE_API_KEY").expect("GOOGLE_API_KEY not set");
    let base_url = env::var("GOOGLE_BASE_URL")
        .unwrap_or_else(|_| "https://generativelanguage.googleapis.com/v1beta".to_string());

    let client = LlmClient::google(&api_key, &base_url)?;

    match client.models().await {
        Ok(models) => {
            println!("Available models:");
            for model in models {
                println!("  - {}", model);
            }
        }
        Err(e) => println!("❌ Failed to list models: {}\n", e),
    }

    Ok(())
}
