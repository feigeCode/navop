use llm_connector::client::LlmClient;
use llm_connector::types::{ChatRequest, Message};

#[tokio::test]
async fn test_providers_live() {
    dotenvy::dotenv().ok();

    let api_key = std::env::var("GATEWAY_API_KEY").unwrap_or_else(|_| "dummy_key".to_string());
    let base_url =
        std::env::var("GATEWAY_BASE_URL").unwrap_or_else(|_| "http://dummy_url".to_string());

    if api_key == "dummy_key" {
        println!("No gateway credentials found, skipping test.");
        return;
    }

    println!("Testing OpenAI Protocol...");
    let openai_client =
        LlmClient::openai(&api_key, &base_url).expect("Failed to create OpenAI client");

    let request =
        ChatRequest::new("gpt-5.2").add_message(Message::user("Please say just 'Hello OpenAI'"));

    match openai_client.chat(&request).await {
        Ok(response) => {
            println!("OpenAI Response Success!");
            println!("Response: {}", response.content);
        }
        Err(e) => println!("OpenAI Error: {:?}", e),
    }

    println!("\nTesting Anthropic Protocol...");
    let anthropic_client =
        LlmClient::anthropic(&api_key, &base_url).expect("Failed to create Anthropic client");

    let request = ChatRequest::new("claude-opus-4-5-20251101")
        .add_message(Message::user("Please say just 'Hello Anthropic'"));

    match anthropic_client.chat(&request).await {
        Ok(response) => {
            println!("Anthropic Response Success!");
            println!("Response: {}", response.content);
        }
        Err(e) => println!("Anthropic Error: {:?}", e),
    }
}
