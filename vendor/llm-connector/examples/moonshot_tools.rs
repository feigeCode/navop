//! Moonshot (Kimi) Tool Calling Example
//!
//! Demonstrates function calling with Kimi models on cn/global endpoints.
//! Note: cn/global keys may differ. Use a key that is valid for the selected region.
//!
//! Run: cargo run --example moonshot_tools
//! Recommended real-world verification: run without local proxy interference.

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message, Tool, ToolChoice},
};
use serde::Deserialize;
use std::env;

#[derive(Deserialize, Debug)]
struct WeatherArgs {
    location: String,
    _unit: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🌙 Moonshot (Kimi) Tool Calling Example\n");

    let api_key = env::var("MOONSHOT_API_KEY").expect("MOONSHOT_API_KEY not set");
    let region = env::var("MOONSHOT_REGION")
        .or_else(|_| env::var("REGION"))
        .unwrap_or_else(|_| "cn".to_string());
    let model = env::var("MOONSHOT_MODEL").unwrap_or_else(|_| "kimi-k2.5".to_string());

    let endpoint_id = format!("moonshot:{}", region);
    let (_, endpoint) = llm_providers::get_endpoint(&endpoint_id)
        .ok_or_else(|| format!("Endpoint {} not found", endpoint_id))?;

    println!("📍 Region: {} ({})", endpoint.label, endpoint.base_url);
    println!("🤖 Model: {}\n", model);

    let client = LlmClient::builder()
        .moonshot(&api_key)
        .base_url(endpoint.base_url)
        .build()?;

    // 1. Define tool
    let weather_tool = Tool::function(
        "get_weather",
        Some("Get the current weather in a given location".to_string()),
        serde_json::json!({
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

    // 2. Initial request
    println!("--- Step 1: Requesting tool ---");
    let mut messages = vec![Message::user("What's the weather like in Beijing?")];
    let request = ChatRequest::new(&model)
        .with_messages(messages.clone())
        .with_tools(vec![weather_tool])
        .with_tool_choice(ToolChoice::auto());

    let response = client.chat(&request).await?;

    // 3. Handle tool call
    if response.has_tool_calls() {
        let tool_call = &response.tool_calls()[0];
        let args: WeatherArgs = tool_call.parse_arguments()?;
        println!(
            "🛠️  Model requested tool: {} with args: {:?}",
            tool_call.function.name, args
        );

        // Add assistant's tool call to history
        messages.push(response.choices[0].message.clone());

        // Perform mock tool execution
        let tool_result = format!(
            "The weather in {} is 25 degrees Celsius and sunny.",
            args.location
        );
        println!("📡 Tool result: {}", tool_result);

        // Add tool result to history
        messages.push(Message::tool(tool_result, &tool_call.id));

        // 4. Final request
        println!("--- Step 2: Getting final answer ---");
        let final_request = ChatRequest::new(&model).with_messages(messages);

        let final_response = client.chat(&final_request).await?;
        println!("🤖 Final Answer: {}", final_response.content);
    } else {
        println!(
            "⚠️  Model did not call the tool. Response: {}",
            response.content
        );
    }

    Ok(())
}
