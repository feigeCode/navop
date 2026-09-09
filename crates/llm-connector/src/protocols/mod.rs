//! Protocol Module - Public Standard Protocols
//!
//! Reorganized into V2 Architecture:
//! - common/: Shared components (Auth, Streamers, Requests)
//! - formats/: Protocol format definitions (e.g., chat completions)
//! - adapters/: Implementation for each specific provider

pub mod adapters;
pub mod common;
pub mod formats;

// Re-export standard protocol types from adapters
pub use adapters::{
    AliyunProtocol, AnthropicProtocol, GoogleProtocol, OllamaProtocol, OpenAIProtocol,
    ZhipuProtocol,
};

#[cfg(feature = "tencent")]
pub use adapters::TencentNativeProtocol;
