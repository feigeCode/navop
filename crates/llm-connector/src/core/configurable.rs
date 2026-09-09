//! Configurable Protocol Adapter - Configuration-driven abstraction
//!
//! This module provides a generic protocol adapter that customizes behavior through configuration,
//! Avoids writing repetitive boilerplate code for each Provider.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::types::{ChatRequest, ChatResponse};
use async_trait::async_trait;
use std::sync::Arc;

#[cfg(feature = "streaming")]
use crate::types::ChatStream;

/// Configurable Protocol Adapter
///
/// Wraps a base protocol and modifies its behavior through configuration (endpoint paths, authentication methods, etc.).
///
/// # Example
/// ```rust,no_run
/// use llm_connector::core::{ConfigurableProtocol, ProtocolConfig, EndpointConfig, AuthConfig};
/// use llm_connector::protocols::OpenAIProtocol;
///
/// let config = ProtocolConfig {
///     name: "custom".to_string(),
///     endpoints: EndpointConfig {
///         chat_template: "{base_url}/v1/chat/completions".to_string(),
///         models_template: Some("{base_url}/v1/models".to_string()),
///         embed_template: None,
///     },
///     auth: AuthConfig::Bearer,
///     extra_headers: vec![],
/// };
///
/// let protocol = ConfigurableProtocol::new(
///     OpenAIProtocol::new("sk-..."),
///     config
/// );
/// ```
#[derive(Clone)]
pub struct ConfigurableProtocol<P: Protocol> {
    inner: P,
    config: ProtocolConfig,
}

/// Protocol Configuration
///
/// Defines static configuration for the protocol, including name, endpoints, authentication methods, etc.
#[derive(Clone, Debug)]
pub struct ProtocolConfig {
    /// Protocol name
    pub name: String,

    /// Endpoint Configuration
    pub endpoints: EndpointConfig,

    /// Authentication Configuration
    pub auth: AuthConfig,

    /// Additional static headers
    pub extra_headers: Vec<(String, String)>,
}

/// Endpoint Configuration
///
/// Defines API endpoint path templates, supporting `{base_url}` variable substitution.
#[derive(Clone, Debug)]
pub struct EndpointConfig {
    /// Chat endpoint template
    ///
    /// Supports variables: `{base_url}`, `{model}`
    ///
    /// Example: `"{base_url}/v1/chat/completions"` or `"{base_url}/models/{model}:generateContent"`
    pub chat_template: String,

    /// Model list endpoint template (optional)
    pub models_template: Option<String>,

    /// Embed endpoint template (optional)
    pub embed_template: Option<String>,
}

/// Authentication Configuration
///
/// Defines how to handle API authentication.
pub type AuthHeaderGenerator = dyn Fn(&str) -> Vec<(String, String)> + Send + Sync;

#[derive(Clone)]
pub enum AuthConfig {
    /// Bearer token authentication
    ///
    /// Generates: `Authorization: Bearer {token}`
    Bearer,

    /// API Key header authentication
    ///
    /// Generates: `{header_name}: {token}`
    ApiKeyHeader {
        /// Header name
        header_name: String,
    },

    /// No authentication
    None,

    /// Custom authentication (through closure)
    ///
    /// Closure receives token, returns headers list
    Custom(Arc<AuthHeaderGenerator>),
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthConfig::Bearer => write!(f, "Bearer"),
            AuthConfig::ApiKeyHeader { header_name } => {
                write!(f, "ApiKeyHeader({})", header_name)
            }
            AuthConfig::None => write!(f, "None"),
            AuthConfig::Custom(_) => write!(f, "Custom(...)"),
        }
    }
}

impl<P: Protocol> ConfigurableProtocol<P> {
    /// Create new configurable protocol adapter
    ///
    /// # Parameters
    /// - `inner`: Base protocol instance
    /// - `config`: Protocol Configuration
    pub fn new(inner: P, config: ProtocolConfig) -> Self {
        Self { inner, config }
    }

    /// Convenience constructor - OpenAI compatible protocol
    ///
    /// Create a configuration using standard OpenAI endpoints and Bearer authentication.
    ///
    /// # Parameters
    /// - `inner`: Base protocol instance
    /// - `name`: Protocol name
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::core::ConfigurableProtocol;
    /// use llm_connector::protocols::OpenAIProtocol;
    ///
    /// let protocol = ConfigurableProtocol::openai_compatible(
    ///     OpenAIProtocol::new("sk-..."),
    ///     "custom-openai"
    /// );
    /// ```
    pub fn openai_compatible(inner: P, name: &str) -> Self {
        Self::new(
            inner,
            ProtocolConfig {
                name: name.to_string(),
                endpoints: EndpointConfig {
                    chat_template: "{base_url}/chat/completions".to_string(),
                    models_template: Some("{base_url}/models".to_string()),
                    embed_template: Some("{base_url}/embeddings".to_string()),
                },
                auth: AuthConfig::Bearer,
                extra_headers: vec![],
            },
        )
    }

