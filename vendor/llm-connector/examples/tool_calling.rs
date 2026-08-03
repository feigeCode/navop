//! Tool Calling (Function Calling) Example (V2)
//!
//! Demonstrates how to define tools, send them in a request,
//! and handle the tool call response from the assistant.
//!
//! Run: cargo run --example tool_calling

use dotenvy::dotenv;
use llm_connector::{
    LlmClient,
    types::{ChatRequest, Message, Tool, ToolChoice},
};
#[allow(unused_imports)]
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    println!("🛠️ Tool Calling (Function Calling) Example\n");

    // We'll use OpenAI for this example as it has robust tool support
    let api_key = env::var("OPENAI_API_KEY").unwrap_or_else(|_| {
        println!("❌ Please set OPENAI_API_KEY in .env");
        std::process::exit(1);
    });
    let base_url =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let client = LlmClient::openai(&api_key, &base_url)?;

    // 1. Define a tool (function)
    // In this case, a function to get weather information
    let weather_tool = Tool::function(
        "get_current_weather",
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

    println!("--- 1. Sending request with tools ---");
    let request = ChatRequest::new("gpt-4o-mini")
        .add_message(Message::user("What's the weather like in Tokyo?"))
        .with_tools(vec![weather_tool])
        .with_tool_choice(ToolChoice::Mode("auto".to_string()));

    let response = client.chat(&request).await?;

    // 2. Check if the assistant wants to call a tool
    if let Some(choice) = response.choices.first() {
        if let Some(tool_calls) = &choice.message.tool_calls {
            println!("🤖 Assistant wants to call {} tool(s):", tool_calls.len());

            for tool_call in tool_calls {
                println!("   - Function: {}", tool_call.function.name);
                println!("   - Arguments: {}", tool_call.function.arguments);

                // 3. Simulate calling the tool and returning the result
                println!("   - [Simulating tool execution...]");
                let tool_result = json!({
                    "location": "Tokyo",
                    "temperature": "22",
                    "unit": "celsius",
                    "description": "Partly cloudy"
                });

                // 4. Send the tool result back to the assistant for a final response
                println!("\n--- 2. Sending tool result back ---");
                let second_request = ChatRequest::new("gpt-4o-mini")
                    .add_message(Message::user("What's the weather like in Tokyo?"))
                    .add_message(choice.message.clone()) // Assistant's tool call message
                    .add_message(Message::tool(tool_result.to_string(), tool_call.id.clone())); // Our tool response

                let final_response = client.chat(&second_request).await?;
                println!("Final AI Response: {}\n", final_response.content);
            }
        } else {
            println!("AI Response (No tool call): {}\n", response.content);
        }
    }

    Ok(())
}
