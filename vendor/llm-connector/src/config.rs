//! Configuration management for LLM providers
//!
//! This module provides simple configuration for LLM providers.
//!
//! # Direct API Key Configuration
//!
//! The simplest way to configure a provider:
//!
//! ```rust
//! use llm_connector::config::ProviderConfig;
//!
//! let config = ProviderConfig::new("your-api-key");
//! ```
//!
//! # Advanced Configuration
//!
//! For custom settings:
//!
//! ```rust
//! use llm_connector::config::{ProviderConfig, RetryConfig};
//!
//! let config = ProviderConfig::new("your-api-key")
//!     .with_base_url("https://api.example.com/v1")
//!     .with_timeout_ms(30000)
//!     .with_retry(RetryConfig {
//!         max_retries: 3,
//!         initial_backoff_ms: 1000,
//!         backoff_multiplier: 2.0,
//!         max_backoff_ms: 30000,
//!     })
//!     .with_header("X-Custom-Header", "value");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for retry behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,

    /// Initial backoff delay in milliseconds
    pub initial_backoff_ms: u64,

    /// Backoff multiplier for exponential backoff
    pub backoff_multiplier: f32,

    /// Maximum backoff delay in milliseconds
    pub max_backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            backoff_multiplier: 2.0,
            max_backoff_ms: 30000,
        }
    }
}

/// Configuration for a single provider
///
/// This is the unified configuration structure used across all protocols.
/// It contains all the necessary information to create and configure a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// API key for authentication
    pub api_key: String,

    /// Optional base URL override
    /// If not provided, the protocol's default URL will be used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Request timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Optional HTTP proxy URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,

    /// Retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,

    /// Custom HTTP headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Maximum concurrent requests (for connection pooling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_requests: Option<usize>,
}

impl ProviderConfig {
    /// Create a new provider configuration with just an API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
            timeout_ms: None,
            proxy: None,
            retry: None,
            headers: None,
            max_concurrent_requests: None,
        }
    }

    /// Set the base URL
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Set the timeout in milliseconds
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Set the proxy URL
    pub fn with_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Set the retry configuration
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Add a custom header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Set custom headers
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set maximum concurrent requests
    pub fn with_max_concurrent_requests(mut self, max: usize) -> Self {
        self.max_concurrent_requests = Some(max);
        self
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_ms.unwrap_or(120000))
    }

    /// Get the retry configuration, or default if not set
    pub fn retry_config(&self) -> RetryConfig {
        self.retry.clone().unwrap_or_default()
    }
}

/// Shared provider configuration
///
/// This is an Arc-wrapped version of ProviderConfig for efficient sharing
/// across multiple components without cloning.
#[derive(Debug, Clone)]
pub struct SharedProviderConfig {
    inner: Arc<ProviderConfig>,
}

impl SharedProviderConfig {
    /// Create a new shared configuration
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            inner: Arc::new(config),
        }
    }

    /// Get a reference to the inner configuration
    pub fn get(&self) -> &ProviderConfig {
        &self.inner
    }
}

impl From<ProviderConfig> for SharedProviderConfig {
    fn from(config: ProviderConfig) -> Self {
        Self::new(config)
    }
}

impl std::ops::Deref for SharedProviderConfig {
    type Target = ProviderConfig;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_builder() {
        let config = ProviderConfig::new("test-key")
            .with_base_url("https://api.example.com")
            .with_timeout_ms(5000)
            .with_header("X-Custom", "value")
            .with_retry(RetryConfig::default());

        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.base_url, Some("https://api.example.com".to_string()));
        assert_eq!(config.timeout_ms, Some(5000));
        assert!(config.headers.is_some());
        assert!(config.retry.is_some());
    }

    #[test]
    fn test_retry_config_default() {
        let retry = RetryConfig::default();
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.initial_backoff_ms, 1000);
        assert_eq!(retry.backoff_multiplier, 2.0);
        assert_eq!(retry.max_backoff_ms, 30000);
    }

    #[test]
    fn test_shared_config() {
        let config = ProviderConfig::new("test-key");
        let shared1 = SharedProviderConfig::new(config.clone());
        let shared2 = shared1.clone();

        assert_eq!(shared1.api_key, shared2.api_key);
        assert_eq!(Arc::strong_count(&shared1.inner), 2);
    }
}
