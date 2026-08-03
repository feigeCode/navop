//! Tencent connectivity and reasoning test
//! Run: TENCENT_SECRET_ID=... TENCENT_SECRET_KEY=... cargo run --example tencent_test --features tencent

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
        println!("🤖 Tencent Hunyuan Connectivity & Reasoning Test\n");

        let secret_id = env::var("TENCENT_SECRET_ID").expect("TENCENT_SECRET_ID not set");
        let secret_key = env::var("TENCENT_SECRET_KEY").expect("TENCENT_SECRET_KEY not set");
        
        // Use default endpoint
        let client = LlmClient::tencent(&secret_id, &secret_key, "")?;

        println!("--- 1. Basic Chat (hunyuan-lite) ---");
        let request = ChatRequest::new("hunyuan-lite")
            .add_message(Message::user("Hello, who are you?"));

        match client.chat(&request).await {
            Ok(response) => println!("✅ Basic Chat Success: {}\n", response.content),
            Err(e) => println!("❌ Basic Chat Failed: {:?}\n", e),
        }

        println!("--- 2. Reasoning Test (hunyuan-lite usually supports search/think if enabled) ---");
        // Note: Tencent sometimes requires specific models for reasoning, we use the flag to check provider compatibility
        let request = ChatRequest::new("hunyuan-lite")
            .add_message(Message::user("Explain quantum entanglement briefly."))
            .with_enable_thinking(true);

        match client.chat(&request).await {
            Ok(response) => {
                println!("✅ Reasoning Request Success!");
                println!("Content: {}", response.content);
                if let Some(reasoning) = response.reasoning_content {
                    println!("Reasoning: {}", reasoning);
                } else {
                    println!("(No separate reasoning field returned, might be embedded or unsupported by this model)");
                }
            },
            Err(e) => println!("❌ Reasoning Request Failed: {:?}\n", e),
        }
    }
    
    #[cfg(not(feature = "tencent"))]
    println!("Please run with --features tencent");

    Ok(())
}
