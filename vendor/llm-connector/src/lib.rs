//! # llm-connector
//!
//! Next-generation Rust library for LLM protocol abstraction.
//!
//! Supports 5 protocols: OpenAI, Anthropic, Aliyun, Zhipu, Ollama.
//! Clean architecture with clear Protocol/Provider separation.
//!
//! ## Quick Start
//!
//! ### OpenAI Protocol
//! ```rust,no_run
//! use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // OpenAI
//!     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
//!
//!     let request = ChatRequest {
//!         model: "gpt-4".to_string(),
//!         messages: vec![Message::text(Role::User, "Hello!")],
//!         ..Default::default()
//!     };
//!
//!     let response = client.chat(&request).await?;
//!     println!("Response: {}", response.content);
//!     Ok(())
//! }
//! ```
//!
//! ### Anthropic Protocol
//! ```rust,no_run
//! use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LlmClient::anthropic("sk-ant-...", "https://api.anthropic.com")?;
//!     let request = ChatRequest {
//!         model: "claude-3-5-sonnet-20241022".to_string(),
//!         messages: vec![Message::text(Role::User, "Hello!")],
//!         ..Default::default()
//!     };
//!
//!     let response = client.chat(&request).await?;
//!     println!("Response: {}", response.content);
//!     Ok(())
//! }
//! ```
//!
//! ### Aliyun Protocol (DashScope)
//! ```rust,no_run
//! use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LlmClient::aliyun("sk-...", "https://dashscope.aliyuncs.com")?;
//!     let request = ChatRequest {
//!         model: "qwen-turbo".to_string(),
//!         messages: vec![Message::text(Role::User, "Hello!")],
//!         ..Default::default()
//!     };
//!
//!     let response = client.chat(&request).await?;
//!     println!("Response: {}", response.content);
//!     Ok(())
//! }
//! ```
//!
//! ### Ollama Protocol (Local)
//! ```rust,no_run
//! use llm_connector::{LlmClient, Provider, types::{ChatRequest, Message, Role}};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Default: localhost:11434
//!     // Default: localhost:11434
//!     let client = LlmClient::ollama("http://localhost:11434")?;
//!
//!     // Custom URL
//!     let client = LlmClient::ollama("http://192.168.1.100:11434")?;
//!
//!     let request = ChatRequest {
//!         model: "llama3.2".to_string(),
//!         messages: vec![Message::text(Role::User, "Hello!")],
//!         ..Default::default()
//!     };
//!
//!     let response = client.chat(&request).await?;
//!     println!("Response: {}", response.content);
//!
//!     // Ollama special features
//!     if let Some(ollama) = client.as_ollama() {
//!         let models = ollama.models().await?;
//!         println!("Available models: {:?}", models);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Installation
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! llm-connector = "0.2"
//! tokio = { version = "1", features = ["full"] }
//! ```
//!
//! Optional features:
//! ```toml
//! llm-connector = { version = "0.2", features = ["streaming"] }
//! ```

// Core modules (V2 Architecture - Default)
pub mod builder;
pub mod client;
pub mod config;
pub mod core;
pub mod error;
pub mod protocols;
pub mod providers;
pub mod types;

// Server-Sent Events (SSE) utilities
pub mod sse;

// Re-exports for convenience (V2 Architecture)
pub use client::LlmClient;
pub use config::ProviderConfig;
pub use error::LlmConnectorError;
pub use types::{
    AnthropicToolChoice, AnthropicToolDefinition, ChatRequest, ChatResponse, Choice,
    FunctionCall, JsonSchemaSpec, Message, ResponseFormat, ResponsesRequest, ResponsesResponse,
    ResponsesStreamEvent, ResponsesUsage, Role, Tool, ToolCall, ToolChoice, Usage,
};

// Re-export core traits
pub use core::{GenericProvider, HttpClient, Protocol, Provider};

// Re-export protocols
pub use protocols::{
    AliyunProtocol, AnthropicProtocol, GoogleProtocol, OllamaProtocol, OpenAIProtocol,
    ZhipuProtocol,
};

// Re-export providers
pub use providers::{
    AliyunProvider,
    AnthropicProvider,
    OllamaProvider,
    OpenAIProvider,
    ZhipuProvider,
    aliyun,
    anthropic,
    ollama,
    // Convenience functions
    openai,
    xinference,
    zhipu,
};

#[cfg(feature = "streaming")]
pub use types::{
    AnthropicSseAdapter, ChatStream, Delta, OllamaChatStream, OllamaMessage, OllamaStreamChunk,
    ResponsesStream, StreamChunk, StreamFormat, StreamingChoice, StreamingConfig, StreamingFormat,
    StreamingResponse, UniversalChatStream,
};
