//! Service Resolution - Dynamic Configuration
//!
//! This module provides traits and implementations for dynamic service resolution,
//! allowing runtime determination of endpoints, authentication, and model mapping.

use crate::error::LlmConnectorError;
use async_trait::async_trait;
use std::collections::HashMap;

/// Resolved service target configuration
#[derive(Debug, Clone)]
pub struct ServiceTarget {
    /// API Endpoint (Base URL)
    pub endpoint: Option<String>,
    /// API Key
    pub api_key: Option<String>,
    /// Target Model Name (may be aliased)
    pub model: String,
    /// Additional headers
    pub extra_headers: Option<HashMap<String, String>>,
}

impl ServiceTarget {
    /// Create new service target with just model name
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            endpoint: None,
            api_key: None,
            model: model.into(),
            extra_headers: None,
        }
    }
}

/// Service Resolver Trait
///
/// Resolves the actual service configuration based on the requested model name.
#[async_trait]
pub trait ServiceResolver: Send + Sync {
    /// Resolve service configuration for the given model
    async fn resolve(&self, model: &str) -> Result<ServiceTarget, LlmConnectorError>;
}

/// Environment Variable Resolver
///
/// Resolves API keys from environment variables based on model prefix.
///
/// # Example
///
/// - `gpt-` -> `OPENAI_API_KEY`
/// - `claude-` -> `ANTHROPIC_API_KEY`
pub struct EnvVarResolver {
    mappings: HashMap<String, String>,
}

impl Default for EnvVarResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvVarResolver {
    pub fn new() -> Self {
        let mut mappings = HashMap::new();
        mappings.insert("gpt".to_string(), "OPENAI_API_KEY".to_string());
        mappings.insert("claude".to_string(), "ANTHROPIC_API_KEY".to_string());
        mappings.insert("gemini".to_string(), "GOOGLE_API_KEY".to_string());
        mappings.insert("deepseek".to_string(), "DEEPSEEK_API_KEY".to_string());
        Self { mappings }
    }

    pub fn with_mapping(mut self, prefix: &str, env_var: &str) -> Self {
        self.mappings
            .insert(prefix.to_string(), env_var.to_string());
        self
    }
}

#[async_trait]
impl ServiceResolver for EnvVarResolver {
    async fn resolve(&self, model: &str) -> Result<ServiceTarget, LlmConnectorError> {
        let mut target = ServiceTarget::new(model);

        for (prefix, env_var) in &self.mappings {
            if model.starts_with(prefix) {
                if let Ok(key) = std::env::var(env_var) {
                    target.api_key = Some(key);
                }
                break;
            }
        }

        Ok(target)
    }
}

/// Static Resolver
///
/// Uses a static configuration to resolve services.
/// Useful for testing or simple configurations.
pub struct StaticResolver {
    target: ServiceTarget,
}

impl StaticResolver {
    pub fn new(target: ServiceTarget) -> Self {
        Self { target }
    }
}

#[async_trait]
impl ServiceResolver for StaticResolver {
    async fn resolve(&self, _model: &str) -> Result<ServiceTarget, LlmConnectorError> {
        Ok(self.target.clone())
    }
}
