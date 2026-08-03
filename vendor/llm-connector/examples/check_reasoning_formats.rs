//! Example to check reasoning formats across different providers.
//!
//! Run: cargo run --example check_reasoning_formats

use dotenvy::dotenv;
use llm_connector::{LlmClient, types::ChatRequest};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let providers = vec![
        (
            "zhipu",
            "ZHIPU_API_KEY",
            "ZHIPU_MODEL",
            "glm-5",
            "https://api.z.ai/api/paas/v4",
        ),
        (
            "moonshot",
            "MOONSHOT_API_KEY",
            "MOONSHOT_MODEL",
            "kimi-k2.5",
            "https://api.moonshot.ai/v1",
        ),
        (
            "minimax",
            "MINIMAX_API_KEY",
            "MINIMAX_REGION",
            "global",
            "https://api.minimax.io/v1",
        ),
    ];

    for (name, key_env, model_env, default_model, base_url) in providers {
        println!("\n--- Testing {} ---", name);
        let api_key = match env::var(key_env) {
            Ok(k) => k,
            Err(_) => {
                println!("⚠️ Skip {}: {} not set", name, key_env);
                continue;
            }
        };

        let model = match name {
            "minimax" => "MiniMax-M2.5".to_string(), // Stick to M2.5 for consistency
            _ => env::var(model_env).unwrap_or(default_model.to_string()),
        };

        // Minor hack for minimax region if it was passed instead of model
        let actual_base_url = if name == "minimax" {
            "https://api.minimax.io/v1".to_string()
        } else {
            base_url.to_string()
        };

        let client = LlmClient::builder()
            .openai_compatible(&api_key, name)
            .base_url(&actual_base_url)
            .build()?;

        let request = ChatRequest::new(&model)
            .add_message(llm_connector::types::Message::user(
                "Which is larger, 9.11 or 9.9? Explain briefly.",
            ))
            .with_enable_thinking(true);

        println!("📡 Sending request to {} (Model: {})...", name, model);
        let response = client.chat(&request).await?;

        println!("✅ Response received.");
        println!("   Content length: {}", response.content.len());
        println!(
            "   Has <think> in content: {}",
            response.content.contains("<think>")
        );
        println!(
            "   Reasoning field present: {}",
            response.reasoning_content.is_some()
        );

        if let Some(r) = &response.reasoning_content {
            println!("   Reasoning field length: {}", r.len());
            println!(
                "   Reasoning start: {}...",
                &r[..std::cmp::min(50, r.len())].replace("\n", " ")
            );
        }

        println!(
            "   Content start: {}...",
            &response.content[..std::cmp::min(50, response.content.len())].replace("\n", " ")
        );
    }

    Ok(())
}
