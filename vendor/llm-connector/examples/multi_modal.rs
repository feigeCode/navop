//! Multi-modal Example (V2)
//!
//! Demonstrates sending images and text in a single request across multiple providers,
//! and using embedding API.
//!
//! Run: cargo run --example multi_modal

use dotenvy::dotenv;
use futures::StreamExt;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, EmbedRequest, Message, MessageBlock, Role},
};
#[allow(unused_imports)]
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🖼️ Multi-modal Chat and Embeddings Example\n");

    // We'll use OpenAI for this example
    let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    let client = LlmClient::openai(&api_key, "https://api.openai.com/v1")?;

    #[cfg(feature = "streaming")]
    {
        println!("--- 1. Multi-modal Streaming Chat ---");
        let request = ChatRequest::new("gpt-4o")
            .add_message(Message::new(Role::User, vec![
                MessageBlock::text("What do you see in this image?"),
                MessageBlock::image_url("https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/1200px-Cat03.jpg"),
            ]))
            .with_stream(true);

        let mut stream = client.chat_stream(&request).await?;
        print!("AI Analysis: ");
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            print!("{}", chunk.content);
            std::io::Write::flush(&mut std::io::stdout())?;
        }
        println!("\n");
    }

    println!("--- 2. Embeddings ---");
    let embed_request = EmbedRequest::new("text-embedding-3-small", "Hello world");
    let embed_response = client.embed(&embed_request).await?;
    println!(
        "Embedding vector first 5: {:?}",
        &embed_response.data[0].embedding[0..5]
    );

    Ok(())
}
