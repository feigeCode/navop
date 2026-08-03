//! Unified Trait Definitions - V2 Architecture Core
//!
//! This module defines core traits for V2 architecture, providing clear and unified abstraction layer。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;

// Reuse existing types, maintain compatibility
use crate::error::LlmConnectorError;
use crate::types::{
    ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, ResponsesRequest, ResponsesResponse,
    ResponsesStreamEvent, ResponsesUsage, chat_response_to_responses_response,
    responses_request_to_chat_request,
};

#[cfg(feature = "streaming")]
use crate::types::{ChatStream, ResponsesStream};

#[cfg(feature = "streaming")]
use futures_util::StreamExt;

/// Protocol trait - Defines pure API specification
///
/// This trait represents an LLM API protocol specification, such as OpenAI API, Anthropic API, etc.
/// It only focuses on API format conversion, not specific network communication.
#[async_trait]
pub trait Protocol: Send + Sync + Clone + 'static {
    /// Protocol-specific request type
    type Request: Serialize + Send + Sync;

    /// Protocol-specific response type  
    type Response: for<'de> Deserialize<'de> + Send + Sync;

    /// Protocol name (such as "openai", "anthropic")
    fn name(&self) -> &str;

    /// Protocol capability metadata.
    fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        crate::protocols::common::capabilities::ProviderCapabilities::default()
    }

    /// Get chat completion endpoint URL
    fn chat_endpoint(&self, base_url: &str, model: &str) -> String;

    /// Resolve the chat endpoint as a transport stage hook.
    ///
    /// This keeps the old endpoint API intact while allowing GenericProvider
    /// to orchestrate the chat lifecycle through explicit stage-oriented entry points.
    fn resolve_chat_endpoint(&self, base_url: &str, model: &str) -> String {
        self.chat_endpoint(base_url, model)
    }

    /// Get chat stream endpoint URL (optional)
    #[cfg(feature = "streaming")]
    fn chat_stream_endpoint(&self, base_url: &str, model: &str) -> String {
        self.chat_endpoint(base_url, model)
    }

    /// Get model list endpoint URL (optional)
    fn models_endpoint(&self, _base_url: &str) -> Option<String> {
        None
    }

    /// Get embeddings endpoint URL (optional)
    fn embed_endpoint(&self, _base_url: &str, _model: &str) -> Option<String> {
        None
    }

    /// Get responses endpoint URL (optional)
    fn responses_endpoint(&self, _base_url: &str, _model: &str) -> Option<String> {
        None
    }

    /// Build protocol-specific request
    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError>;

    /// Build the chat request body as a request-assembly stage hook.
    fn build_chat_request_body(
        &self,
        request: &ChatRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        let built = self.build_request(request)?;
        serde_json::to_value(built).map_err(|e| {
            LlmConnectorError::InvalidRequest(format!(
                "{}: failed to serialize chat request body: {}",
                self.name(),
                e
            ))
        })
    }

    /// Parse protocol-specific response
    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError>;

    /// Parse the final chat response as a response-normalization stage hook.
    fn normalize_chat_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        self.parse_response(response)
    }

    /// Parse model list response
    fn parse_models(&self, _response: &str) -> Result<Vec<String>, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support model listing",
            self.name()
        )))
    }

    /// Build protocol-specific embedding request
    fn build_embed_request(
        &self,
        _request: &EmbedRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support embeddings",
            self.name()
        )))
    }

    /// Parse protocol-specific embedding response
    fn parse_embed_response(&self, _response: &str) -> Result<EmbedResponse, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support embeddings",
            self.name()
        )))
    }

    /// Build protocol-specific responses request
    fn build_responses_request(
        &self,
        _request: &ResponsesRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support responses API",
            self.name()
        )))
    }

    /// Parse protocol-specific responses response
    fn parse_responses_response(
        &self,
        response: &str,
    ) -> Result<ResponsesResponse, LlmConnectorError> {
        let mut parsed = serde_json::from_str::<ResponsesResponse>(response).map_err(|e| {
            LlmConnectorError::ParseError(format!(
                "{}: failed to parse responses response: {}",
                self.name(),
                e
            ))
        })?;
        parsed.populate_output_text();
        Ok(parsed)
    }

    /// Map HTTP errors to unified error type
    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError;

    /// Resolve the authentication strategy for this protocol.
    fn auth_strategy(&self) -> crate::protocols::common::auth::AuthStrategy {
        crate::protocols::common::auth::AuthStrategy::None
    }

    /// Resolve fixed header policy for this protocol.
    fn header_policy(&self) -> crate::protocols::common::auth::HeaderPolicy {
        crate::protocols::common::auth::HeaderPolicy::default()
    }

    /// Resolve request metadata policy for this protocol.
    fn request_metadata_policy(&self) -> crate::protocols::common::auth::RequestMetadataPolicy {
        crate::protocols::common::auth::RequestMetadataPolicy::default()
    }

    /// Get authentication headers (optional)
    fn auth_headers(&self) -> Vec<(String, String)> {
        crate::protocols::common::auth::apply_header_policy(
            crate::protocols::common::auth::materialize_auth_headers(&self.auth_strategy()),
            &self.header_policy(),
        )
    }

    /// Build authentication headers for request overrides
    ///
    /// This allows protocols to specify which headers should be injected when an API key is provided in the request.
    /// Default implementation returns empty list to avoid duplicate header injection.
    fn build_auth_headers_for_override(&self, api_key: &str) -> Vec<(String, String)> {
        crate::protocols::common::auth::apply_header_policy(
            crate::protocols::common::auth::materialize_auth_headers(
                &self.override_auth_strategy(api_key),
            ),
            &self.header_policy(),
        )
    }

    /// Resolve override authentication strategy when request-level api_key is provided.
    fn override_auth_strategy(
        &self,
        _api_key: &str,
    ) -> crate::protocols::common::auth::AuthStrategy {
        crate::protocols::common::auth::AuthStrategy::None
    }

    /// Parse streaming response (optional)
    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ChatStream, LlmConnectorError> {
        // Default to use generic SSE stream parser
        Ok(crate::sse::sse_to_streaming_response(response))
    }

    /// Parse the streaming response as a stream-interpretation stage hook.
    #[cfg(feature = "streaming")]
    async fn interpret_chat_stream(
        &self,
        response: reqwest::Response,
    ) -> Result<ChatStream, LlmConnectorError> {
        self.parse_stream_response(response).await
    }
}

