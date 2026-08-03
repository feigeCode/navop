//! Aliyun Qwen Tool Calling Example
//!
//! Run: cargo run --example aliyun_tools

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message, Tool, ToolChoice},
};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🤖 Aliyun Qwen Tool Calling Example\n");

    let api_key = env::var("ALIYUN_API_KEY").expect("ALIYUN_API_KEY not set");
    let base_url = env::var("ALIYUN_BASE_URL")
        .unwrap_or_else(|_| "https://dashscope.aliyuncs.com".to_string());

    let client = LlmClient::aliyun(&api_key, &base_url)?;

    // 1. Define tool
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

    println!("--- Step 1: Requesting tool ---");
    let messages = vec![Message::user("What's the weather like in Hangzhou?")];
    let request = ChatRequest::new("qwen-max")
        .with_messages(messages.clone())
        .with_tools(vec![weather_tool])
        .with_tool_choice(ToolChoice::auto());

    let response = client.chat(&request).await?;

    if response.has_tool_calls() {
        let tool_call = &response.tool_calls()[0];
        println!(
            "🛠️  Model requested tool: {} with args: {}",
            tool_call.function.name, tool_call.function.arguments
        );

        // Add assistant's tool call to history
        let mut messages = messages.clone();
        messages.push(response.choices[0].message.clone());

        // Perform mock tool execution
        let tool_result = json!({
            "location": "Hangzhou",
            "temperature": "22",
            "unit": "celsius",
            "description": "Mostly sunny"
        });
        println!("📡 Tool result: {}", tool_result);

        // Add tool result to history
        messages.push(Message::tool(tool_result.to_string(), &tool_call.id));

        // 4. Final request
        println!("--- Step 2: Getting final answer ---");
        let final_request = ChatRequest::new("qwen-max").with_messages(messages);

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
