---
layout: home

hero:
  name: "llm-connector"
  text: "Rust LLM Connector"
  tagline: "Provider abstraction for OpenAI/Anthropic/Gemini and more — streaming, tools, multimodal, and proxy-ready."
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: View on GitHub
      link: https://github.com/lipish/llm-connector
    - theme: alt
      text: Crates.io
      link: https://crates.io/crates/llm-connector

features:
  - title: Responses API Ready
    details: Native `/responses` + streaming support with automatic fallback to `/chat/completions`.
  - title: Unified Streaming
    details: One streaming model across providers with a consistent response type.
  - title: Tools / Function Calling
    details: OpenAI-compatible tools with streaming support across all major providers.
  - title: Multi-modal
    details: Text + images in a single message block, automatically adapted per provider.
  - title: Provider Matrix
    details: 12+ providers with clean Protocol/Provider separation and per-request overrides.
  - title: Proxy / Middleware Ready
    details: All protocol request types support both Serialize and Deserialize for intercepting wire-format traffic.
  - title: Mock Client
    details: Built-in mock provider for testing — no real API calls needed.
---

# Install

```bash
cargo add llm-connector
```

Or in `Cargo.toml`:

```toml
[dependencies]
llm-connector = "1.2.1"
tokio = { version = "1", features = ["full"] }
```

OpenAI-compatible tool-calling now handles assistant messages with `tool_calls` and no text content more safely: `content` is no longer serialized as `[]` by default.

# Quick Example

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
