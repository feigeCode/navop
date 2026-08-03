//! Ollama Example (V2)
//!
//! Demonstrates basic/multi-modal chat, streaming, and model listing using local Ollama.
//!
//! Run: cargo run --example ollama

use {
    base64::prelude::*,
    llm_connector::{
        LlmClient, Role,
        types::{ChatRequest, Message, MessageBlock},
    },
    std::error::Error,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🤖 Ollama Local Example\n");

    let client = LlmClient::ollama("http://localhost:11434")?;
    let mut selected_model = "gemma4:e4b".to_string();

    println!("--- 1. List Local Models ---");
    match client.models().await {
        Ok(models) => {
            println!("Available models: {:?}", models);
            if let Some(model) = models.iter().find(|m| m.as_str() == "kimi-k2.5:cloud") {
                selected_model = model.clone();
            } else if let Some(model) = models.first() {
                selected_model = model.clone();
            }
            println!("Using model: {}\n", selected_model);
        }
        Err(e) => println!(
            "⚠️ Could not list models (Ollama might not be running): {}\n",
            e
        ),
    }

    println!("--- 2. Basic Chat ---");
    basic(&client, &selected_model).await?;
    println!("--- 3. Multi-modal Chat ---");
    multimodal(&client, &selected_model).await?;

    #[cfg(feature = "streaming")]
    {
        println!("--- 4. Streaming Chat ---");
        let request = ChatRequest::new(selected_model)
            .add_message(Message::user("Tell me a joke about programming."))
            .with_stream(true);

        match client.chat_stream(&request).await {
            Ok(mut stream) => {
                print!("Streaming: ");
                while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                    match chunk {
                        Ok(chunk) => {
                            print!("{}", chunk.content);
                            std::io::Write::flush(&mut std::io::stdout())?;
                        }
                        Err(e) => {
                            println!("\n⚠️ Streaming chunk parse failed: {}", e);
                            break;
                        }
                    }
                }
                println!("\n");
            }
            Err(e) => println!("⚠️ Streaming failed: {}\n", e),
        }
    }

    Ok(())
}

async fn basic(client: &LlmClient, model: &str) -> Result<(), Box<dyn Error>> {
    let request = ChatRequest::new(model)
        .add_message(Message::user("Why is Rust so popular?"))
        .with_stream(false);

    let res = client.chat(&request).await?;
    println!("Response: {}\n", res.content);

    Ok(())
}

async fn multimodal(client: &LlmClient, model: &str) -> Result<(), Box<dyn Error>> {
    let img = BASE64_STANDARD.encode(include_bytes!("math.png"));
    let request = ChatRequest::new(model)
        .add_message(Message::new(
            Role::User,
            vec![
                MessageBlock::text("请回答图像中的问题"),
                MessageBlock::image_base64("image/png", img),
            ],
        ))
        .with_stream(false);

    let res = client.chat(&request).await?;
    println!("{}", res.content);

    Ok(())
}