/// Service Provider trait - Define unified service interface
///
/// This trait represents a specific LLM service provider, providing complete service functionality.
/// It is the direct user interaction interface.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name (such as "openai", "aliyun", "ollama")
    fn name(&self) -> &str;

    /// Provider capability metadata.
    fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities;

    /// Chat completion
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError>;

    /// Streaming chat completion
    #[cfg(feature = "streaming")]
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, LlmConnectorError>;

    /// Get available models list
    async fn models(&self) -> Result<Vec<String>, LlmConnectorError>;

    /// Generate embeddings
    async fn embed(&self, request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError>;

    /// OpenAI Responses API (non-stream)
    async fn invoke_responses(
        &self,
        _request: &ResponsesRequest,
    ) -> Result<ResponsesResponse, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(
            "responses API is not supported by this provider".to_string(),
        ))
    }

    /// OpenAI Responses API (stream)
    #[cfg(feature = "streaming")]
    async fn invoke_responses_stream(
        &self,
        _request: &ResponsesRequest,
    ) -> Result<ResponsesStream, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(
            "responses streaming API is not supported by this provider".to_string(),
        ))
    }

    /// Type conversion support (for special feature access)
    fn as_any(&self) -> &dyn Any;
}

/// Helper to build request overrides map
fn build_request_overrides<P: Protocol>(
    protocol: &P,
    request: &ChatRequest,
) -> HashMap<String, String> {
    let mut overrides = protocol.request_metadata_policy().header_overrides;

    // 1. API Key override
    if let Some(ref key) = request.api_key {
        let auth_headers = protocol.build_auth_headers_for_override(key);
        for (k, v) in auth_headers {
            overrides.insert(k, v);
        }
    }

    // 2. Extra headers override
    if let Some(ref extra) = request.extra_headers {
        overrides.extend(extra.clone());
    }

    overrides
}

