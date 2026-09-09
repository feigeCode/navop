//! V2 Architecture Core Module
//!
//! This module contains all core components of V2 architecture：
//! - Unified trait definitions (Protocol, Provider)
//! - HTTP client implementation
//! - Generic provider implementation

pub mod builder;
pub mod client;
pub mod configurable;
pub mod resolver;
pub mod traits;

// Re-export core types
pub use builder::ProviderBuilder;
pub use client::HttpClient;
pub use configurable::{AuthConfig, ConfigurableProtocol, EndpointConfig, ProtocolConfig};
pub use resolver::{EnvVarResolver, ServiceResolver, ServiceTarget, StaticResolver};
pub use traits::{GenericProvider, Protocol, Provider};
