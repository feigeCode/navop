//! Google Gemini Tools & Thinking Example
//!
//! Demonstrates tool calling and thinking process (reasoning) together.
//!
//! Run: cargo run --example google_tools_thinking

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
    println!("🤖 Google Gemini Full Feature Test (Tools & Thinking)\n");

    let api_key = env::var("GOOGLE_API_KEY").expect("GOOGLE_API_KEY not set");
    let region = env::var("GOOGLE_REGION").unwrap_or_else(|_| "global".to_string());

    // Fetch endpoint from llm-providers
    let endpoint_id = format!("google:{}", region);
    let (_provider_id, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    let base_url = env::var("GOOGLE_BASE_URL").unwrap_or_else(|_| endpoint.base_url.to_string());

    // We'll use gemini-3-flash-preview for tools, and try to enable thinking
    let model = env::var("GOOGLE_MODEL").unwrap_or_else(|_| "gemini-3-flash-preview".to_string());

    println!("📍 Testing Region: {}", endpoint.label);
    println!("🔗 Base URL: {}", base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::google(&api_key, &base_url)?;

    // 1. Define a tool
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

    // 2. Initial request with tools and thinking enabled
    println!("--- Step 1: Request with Tools & Thinking ---");
    let request = ChatRequest::new(&model)
        .add_message(Message::user(
            "What's the weather like in Beijing right now? Solve this by thinking carefully.",
        ))
        .with_tools(vec![weather_tool])
        .with_enable_thinking(true);

    let response = client.chat(&request).await?;

    if let Some(reasoning) = &response.reasoning_content {
        println!("🧠 Thinking Process:\n{}\n", reasoning);
    }

    if let Some(tool_calls) = &response.choices[0].message.tool_calls {
        println!("🛠️ Tool Calls:");
        for tc in tool_calls {
            println!("  - Function: {}", tc.function.name);
            println!("  - Arguments: {}", tc.function.arguments);

            // 3. Mock tool execution
            println!("\n--- Step 2: Executing Tool ---");
            let tool_result = json!({"location": "Beijing", "temperature": "15", "unit": "celsius", "description": "Sunny"}).to_string();
            println!("Tool Result: {}", tool_result);

            // 4. Send tool result back
            println!("\n--- Step 3: Final Response ---");
            let mut messages = request.messages.clone();
            messages.push(response.choices[0].message.clone()); // Assistant's tool call
            messages.push(Message::tool(tool_result, &tc.id)); // Tool result

            let final_request = ChatRequest::new(&model)
                .with_messages(messages)
                .with_tools(request.tools.clone().unwrap())
                .with_enable_thinking(true);

            let final_response = client.chat(&final_request).await?;

            if let Some(final_reasoning) = &final_response.reasoning_content {
                println!("🧠 Final Thinking Process:\n{}\n", final_reasoning);
            }
            println!("Final Answer: {}\n", final_response.content);
        }
    } else {
        println!("⚠️ No tool calls were made.");
        println!("Response: {}\n", response.content);
    }

    Ok(())
}
