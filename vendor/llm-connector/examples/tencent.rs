//! Tencent Hunyuan Example (V2)
//!
//! Demonstrates basic chat and streaming using Tencent Hunyuan.
//!
//! Run: cargo run --example tencent --features tencent

#[cfg(feature = "tencent")]
use dotenvy::dotenv;
#[cfg(feature = "tencent")]
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message},
};
#[cfg(feature = "tencent")]
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "tencent")]
    {
        dotenv().ok();
        println!("🤖 Tencent Hunyuan Comprehensive Example\n");

        let secret_id = env::var("TENCENT_SECRET_ID").unwrap_or_else(|_| {
            println!("❌ Please set TENCENT_SECRET_ID in .env or environment");
            std::process::exit(1);
        });
        let secret_key = env::var("TENCENT_SECRET_KEY").unwrap_or_else(|_| {
            println!("❌ Please set TENCENT_SECRET_KEY in .env or environment");
            std::process::exit(1);
        });
        let base_url = env::var("TENCENT_BASE_URL")
            .unwrap_or_else(|_| "hunyuan.tencentcloudapi.com".to_string());

        let client = LlmClient::tencent(&secret_id, &secret_key, &base_url)?;

        println!("--- 1. Basic Chat ---");
        let request = ChatRequest::new("hunyuan-lite").add_message(Message::user(
            "Hello Tencent, describe the city of Shenzhen.",
        ));

        let response = client.chat(&request).await?;
        println!("Response: {}\n", response.content);

        #[cfg(feature = "streaming")]
        {
            println!("--- 2. Streaming Chat ---");
            let request = ChatRequest::new("hunyuan-lite")
                .add_message(Message::user("Tell me a fun fact about pandas."))
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
    }

    #[cfg(not(feature = "tencent"))]
    {
        println!("❌ The 'tencent' feature needs to be enabled to run this example");
        println!("   Please use: cargo run --example tencent --features tencent");
    }

    Ok(())
}
