<div align="center">

<h1>llm-connector</h1>

[![crates.io](https://img.shields.io/crates/v/llm-connector.svg)](https://crates.io/crates/llm-connector)
[![build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/lipish/llm-connector)
[![downloads](https://img.shields.io/crates/d/llm-connector.svg)](https://crates.io/crates/llm-connector)
[![msrv](https://img.shields.io/badge/MSRV-1.85%2B-blue.svg)](https://www.rust-lang.org)
[![docs.rs](https://img.shields.io/docsrs/llm-connector)](https://docs.rs/llm-connector)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**A simple protocol adapter layer for LLM services.**

[Installation](#installation) • [Usage](#usage)

</div>

---

## Overview

`llm-connector` is a lightweight protocol adapter for connecting various LLM services. It handles protocol conversion and standardization without caring about specific API endpoints.

- **Zero Hardcoded URLs**: No default endpoints. You provide the `base_url`
- **Protocol Agnostic**: Support multiple LLM providers with unified interface
- **Lightweight & Standalone**: No external configuration management or database dependencies

## Protocol Architecture

The `src/protocols/` module uses adapter pattern to convert different vendor APIs into unified internal interface:

- **`formats/`**: Protocol-agnostic data structures
- **`adapters/`**: Vendor-specific protocol adapters
- **`common/`**: Shared utility functions

## Installation

**Minimum Rust Version**: 1.85+

```toml
[dependencies]
llm-connector = "1.2.1"
tokio = { version = "1", features = ["full"] }
```

OpenAI-compatible tool-calling requests now preserve empty assistant content safely: assistant messages with `tool_calls` and no text no longer serialize `content` as `[]` by default.

## Usage

### Basic Chat

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let request = ChatRequest::new("gpt-4o")
    .add_message(Message::user("What is the speed of light?"));

let response = client.chat(&request).await?;
println!("{}", response.content);
```

### Streaming Response

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};
use futures_util::StreamExt;

let client = LlmClient::anthropic("sk-ant-...", "https://api.anthropic.com")?;
let request = ChatRequest::new("claude-3-5-sonnet-20240620").with_stream(true);

let mut stream = client.chat_stream(&request).await?;
while let Some(chunk) = stream.next().await {
    if let Some(content) = chunk?.get_content() {
        print!("{}", content);
    }
}
```

### OpenAI Responses API

```rust
use llm_connector::{LlmClient, types::ResponsesRequest};

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let request = ResponsesRequest {
    model: "gpt-4.1".to_string(),
    input: Some(serde_json::json!("Write one sentence about Rust.")),
    ..Default::default()
};

let response = client.invoke_responses(&request).await?;
println!("{}", response.output_text);
```

If provider does not support `/responses`, connector automatically falls back to `/chat/completions`.

### OpenAI Responses Streaming

```rust
use llm_connector::{LlmClient, types::ResponsesRequest};
use futures_util::StreamExt;

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
let request = ResponsesRequest {
    model: "gpt-4.1".to_string(),
    input: Some(serde_json::json!("Count 1 to 5")),
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

### Builder Pattern

```rust
let client = LlmClient::builder()
    .openai("sk-...")
    .base_url("https://api.deepseek.com") // Required
    .timeout(60)
    .build()?;
```

## Advanced Features

### Reasoning Models

Support for reasoning models like OpenAI o1/o3 and Claude 3.7 Sonnet.

```rust
use llm_connector::types::ReasoningEffort;

let request = ChatRequest::new("claude-3-7-sonnet-20250219")
    .add_message(Message::user("Solve this logic puzzle..."))
    .with_thinking_budget(16000) // Enable thinking with 16k token budget
    .with_max_tokens(20000);     // Ensure max_tokens > thinking_budget

let response = client.chat(&request).await?;
```

### File Upload

Support for uploading local files (images, PDFs, etc.) with automatic Base64 encoding and MIME type detection.

```rust
use llm_connector::types::MessageBlock;

let request = ChatRequest::new("claude-3-5-sonnet")
    .add_message(Message::user("Analyze this document"))
    .add_message_block(MessageBlock::from_file_path("report.pdf")?);

let response = client.chat(&request).await?;
```

## License

MIT
