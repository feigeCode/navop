# Getting Started

## Requirements

- Rust 1.85+ (Rust 2024 edition)

## Install

```toml
[dependencies]
llm-connector = "1.2.1"
tokio = { version = "1", features = ["full"] }
```

If you use multi-turn tool calling with OpenAI-compatible providers, `assistant_with_tool_calls()` messages are serialized in a gateway-friendly way by default instead of emitting empty `content: []`.

## First Request

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message}};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

    let request = ChatRequest::new("gpt-4o")
        .add_message(Message::user("Hello!"));

    let response = client.chat(&request).await?;
    println!("{}", response.content);

    Ok(())
}
```

## Builder Pattern

For more control (timeout, proxy, custom headers):

```rust
use llm_connector::LlmClient;

let client = LlmClient::builder()
    .openai("sk-...")
    .base_url("https://api.openai.com/v1")
    .timeout(60)
    .build()?;
```

## Next

- [Providers](/guide/providers)
- [Streaming](/guide/streaming)
- [Tools](/guide/tools)
- [Multi-modal](/guide/multimodal)
