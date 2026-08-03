# Tools / Function Calling

`llm-connector` supports OpenAI-compatible function calling (tools) across all major providers.

## Basic Usage

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message, Tool, ToolChoice}};

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let tools = vec![Tool::function(
    "get_weather",
    Some("Get weather information for a city".to_string()),
    serde_json::json!({
        "type": "object",
        "properties": {
            "location": {
                "type": "string",
                "description": "City name, e.g. Beijing, Shanghai"
            }
        },
        "required": ["location"]
    }),
)];

let request = ChatRequest::new("gpt-4o")
    .add_message(Message::user("What's the weather in Beijing?"))
    .with_tools(tools)
    .with_tool_choice(ToolChoice::auto());

let response = client.chat(&request).await?;

if response.is_tool_call() {
    for tc in response.tool_calls() {
        let args: serde_json::Value = tc.parse_arguments()?;
        println!("{} => {:?}", tc.function.name, args);
    }
}
```

## Multi-Turn Tool Calls

When you replay the assistant tool-call turn with `Message::assistant_with_tool_calls(...)`, OpenAI-compatible request serialization keeps `tool_calls` intact and avoids sending empty `content` as `[]` by default. Standard OpenAI-compatible providers emit `null`; text-only OpenAI-compatible providers emit `""`.

```rust
use llm_connector::types::{Message, ToolCall, FunctionCall};

// Step 1: Send initial request, get tool call response
let response = client.chat(&request).await?;

// Step 2: Execute the tool, then send the result back
let tool_call = &response.tool_calls()[0];
let tool_result = "sunny, 22°C"; // from your actual tool

let follow_up = ChatRequest::new("gpt-4o")
    .add_message(Message::user("What's the weather in Beijing?"))
    .add_message(Message::assistant_with_tool_calls(response.tool_calls().to_vec()))
    .add_message(Message::tool(tool_result, &tool_call.id));

let final_response = client.chat(&follow_up).await?;
println!("{}", final_response.content);
```

## Streaming Tool Calls

```rust
use futures_util::StreamExt;

let mut stream = client.chat_stream(&request).await?;

while let Some(chunk) = stream.next().await {
    let chunk = chunk?;
    if let Some(choice) = chunk.choices.first() {
        if let Some(tool_calls) = &choice.delta.tool_calls {
            for call in tool_calls {
                println!("Tool: {}", call.function.name);
            }
        }
    }
}
```

## Examples

```bash
cargo run --example tool_calling       # OpenAI tool calling
cargo run --example moonshot_tools     # Moonshot tool calling
cargo run --example zhipu_tools        # Zhipu tool calling
cargo run --example google_tools_thinking  # Google tools + thinking
```