fn validate_chat_request_capabilities<P: Protocol>(
    protocol: &P,
    request: &ChatRequest,
) -> Result<(), LlmConnectorError> {
    let capabilities = protocol.capabilities();
    let supports_enable_thinking = matches!(
        capabilities.reasoning_request_strategy,
        crate::protocols::common::capabilities::ReasoningRequestStrategy::EnableThinking
            | crate::protocols::common::capabilities::ReasoningRequestStrategy::EnableThinkingWithBudget
    );
    let supports_thinking_budget = matches!(
        capabilities.reasoning_request_strategy,
        crate::protocols::common::capabilities::ReasoningRequestStrategy::ThinkingBudget
            | crate::protocols::common::capabilities::ReasoningRequestStrategy::EnableThinkingWithBudget
    );
    let supports_reasoning_effort = matches!(
        capabilities.reasoning_request_strategy,
        crate::protocols::common::capabilities::ReasoningRequestStrategy::ReasoningEffort
    );

    if !capabilities.supports_multimodal_input && request.has_non_text_content() {
        return Err(LlmConnectorError::InvalidRequest(format!(
            "{} does not support non-text message blocks for this request",
            protocol.name()
        )));
    }

    if !capabilities.supports_response_format && request.response_format.is_some() {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support response_format",
            protocol.name()
        )));
    }

    if !capabilities.supports_tool_choice && request.tool_choice.is_some() {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support tool_choice",
            protocol.name()
        )));
    }

    if request.enable_thinking.is_some() && !supports_enable_thinking {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support enable_thinking",
            protocol.name()
        )));
    }

    if request.thinking_budget.is_some() && !supports_thinking_budget {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support thinking_budget",
            protocol.name()
        )));
    }

    if request.reasoning_effort.is_some() && !supports_reasoning_effort {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support reasoning_effort",
            protocol.name()
        )));
    }

    Ok(())
}

fn validate_embed_request_capabilities<P: Protocol>(
    protocol: &P,
    _request: &EmbedRequest,
) -> Result<(), LlmConnectorError> {
    let capabilities = protocol.capabilities();

    if !capabilities.supports_embeddings {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support embeddings",
            protocol.name()
        )));
    }

    Ok(())
}

fn validate_responses_request_capabilities<P: Protocol>(
    protocol: &P,
    request: &ResponsesRequest,
) -> Result<(), LlmConnectorError> {
    let capabilities = protocol.capabilities();

    if !capabilities.supports_responses_api {
        if request.tool_choice.is_some() && !capabilities.supports_tool_choice {
            return Err(LlmConnectorError::UnsupportedOperation(format!(
                "{} fallback chat path does not support tool_choice for responses requests",
                protocol.name()
            )));
        }

        return Ok(());
    }

    if request.tool_choice.is_some() && !capabilities.supports_tool_choice {
        return Err(LlmConnectorError::UnsupportedOperation(format!(
            "{} does not support tool_choice for responses requests",
            protocol.name()
        )));
    }

    Ok(())
}

fn build_responses_request_overrides<P: Protocol>(
    protocol: &P,
    request: &ResponsesRequest,
) -> HashMap<String, String> {
    let mut overrides = protocol.request_metadata_policy().header_overrides;

    if let Some(ref key) = request.api_key {
        let auth_headers = protocol.build_auth_headers_for_override(key);
        for (k, v) in auth_headers {
            overrides.insert(k, v);
        }
    }

    if let Some(ref extra) = request.extra_headers {
        overrides.extend(extra.clone());
    }

    overrides
}

fn safe_body_snippet(body: &str) -> String {
    body.chars().take(240).collect()
}

fn should_fallback_to_chat(status: u16, body: &str) -> bool {
    if status == 404 {
        return true;
    }
    let body_lower = body.to_ascii_lowercase();
    body_lower.contains("not found") && body_lower.contains("response")
}

