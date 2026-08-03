//! Zhipu GLM-5 Tool Calling Example
//!
//! Demonstrates function calling (tools) using Zhipu GLM-5 on cn/global endpoints.
//!
//! Run: cargo run --example zhipu_tools
//! Recommended real-world verification: run without local proxy interference.

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message, Tool},
};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Zhipu GLM-5 Tool Calling Example\n");

    let api_key = env::var("ZHIPU_API_KEY").expect("ZHIPU_API_KEY not set");
    let region = env::var("ZHIPU_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "global".to_string());
    let model = env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-5".to_string());

    // 1. Get endpoint from llm-providers
    let endpoint_id = format!("zhipu:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    println!("📍 Region: {} ({})", endpoint.label, endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::builder()
        .zhipu(&api_key)
        .base_url(endpoint.base_url)
        .build()?;

    // 2. Define a mock tool (Get weather)
    let weather_tool = Tool::function(
        "get_weather",
        Some("Get the current weather in a given location".to_string()),
        json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city and state, e.g. San Francisco, CA"
                },
                "unit": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"]
                }
            },
            "required": ["location"]
        }),
    );

    // 3. Initial request
    println!("--- Step 1: User asks a question requiring a tool ---");
    let mut request = ChatRequest::new(&model)
        .add_message(Message::user("What's the weather like in San Francisco?"))
        .with_tools(vec![weather_tool]);

    let response = client.chat(&request).await?;

    // Show reasoning if present
    if let Some(reasoning) = response
        .choices
        .first()
        .and_then(|c| c.message.reasoning_any())
    {
        println!("🧠 Reasoning:\n{}\n", reasoning);
    }

    if response.has_tool_calls() {
        let tool_calls = response.tool_calls();
        println!("🛠️ Model requested {} tool call(s):", tool_calls.len());

        // Add the assistant's tool call message to history
        if let Some(choice) = response.choices.first() {
            request.messages.push(choice.message.clone());
        }

        for tool_call in tool_calls {
            let name = &tool_call.function.name;
            let args = &tool_call.function.arguments;
            println!("  - Function: {}, Args: {}", name, args);

            // 4. Mock tool execution
            if name == "get_weather" {
                let result = json!({
                    "location": "San Francisco",
                    "temperature": "18",
                    "unit": "celsius",
                    "description": "Partly cloudy"
                });

                println!("✅ Executed tool: {} -> {}", name, result);

                // Add tool result to history
                request
                    .messages
                    .push(Message::tool(result.to_string(), &tool_call.id));
            }
        }

        // 5. Final request to get the answer
        println!("\n--- Step 2: Sending tool result back to model ---");
        let final_response = client.chat(&request).await?;

        println!("🏁 Final Answer:\n{}\n", final_response.content);
    } else {
        println!("Response: {}\n", response.content);
    }

    Ok(())
}
