//! OpenAI Example (V2)
//!
//! Demonstrates basic chat, streaming, and embeddings.
//!
//! Run: cargo run --example openai

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, EmbedRequest, Message, ResponsesRequest},
};
#[allow(unused_imports)]
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 OpenAI Comprehensive Example\n");

    let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    let base_url =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let client = LlmClient::openai(&api_key, &base_url)?;

    println!("--- 1. Basic Chat ---");
    let request = ChatRequest::new("gpt-5.2-2025-12-11").add_message(Message::user(
        "Explain quantum entanglement in one sentence.",
    ));

    let response = client.chat(&request).await?;
    println!("Response: {}\n", response.content);

    #[cfg(feature = "streaming")]
    {
        println!("--- 2. Streaming Chat ---");
        let request = ChatRequest::new("gpt-5.2-2025-12-11")
            .add_message(Message::user("Count from 1 to 5."))
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

    println!("--- 3. Responses API ---");
    let responses_request = ResponsesRequest {
        model: "gpt-5.2-2025-12-11".to_string(),
        input: Some(serde_json::json!("Write one short sentence about Rust.")),
        ..Default::default()
    };
    let responses = client.invoke_responses(&responses_request).await?;
    println!("Responses output: {}\n", responses.output_text);

    #[cfg(feature = "streaming")]
    {
        println!("--- 4. Responses Streaming ---");
        let request = ResponsesRequest {
            model: "gpt-5.2-2025-12-11".to_string(),
            input: Some(serde_json::json!("Count 1 to 5")),
            stream: Some(true),
            ..Default::default()
        };

        let mut stream = client.invoke_responses_stream(&request).await?;
        print!("Responses streaming: ");
        while let Some(event) = futures_util::StreamExt::next(&mut stream).await {
            let event = event?;
            if event.event_type == "response.output_text.delta"
                && let Some(delta) = event.data.get("delta").and_then(|v| v.as_str())
            {
                print!("{}", delta);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
        }
        println!("\n");
    }

    println!("--- 5. Embeddings ---");
    let embed_request = EmbedRequest::new("text-embedding-3-small", "Hello world");
    let embed_response = client.embed(&embed_request).await?;
    println!(
        "Embedding vector size: {}\n",
        embed_response.data[0].embedding.len()
    );

    Ok(())
}
