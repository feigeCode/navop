//! Zhipu GLM Protocol Implementation
//!
//! This module provides the private Zhipu GLM API protocol.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::{
    ContentBlockMode, ProviderCapabilities, ReasoningRequestStrategy, StreamReasoningStrategy,
};
use crate::protocols::common::openai_compatible::{
    OpenAICompatibleCapabilities, build_openai_compatible_request_parts,
    parse_openai_compatible_chat_response,
};
use crate::protocols::common::transport::resolve_endpoint;
use crate::types::{ChatRequest, ChatResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZhipuApiMode {
    Native,
    OpenAICompatible,
}

/// Zhipu GLM private protocol implementation
#[derive(Clone, Debug)]
pub struct ZhipuProtocol {
    api_key: String,
    mode: ZhipuApiMode,
}

impl ZhipuProtocol {
    /// Create new Zhipu Protocol instance (using native format)
    pub fn new(api_key: &str) -> Self {
        Self::with_mode(api_key, ZhipuApiMode::Native)
    }

    /// Create Zhipu Protocol instance using OpenAI compatible format
    pub fn new_openai_compatible(api_key: &str) -> Self {
        Self::with_mode(api_key, ZhipuApiMode::OpenAICompatible)
    }

    pub fn with_mode(api_key: &str, mode: ZhipuApiMode) -> Self {
        Self {
            api_key: api_key.to_string(),
            mode,
        }
    }

    /// Get API key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn mode(&self) -> ZhipuApiMode {
        self.mode
    }

    /// Whether to use OpenAI compatible format
    pub fn is_openai_compatible(&self) -> bool {
        matches!(self.mode, ZhipuApiMode::OpenAICompatible)
    }

    fn capabilities(&self) -> OpenAICompatibleCapabilities {
        let provider_capabilities = crate::protocols::common::capabilities::ProviderCapabilities::zhipu_openai_compatible();

        match self.mode {
            ZhipuApiMode::Native | ZhipuApiMode::OpenAICompatible => OpenAICompatibleCapabilities {
                content_block_mode: ContentBlockMode::Standard,
                supports_tool_choice: true,
                supports_response_format: false,
                reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
                stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
                empty_assistant_tool_content_strategy: provider_capabilities
                    .empty_assistant_tool_content_strategy,
            },
        }
    }

    fn parse_chat_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        match self.mode {
            ZhipuApiMode::Native | ZhipuApiMode::OpenAICompatible => {
                parse_openai_compatible_chat_response(
                    response,
                    self.name(),
                    crate::core::Protocol::capabilities(self).stream_reasoning_strategy,
                )
            }
        }
    }

    #[cfg(feature = "streaming")]
    fn streaming_parse_mode(&self) -> crate::sse::StreamingParseMode {
        match self.mode {
            ZhipuApiMode::Native | ZhipuApiMode::OpenAICompatible => {
                crate::sse::StreamingParseMode::OpenAIOnly
            }
        }
    }
}

#[async_trait::async_trait]
impl Protocol for ZhipuProtocol {
    type Request = ZhipuRequest;
    type Response = crate::protocols::formats::chat_completions::ChatCompletionsResponse;

    fn name(&self) -> &str {
        "zhipu"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::zhipu_openai_compatible()
    }

    fn chat_endpoint(&self, base_url: &str, _model: &str) -> String {
        match self.mode {
            ZhipuApiMode::Native | ZhipuApiMode::OpenAICompatible => {
                resolve_endpoint(base_url, "/api/paas/v4", "/chat/completions")
            }
        }
    }

    fn resolve_chat_endpoint(&self, base_url: &str, model: &str) -> String {
        self.chat_endpoint(base_url, model)
    }

    fn auth_strategy(&self) -> crate::protocols::common::auth::AuthStrategy {
        crate::protocols::common::auth::AuthStrategy::Bearer {
            api_key: self.api_key.clone(),
        }
    }

    fn override_auth_strategy(
        &self,
        api_key: &str,
    ) -> crate::protocols::common::auth::AuthStrategy {
        crate::protocols::common::auth::AuthStrategy::Bearer {
            api_key: api_key.to_string(),
        }
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        let parts = build_openai_compatible_request_parts(request, &self.capabilities())?;

        // Zhipu doesn't support float values for temperature > 1.0 or highly specific fields in some models,
        // but the main issue is usually the message format if multimodal is used.
        // Let's ensure the request is clean.

        // Zhipu often requires do_sample if temperature is set
        let do_sample = if request.temperature.is_some() || request.top_p.is_some() {
            Some(true)
        } else {
            None
        };

        Ok(ZhipuRequest {
            model: request.model.clone(),
            messages: parts.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stream: request.stream,
            tools: parts.tools,
            tool_choice: parts.tool_choice,
            do_sample,
            enable_thinking: parts.reasoning_parts.enable_thinking,
        })
    }

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

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        self.parse_chat_response(response)
    }

    fn normalize_chat_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        self.parse_response(response)
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        let body_lower = body.to_lowercase();
        if body_lower.contains("context_length_exceeded")
            || body_lower.contains("maximum context length")
            || body_lower.contains("token limit")
        {
            return LlmConnectorError::ContextLengthExceeded(format!("Zhipu: {}", body));
        }
        LlmConnectorError::from_status_code(status, format!("Zhipu API error: {}", body))
    }

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        Ok(
            crate::protocols::common::openai_compatible::parse_openai_compatible_stream(
                response,
                self.streaming_parse_mode(),
                crate::core::Protocol::capabilities(self).stream_reasoning_strategy,
            ),
        )
    }

    #[cfg(feature = "streaming")]
    async fn interpret_chat_stream(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        self.parse_stream_response(response).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZhipuRequest {
    pub model: String,
    pub messages: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_sample: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZhipuMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