fn enrich_endpoint_error(
    err: LlmConnectorError,
    provider: &str,
    endpoint: &str,
    status: Option<u16>,
    body: Option<&str>,
) -> LlmConnectorError {
    let status_txt = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let body_txt = body.map(safe_body_snippet).unwrap_or_default();
    let prefix = format!(
        "provider={} endpoint={} status={} body={} ",
        provider, endpoint, status_txt, body_txt
    );

    match err {
        LlmConnectorError::AuthenticationError(msg) => {
            LlmConnectorError::AuthenticationError(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::RateLimitError(msg) => {
            LlmConnectorError::RateLimitError(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::InvalidRequest(msg) => {
            LlmConnectorError::InvalidRequest(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::NotFoundError(msg) => {
            LlmConnectorError::NotFoundError(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::ServerError(msg) => {
            LlmConnectorError::ServerError(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::ParseError(msg) => {
            LlmConnectorError::ParseError(format!("{}{}", prefix, msg))
        }
        LlmConnectorError::ApiError(msg) => {
            LlmConnectorError::ApiError(format!("{}{}", prefix, msg))
        }
        other => LlmConnectorError::ApiError(format!("{}{}", prefix, other)),
    }
}

fn usage_to_responses_usage(usage: Option<&crate::types::Usage>) -> Option<ResponsesUsage> {
    usage.map(|u| ResponsesUsage {
        input_tokens: Some(u.prompt_tokens),
        output_tokens: Some(u.completion_tokens),
        total_tokens: Some(u.total_tokens),
        extra: HashMap::new(),
    })
}

/// Generic provider implementation
///
/// This struct provides generic implementation for most standard LLM APIs.
/// It uses Protocol trait to handle API-specific format conversion,
/// uses HttpClient to handle network communication.
#[derive(Debug)]
pub struct GenericProvider<P: Protocol> {
    protocol: P,
    client: super::HttpClient,
}

impl<P: Protocol> GenericProvider<P> {
    /// Create new generic provider
    pub fn new(protocol: P, client: super::HttpClient) -> Self {
        Self { protocol, client }
    }

    /// Get protocol reference
    pub fn protocol(&self) -> &P {
        &self.protocol
    }

    /// Get client reference
    pub fn client(&self) -> &super::HttpClient {
        &self.client
    }

    pub fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        self.protocol.capabilities()
    }
}

impl<P: Protocol> Clone for GenericProvider<P> {
    fn clone(&self) -> Self {
        Self {
            protocol: self.protocol.clone(),
            client: self.client.clone(),
        }
    }
}

#[async_trait]
impl<P: Protocol> Provider for GenericProvider<P> {
    fn name(&self) -> &str {
        self.protocol.name()
    }

    fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        self.protocol.capabilities()
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError> {
        validate_chat_request_capabilities(&self.protocol, request)?;
        let protocol_request = self.protocol.build_chat_request_body(request)?;

        // Dynamic endpoint resolution
        // Note: We use a placeholder base_url here because actual resolution happens inside resolve_endpoint
        // But protocol.chat_endpoint() expects a base_url string.
        // So we need to determine base_url first.
        let base_url = request
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.client.base_url());
        let url = self
            .protocol
            .resolve_chat_endpoint(base_url, &request.model);
        let overrides = build_request_overrides(&self.protocol, request);

        // Execute request with overrides
        let response = if overrides.is_empty() {
            self.client.post(&url, &protocol_request).await?
        } else {
            self.client
                .post_with_overrides(&url, &protocol_request, &overrides)
                .await?
        };

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;

        if !status.is_success() {
            // Try to parse detailed error from JSON body
            let error_detail = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                format!("HTTP {} - Body: {}", status, json)
            } else {
                format!("HTTP {} - Body: {}", status, text)
            };

            return Err(self.protocol.map_error(status.as_u16(), &error_detail));
        }
        self.protocol.normalize_chat_response(&text)
    }

    #[cfg(feature = "streaming")]
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, LlmConnectorError> {
        let mut streaming_request = request.clone();
        streaming_request.stream = Some(true);

        validate_chat_request_capabilities(&self.protocol, &streaming_request)?;

        let protocol_request = self.protocol.build_chat_request_body(&streaming_request)?;

        let base_url = request
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.client.base_url());
        let url = self.protocol.chat_stream_endpoint(base_url, &request.model);
        let overrides = build_request_overrides(&self.protocol, request);

        let response = if overrides.is_empty() {
            self.client.stream(&url, &protocol_request).await?
        } else {
            self.client
                .stream_with_overrides(&url, &protocol_request, &overrides)
                .await?
        };

        let status = response.status();

        if !status.is_success() {
            let text = response
                .text()
                .await
                .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;
            return Err(self.protocol.map_error(status.as_u16(), &text));
        }
        self.protocol.interpret_chat_stream(response).await
    }

    async fn models(&self) -> Result<Vec<String>, LlmConnectorError> {
        let endpoint = self
            .protocol
            .models_endpoint(self.client.base_url())
            .ok_or_else(|| {
                LlmConnectorError::UnsupportedOperation(format!(
                    "{} does not support model listing",
                    self.protocol.name()
                ))
            })?;

        let response = self.client.get(&endpoint).await?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;

        if !status.is_success() {
            return Err(self.protocol.map_error(status.as_u16(), &text));
        }

        self.protocol.parse_models(&text)
    }

    async fn embed(&self, request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError> {
        validate_embed_request_capabilities(&self.protocol, request)?;
        let endpoint = self
            .protocol
            .embed_endpoint(self.client.base_url(), &request.model)
            .ok_or_else(|| {
                LlmConnectorError::UnsupportedOperation(format!(
                    "{} does not support embeddings",
                    self.protocol.name()
                ))
            })?;

        let protocol_request = self.protocol.build_embed_request(request)?;
        let response = self.client.post(&endpoint, &protocol_request).await?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;

        if !status.is_success() {
            return Err(self.protocol.map_error(status.as_u16(), &text));
        }

        self.protocol.parse_embed_response(&text)
    }

    async fn invoke_responses(
        &self,
        request: &ResponsesRequest,
    ) -> Result<ResponsesResponse, LlmConnectorError> {
        validate_responses_request_capabilities(&self.protocol, request)?;
        let base_url = request
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.client.base_url());

        if let Some(url) = self.protocol.responses_endpoint(base_url, &request.model) {
            log::info!(
                "llm-connector responses path=direct provider={} endpoint={}",
                self.protocol.name(),
                url
            );

            let protocol_request = self
                .protocol
                .build_responses_request(request)
                .map_err(|e| {
                    enrich_endpoint_error(
                        e,
                        self.protocol.name(),
                        "/v1/responses",
                        None,
                        Some("build_responses_request_failed"),
                    )
                })?;

            let overrides = build_responses_request_overrides(&self.protocol, request);
            let response = if overrides.is_empty() {
                self.client.post(&url, &protocol_request).await?
            } else {
                self.client
                    .post_with_overrides(&url, &protocol_request, &overrides)
                    .await?
            };

            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;

            if status.is_success() {
                return self.protocol.parse_responses_response(&text).map_err(|e| {
                    enrich_endpoint_error(
                        e,
                        self.protocol.name(),
                        "/v1/responses",
                        Some(status.as_u16()),
                        Some(&text),
                    )
                });
            }

            if should_fallback_to_chat(status.as_u16(), &text) {
                log::warn!(
                    "llm-connector responses path=fallback provider={} reason=endpoint_unsupported status={} body={}",
                    self.protocol.name(),
                    status.as_u16(),
                    safe_body_snippet(&text)
                );
            } else {
                let err = self.protocol.map_error(status.as_u16(), &text);
                return Err(enrich_endpoint_error(
                    err,
                    self.protocol.name(),
                    "/v1/responses",
                    Some(status.as_u16()),
                    Some(&text),
                ));
            }
        }

        log::info!(
            "llm-connector responses path=fallback provider={} reason=no_direct_endpoint",
            self.protocol.name()
        );

        let chat_request = responses_request_to_chat_request(request).map_err(|e| {
            enrich_endpoint_error(
                e,
                self.protocol.name(),
                "responses->chat mapping",
                None,
                None,
            )
        })?;
        let chat_response = self.chat(&chat_request).await.map_err(|e| {
            enrich_endpoint_error(e, self.protocol.name(), "/v1/chat/completions", None, None)
        })?;

        Ok(chat_response_to_responses_response(&chat_response))
    }

    #[cfg(feature = "streaming")]
    async fn invoke_responses_stream(
        &self,
        request: &ResponsesRequest,
    ) -> Result<ResponsesStream, LlmConnectorError> {
        validate_responses_request_capabilities(&self.protocol, request)?;
        let base_url = request
            .base_url
            .as_deref()
            .unwrap_or_else(|| self.client.base_url());

        if let Some(url) = self.protocol.responses_endpoint(base_url, &request.model) {
            log::info!(
                "llm-connector responses_stream path=direct provider={} endpoint={}",
                self.protocol.name(),
                url
            );

            let mut stream_request = request.clone();
            stream_request.stream = Some(true);
            let protocol_request = self
                .protocol
                .build_responses_request(&stream_request)
                .map_err(|e| {
                    enrich_endpoint_error(
                        e,
                        self.protocol.name(),
                        "/v1/responses",
                        None,
                        Some("build_responses_stream_request_failed"),
                    )
                })?;

            let overrides = build_responses_request_overrides(&self.protocol, &stream_request);
            let response = if overrides.is_empty() {
                self.client.stream(&url, &protocol_request).await?
            } else {
                self.client
                    .stream_with_overrides(&url, &protocol_request, &overrides)
                    .await?
            };

            let status = response.status();
            if status.is_success() {
                let provider = self.protocol.name().to_string();
                let endpoint = "/v1/responses".to_string();
                let stream =
                    crate::sse::create_text_stream(response, crate::sse::StreamFormat::Auto).map(
                        move |item| {
                            let payload = item?;
                            serde_json::from_str::<ResponsesStreamEvent>(&payload).map_err(|e| {
                                enrich_endpoint_error(
                                    LlmConnectorError::ParseError(format!(
                                        "Failed to parse responses stream event: {}",
                                        e
                                    )),
                                    &provider,
                                    &endpoint,
                                    None,
                                    Some(&payload),
                                )
                            })
                        },
                    );
                return Ok(Box::pin(stream));
            }

            let text = response
                .text()
                .await
                .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;

            if should_fallback_to_chat(status.as_u16(), &text) {
                log::warn!(
                    "llm-connector responses_stream path=fallback provider={} reason=endpoint_unsupported status={} body={}",
                    self.protocol.name(),
                    status.as_u16(),
                    safe_body_snippet(&text)
                );
            } else {
                let err = self.protocol.map_error(status.as_u16(), &text);
                return Err(enrich_endpoint_error(
                    err,
                    self.protocol.name(),
                    "/v1/responses",
                    Some(status.as_u16()),
                    Some(&text),
                ));
            }
        }

        log::info!(
            "llm-connector responses_stream path=fallback provider={} reason=no_direct_endpoint",
            self.protocol.name()
        );

        let mut chat_request = responses_request_to_chat_request(request).map_err(|e| {
            enrich_endpoint_error(
                e,
                self.protocol.name(),
                "responses->chat mapping",
                None,
                None,
            )
        })?;
        chat_request.stream = Some(true);

        let chat_stream = self.chat_stream(&chat_request).await.map_err(|e| {
            enrich_endpoint_error(e, self.protocol.name(), "/v1/chat/completions", None, None)
        })?;

        struct FallbackState {
            created: bool,
            response_id: String,
            model: Option<String>,
        }

        let stream = chat_stream
            .scan(
                FallbackState {
                    created: false,
                    response_id: String::new(),
                    model: Some(chat_request.model.clone()),
                },
                |state, item| {
                    let mut out = Vec::<Result<ResponsesStreamEvent, LlmConnectorError>>::new();
                    match item {
                        Err(e) => out.push(Err(e)),
                        Ok(chunk) => {
                            if !state.created {
                                state.created = true;
                                state.response_id = if chunk.id.is_empty() {
                                    format!(
                                        "resp_{}{}",
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis(),
                                        rand::random::<u16>()
                                    )
                                } else {
                                    chunk.id.clone()
                                };

                                out.push(Ok(ResponsesStreamEvent::response_created(
                                    state.response_id.clone(),
                                    state.model.clone(),
                                )));
                            }

                            if let Some(delta) = chunk.get_content()
                                && !delta.is_empty()
                            {
                                out.push(Ok(ResponsesStreamEvent::output_text_delta(
                                    state.response_id.clone(),
                                    delta,
                                )));
                            }

                            let finished = chunk
                                .choices
                                .first()
                                .and_then(|c| c.finish_reason.as_ref())
                                .is_some();
                            if finished {
                                out.push(Ok(ResponsesStreamEvent::response_completed(
                                    state.response_id.clone(),
                                    usage_to_responses_usage(chunk.usage.as_ref()),
                                    state.model.clone(),
                                )));
                            }
                        }
                    }

                    std::future::ready(Some(out))
                },
            )
            .flat_map(futures_util::stream::iter);

        Ok(Box::pin(stream))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::validate_chat_request_capabilities;
    use crate::{
        protocols::{
            AnthropicProtocol, OllamaProtocol, OpenAIProtocol,
            common::capabilities::ProviderCapabilities,
        },
        types::{ChatRequest, Tool, ToolChoice},
    };

    #[test]
    fn test_capability_precheck_rejects_tool_choice_when_unsupported() {
        let protocol = OllamaProtocol::new();
        let request = ChatRequest::new("llama3.2")
            .add_message(crate::types::Message::user("hello"))
            .with_tool_choice(ToolChoice::auto());

        let error = validate_chat_request_capabilities(&protocol, &request)
            .expect_err("ollama should reject tool_choice precheck");

        match error {
            crate::error::LlmConnectorError::UnsupportedOperation(message) => {
                assert!(message.contains("tool_choice"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_capability_precheck_rejects_thinking_controls_when_unsupported() {
        let protocol = OllamaProtocol::new();
        let request = ChatRequest::new("llama3.2")
            .add_message(crate::types::Message::user("hello"))
            .with_enable_thinking(true);

        let error = validate_chat_request_capabilities(&protocol, &request)
            .expect_err("ollama should reject thinking controls precheck");

        match error {
            crate::error::LlmConnectorError::UnsupportedOperation(message) => {
                assert!(message.contains("enable_thinking"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_ollama_capabilities_exposed() {
        let capabilities = ProviderCapabilities::ollama();
        assert!(!capabilities.supports_tool_choice);
        assert_eq!(
            capabilities.reasoning_request_strategy,
            crate::protocols::common::capabilities::ReasoningRequestStrategy::Unsupported
        );
        assert!(capabilities.supports_multimodal_input);
    }

    #[test]
    fn test_capability_precheck_allows_anthropic_tool_choice() {
        let protocol = AnthropicProtocol::new("test-key");
        let request = ChatRequest::new("claude-3-5-sonnet")
            .add_message(crate::types::Message::user("hello"))
            .with_tool_choice(ToolChoice::auto());

        validate_chat_request_capabilities(&protocol, &request)
            .expect("anthropic should accept tool_choice precheck");
    }

    #[test]
    fn test_capability_precheck_allows_openai_required_tool_choice_non_stream() {
        let protocol = OpenAIProtocol::new("test-key");
        let weather_tool = Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type":"object"}),
        );
        let request = ChatRequest::new("gpt-4o-mini")
            .add_message(crate::types::Message::user("hello"))
            .with_tools(vec![weather_tool])
            .with_tool_choice(ToolChoice::required());

        validate_chat_request_capabilities(&protocol, &request)
            .expect("openai should accept required tool_choice precheck");
    }

    #[test]
    fn test_capability_precheck_allows_openai_specific_tool_choice_stream() {
        let protocol = OpenAIProtocol::new("test-key");
        let weather_tool = Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type":"object"}),
        );
        let request = ChatRequest::new("gpt-4o-mini")
            .add_message(crate::types::Message::user("hello"))
            .with_tools(vec![weather_tool])
            .with_tool_choice(ToolChoice::function("get_weather"))
            .with_stream(true);

        validate_chat_request_capabilities(&protocol, &request)
            .expect("openai should accept specific tool_choice precheck in stream mode");
    }
}
