# Streaming

`llm-connector` provides unified streaming support across all providers.

## Basic Streaming

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message}};
use futures_util::StreamExt;

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let request = ChatRequest::new("gpt-4o")
    .add_message(Message::user("Tell me a story"))
    .with_stream(true);

let mut stream = client.chat_stream(&request).await?;

while let Some(chunk) = stream.next().await {
    if let Some(content) = chunk?.get_content() {
        print!("{}", content);
    }
}
```

## Responses API Streaming

```rust
use llm_connector::{LlmClient, types::ResponsesRequest};
use futures_util::StreamExt;

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let request = ResponsesRequest {
    model: "gpt-4.1".to_string(),
    input: Some(serde_json::json!("Count from 1 to 5")),
    stream: Some(true),
    ..Default::default()
};

let mut stream = client.invoke_responses_stream(&request).await?;
while let Some(event) = stream.next().await {
    let event = event?;
    if event.event_type == "response.output_text.delta"
        && let Some(delta) = event.data.get("delta").and_then(|v| v.as_str())
    {
        print!("{}", delta);
    }
}
```

If `/responses` is unsupported by upstream provider, connector automatically falls back to `/chat/completions` and emits a minimal compatible event sequence:

- `response.created`
- `response.output_text.delta` (repeated)
- `response.completed`

## Streaming with Reasoning Models

DeepSeek R1, Moonshot Thinking, Google Gemini Thinking, etc. expose separate `reasoning_content`:

```rust
let mut stream = client.chat_stream(&request).await?;

while let Some(chunk) = stream.next().await {
    let chunk = chunk?;

    if let Some(reasoning) = &chunk.reasoning_content {
        print!("<think>{}</think>", reasoning);
    }

    if let Some(content) = chunk.get_content() {
        print!("{}", content);
    }
}
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
                print!("Tool: {}", call.function.name);
            }
        }
    }
}
```

## Examples

```bash
cargo run --example anthropic          # Anthropic streaming
cargo run --example google             # Google Gemini streaming
cargo run --example moonshot_thinking  # Moonshot thinking model
cargo run --example zhipu_thinking     # Zhipu reasoning streaming
cargo run --example tencent            # Tencent Hunyuan streaming
cargo run --example ollama             # Ollama streaming
```