    /// Extract token from internal protocol
    ///
    /// This is a helper method to extract token from the internal protocol's authentication headers.
    fn extract_token_from_inner(&self) -> String {
        let headers = self.inner.auth_headers();
        for (key, value) in headers {
            if key.to_lowercase() == "authorization" {
                // Extract "Bearer xxx" or "xxx"
                if let Some(token) = value.strip_prefix("Bearer ") {
                    return token.to_string();
                }
                return value;
            } else if key.to_lowercase() == "x-api-key" {
                return value;
            }
        }
        // If not found, returns empty string
        String::new()
    }
}

#[async_trait]
impl<P: Protocol> Protocol for ConfigurableProtocol<P> {
    type Request = P::Request;
    type Response = P::Response;

    fn name(&self) -> &str {
        &self.config.name
    }

    fn chat_endpoint(&self, base_url: &str, model: &str) -> String {
        self.config
            .endpoints
            .chat_template
            .replace("{base_url}", base_url.trim_end_matches('/'))
            .replace("{model}", model)
    }

    fn models_endpoint(&self, base_url: &str) -> Option<String> {
        self.config
            .endpoints
            .models_template
            .as_ref()
            .map(|template| template.replace("{base_url}", base_url.trim_end_matches('/')))
    }

    fn embed_endpoint(&self, base_url: &str, model: &str) -> Option<String> {
        self.config
            .endpoints
            .embed_template
            .as_ref()
            .map(|template| {
                template
                    .replace("{base_url}", base_url.trim_end_matches('/'))
                    .replace("{model}", model)
            })
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        self.inner.build_request(request)
    }

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        self.inner.parse_response(response)
    }

    fn parse_models(&self, response: &str) -> Result<Vec<String>, LlmConnectorError> {
        self.inner.parse_models(response)
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        self.inner.map_error(status, body)
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        let mut headers = match &self.config.auth {
            AuthConfig::Bearer => {
                // Extract token from inner protocol and convert to Bearer format
                let token = self.extract_token_from_inner();
                if token.is_empty() {
                    vec![]
                } else {
                    vec![("Authorization".to_string(), format!("Bearer {}", token))]
                }
            }
            AuthConfig::ApiKeyHeader { header_name } => {
                // from inner protocol Get token，Usecustom header name
                let token = self.extract_token_from_inner();
                if token.is_empty() {
                    vec![]
                } else {
                    vec![(header_name.clone(), token)]
                }
            }
            AuthConfig::None => vec![],
            AuthConfig::Custom(f) => {
                let token = self.extract_token_from_inner();
                f(&token)
            }
        };

        // Add additional static headers
        headers.extend(self.config.extra_headers.clone());
        headers
    }

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ChatStream, LlmConnectorError> {
        // Automatically detect stream format based on content-type or body inspection
        // This makes ConfigurableProtocol robust for all OpenAI-compatible providers
        // even if they use NDJSON (like Ollama) or non-standard SSE.
        Ok(crate::sse::sse_to_streaming_response(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::OpenAIProtocol;

    #[test]
    fn test_configurable_protocol_basic() {
        let config = ProtocolConfig {
            name: "test".to_string(),
            endpoints: EndpointConfig {
                chat_template: "{base_url}/v1/chat/completions".to_string(),
                models_template: Some("{base_url}/v1/models".to_string()),
                embed_template: Some("{base_url}/v1/embeddings".to_string()),
            },
            auth: AuthConfig::Bearer,
            extra_headers: vec![],
        };

        let protocol = ConfigurableProtocol::new(OpenAIProtocol::new("sk-test"), config);

        assert_eq!(protocol.name(), "test");
        assert_eq!(
            protocol.chat_endpoint("https://api.example.com", "gpt-4"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            protocol.models_endpoint("https://api.example.com"),
            Some("https://api.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn test_openai_compatible() {
        let protocol =
            ConfigurableProtocol::openai_compatible(OpenAIProtocol::new("sk-test"), "custom");

        assert_eq!(protocol.name(), "custom");
        assert_eq!(
            protocol.chat_endpoint("https://api.example.com", "any"),
            "https://api.example.com/chat/completions"
        );
    }

    #[test]
    fn test_custom_endpoint() {
        let config = ProtocolConfig {
            name: "volcengine".to_string(),
            endpoints: EndpointConfig {
                chat_template: "{base_url}/api/v3/chat/completions".to_string(),
                models_template: Some("{base_url}/api/v3/models".to_string()),
                embed_template: Some("{base_url}/api/v3/embeddings".to_string()),
            },
            auth: AuthConfig::Bearer,
            extra_headers: vec![],
        };

        let protocol = ConfigurableProtocol::new(OpenAIProtocol::new("sk-test"), config);

        assert_eq!(
            protocol.chat_endpoint("https://api.example.com", "any"),
            "https://api.example.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn test_extra_headers() {
        let config = ProtocolConfig {
            name: "test".to_string(),
            endpoints: EndpointConfig {
                chat_template: "{base_url}/v1/chat/completions".to_string(),
                models_template: None,
                embed_template: None,
            },
            auth: AuthConfig::Bearer,
            extra_headers: vec![
                ("X-Custom-Header".to_string(), "value".to_string()),
                ("X-Another-Header".to_string(), "value2".to_string()),
            ],
        };

        let protocol = ConfigurableProtocol::new(OpenAIProtocol::new("sk-test"), config);
        let headers = protocol.auth_headers();

        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "X-Custom-Header" && v == "value")
        );
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "X-Another-Header" && v == "value2")
        );
    }
}
