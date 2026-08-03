//! OpenAI Protocol Implementation - V2 Architecture
//!
//! This module implements the standard OpenAI API protocol specification.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::{ContentBlockMode, ProviderCapabilities};
use crate::protocols::common::openai_compatible::{
    OpenAICompatibleCapabilities, build_openai_compatible_request_parts,
    parse_openai_compatible_chat_response,
};
use crate::protocols::common::transport::resolve_endpoint;
use crate::types::{
    ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, ReasoningEffort, ResponsesRequest,
    ResponsesResponse,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// OpenAIprotocolimplementation
#[derive(Clone, Debug)]
pub struct OpenAIProtocol {
    api_key: String,
    service_name: String,
}

impl OpenAIProtocol {
    /// Create new OpenAI Protocol instance
    pub fn new(api_key: &str) -> Self {
        Self::with_service(api_key, "openai")
    }

    pub fn with_service(api_key: &str, service_name: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            service_name: service_name.to_string(),
        }
    }

    /// GetAPI key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    fn service_capabilities(&self) -> ProviderCapabilities {
        match self.service_name.as_str() {
            "openai" | "azure-openai" => ProviderCapabilities::openai(),
            "zhipu" => ProviderCapabilities::zhipu_openai_compatible(),
            "moonshot" | "deepseek" => {
                let mut capabilities = ProviderCapabilities::openai_compatible_text_only();
                capabilities.reasoning_request_strategy =
                    crate::protocols::common::capabilities::ReasoningRequestStrategy::EnableThinking;
                capabilities.stream_reasoning_strategy =
                    crate::protocols::common::capabilities::StreamReasoningStrategy::SeparateField;
                capabilities.region_key_scope_sensitive = self.service_name.as_str() == "moonshot";
                capabilities.requires_region_routing = self.service_name.as_str() == "moonshot";
                capabilities
            }
            "minimax" | "abab" => {
                let mut capabilities = ProviderCapabilities::openai_compatible_text_only();
                capabilities.region_key_scope_sensitive = self.service_name.as_str() == "minimax";
                capabilities.requires_region_routing = self.service_name.as_str() == "minimax";
                capabilities
            }
            "openrouter" => ProviderCapabilities::openrouter(),
            "xinference" => {
                let mut capabilities = ProviderCapabilities::openai_compatible_text_only();
                capabilities.auth_kind = crate::protocols::common::capabilities::AuthKind::None;
                capabilities.supports_embeddings = true;
                capabilities
            }
            _ => ProviderCapabilities::openai(),
        }
    }

    fn capabilities_for_model(&self, model: &str) -> OpenAICompatibleCapabilities {
        let base_capabilities = self.service_capabilities();
        let model_lower = model.to_lowercase();
        let content_block_mode = match base_capabilities.content_block_mode {
            ContentBlockMode::Standard if self.service_name == "openai" => {
                if model_lower.contains("deepseek")
                    || model_lower.contains("moonshot")
                    || model_lower.contains("abab")
                    || model_lower.contains("minimax")
                {
                    ContentBlockMode::TextOnly
                } else {
                    ContentBlockMode::Standard
                }
            }
            other => other,
        };

        OpenAICompatibleCapabilities {
            content_block_mode,
            supports_tool_choice: base_capabilities.supports_tool_choice,
            supports_response_format: base_capabilities.supports_response_format,
            reasoning_request_strategy: base_capabilities.reasoning_request_strategy,
            stream_reasoning_strategy: base_capabilities.stream_reasoning_strategy,
            empty_assistant_tool_content_strategy: base_capabilities
                .empty_assistant_tool_content_strategy,
        }
    }
}

#[async_trait]
impl Protocol for OpenAIProtocol {
    type Request = OpenAIRequest;
    type Response = crate::protocols::formats::chat_completions::ChatCompletionsResponse;

    fn name(&self) -> &str {
        &self.service_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.service_capabilities()
    }

    fn chat_endpoint(&self, base_url: &str, _model: &str) -> String {
        resolve_endpoint(base_url, "", "/chat/completions")
    }

    fn resolve_chat_endpoint(&self, base_url: &str, model: &str) -> String {
        self.chat_endpoint(base_url, model)
    }

    fn models_endpoint(&self, base_url: &str) -> Option<String> {
        Some(resolve_endpoint(base_url, "", "/models"))
    }

    fn embed_endpoint(&self, base_url: &str, _model: &str) -> Option<String> {
        Some(resolve_endpoint(base_url, "", "/embeddings"))
    }

    fn responses_endpoint(&self, base_url: &str, _model: &str) -> Option<String> {
        Some(resolve_endpoint(base_url, "", "/responses"))
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        let capabilities = self.capabilities_for_model(&request.model);
        let parts = build_openai_compatible_request_parts(request, &capabilities)?;

        Ok(OpenAIRequest {
            model: request.model.clone(),
            messages: parts.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stream: request.stream,
            tools: parts.tools,
            tool_choice: parts.tool_choice,
            response_format: parts.response_format,
            reasoning_effort: parts.reasoning_effort,
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
        parse_openai_compatible_chat_response(
            response,
            self.name(),
            self.service_capabilities().stream_reasoning_strategy,
        )
    }

    fn normalize_chat_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        self.parse_response(response)
    }

    fn parse_models(&self, response: &str) -> Result<Vec<String>, LlmConnectorError> {
        let models_response: OpenAIModelsResponse =
            serde_json::from_str(response).map_err(|e| {
                LlmConnectorError::ParseError(format!("Failed to parse models response: {}", e))
            })?;

        Ok(models_response
            .data
            .into_iter()
            .map(|model| model.id)
            .collect())
    }

    fn build_embed_request(
        &self,
        request: &EmbedRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        let req = OpenAIEmbedRequest {
            model: request.model.clone(),
            input: request.input.clone(),
            encoding_format: request.encoding_format.clone(),
            user: request.user.clone(),
        };
        serde_json::to_value(req).map_err(|e| {
            LlmConnectorError::ParseError(format!("Failed to serialize embed request: {}", e))
        })
    }

    fn parse_embed_response(&self, response: &str) -> Result<EmbedResponse, LlmConnectorError> {
        crate::protocols::formats::chat_completions::parse_chat_completions_embed_response(
            response,
            self.name(),
        )
    }

    fn build_responses_request(
        &self,
        request: &ResponsesRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        serde_json::to_value(request).map_err(|e| {
            LlmConnectorError::ParseError(format!("Failed to serialize responses request: {}", e))
        })
    }

    fn parse_responses_response(
        &self,
        response: &str,
    ) -> Result<ResponsesResponse, LlmConnectorError> {
        let mut parsed = serde_json::from_str::<ResponsesResponse>(response)
            .map_err(|e| LlmConnectorError::ParseError(format!("{}: {}", self.name(), e)))?;
        parsed.populate_output_text();
        Ok(parsed)
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        let error_info = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error").cloned())
            .unwrap_or_else(|| serde_json::json!({"message": body}));

        let message = error_info
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown OpenAI error");

        // Check error code for context length exceeded
        let error_code = error_info
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let msg = format!("OpenAI: {}", message);

        // Detect context length exceeded from error code or message content
        if error_code == "context_length_exceeded"
            || message.contains("maximum context length")
            || message.contains("context_length_exceeded")
        {
            return LlmConnectorError::ContextLengthExceeded(msg);
        }

        match status {
            400 => LlmConnectorError::InvalidRequest(msg),
            401 => LlmConnectorError::AuthenticationError(msg),
            403 => LlmConnectorError::PermissionError(msg),
            429 => LlmConnectorError::RateLimitError(msg),
            500..=599 => LlmConnectorError::ServerError(msg),
            _ => LlmConnectorError::ApiError(format!("OpenAI HTTP {}: {}", status, message)),
        }
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

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        let stream_reasoning_strategy = self.capabilities().stream_reasoning_strategy;
        Ok(
            crate::protocols::common::openai_compatible::parse_openai_compatible_stream(
                response,
                crate::sse::StreamingParseMode::OpenAIOnly,
                stream_reasoning_strategy,
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

// OpenAIrequesttype
#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: serde_json::Value, // Support String or Array
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// Model list response
#[derive(Deserialize, Debug)]
pub struct OpenAIModelsResponse {
    pub data: Vec<OpenAIModel>,
}

#[derive(Deserialize, Debug)]
pub struct OpenAIModel {
    pub id: String,
}

// Embedding Data Structures
#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIEmbedRequest {
    pub model: String,
    pub input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionCall, Message, MessageBlock, Role, Tool, ToolCall, ToolChoice};

    #[test]
    fn test_openai_request_downgrade_for_deepseek() {
        let protocol = OpenAIProtocol::new("test-key");
        let request = ChatRequest {
            model: "deepseek-chat".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: vec![MessageBlock::text("Part 1"), MessageBlock::text("Part 2")],
                ..Default::default()
            }],
            ..Default::default()
        };

        let req = protocol.build_request(&request).unwrap();
        let content = req.messages[0].get("content").unwrap();

        // Should be downgraded to a single string
        assert!(content.is_string());
        assert_eq!(content.as_str().unwrap(), "Part 1Part 2");
    }

    #[test]
    fn test_openai_request_keep_array_for_gpt4() {
        let protocol = OpenAIProtocol::new("test-key");
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: vec![MessageBlock::text("Part 1"), MessageBlock::text("Part 2")],
                ..Default::default()
            }],
            ..Default::default()
        };

        let req = protocol.build_request(&request).unwrap();
        let content = req.messages[0].get("content").unwrap();

        // Should be kept as array
        assert!(content.is_array());
        let arr = content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_openai_request_uses_null_content_for_assistant_tool_calls() {
        let protocol = OpenAIProtocol::new("test-key");
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"city":"Paris"}"#.to_string(),
                thought_signature: None,
            },
            ..Default::default()
        };
        let request = ChatRequest::new("gpt-4o-mini")
            .add_message(Message::assistant_with_tool_calls(vec![tool_call.clone()]));

        let req = protocol.build_request(&request).unwrap();
        let message = &req.messages[0];

        assert_eq!(message["role"], "assistant");
        assert_eq!(message["tool_calls"], serde_json::json!([tool_call]));
        assert!(message["content"].is_null());
        assert_ne!(message["content"], serde_json::json!([]));
    }

    #[test]
    fn test_text_only_openai_compatible_request_uses_empty_string_for_assistant_tool_calls() {
        let protocol = OpenAIProtocol::with_service("test-key", "deepseek");
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"city":"Tokyo"}"#.to_string(),
                thought_signature: None,
            },
            ..Default::default()
        };
        let request = ChatRequest::new("deepseek-chat")
            .add_message(Message::assistant_with_tool_calls(vec![tool_call.clone()]));

        let req = protocol.build_request(&request).unwrap();
        let message = &req.messages[0];

        assert_eq!(message["tool_calls"], serde_json::json!([tool_call]));
        assert_eq!(message["content"], serde_json::json!(""));
        assert_ne!(message["content"], serde_json::json!([]));
    }

    #[test]
    fn test_openai_request_fail_for_deepseek_with_image() {
        let protocol = OpenAIProtocol::new("test-key");
        let request = ChatRequest {
            model: "deepseek-chat".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: vec![MessageBlock::image_base64("image/png", "base64data")],
                ..Default::default()
            }],
            ..Default::default()
        };

        let result = protocol.build_request(&request);
        assert!(result.is_err());
        match result {
            Err(LlmConnectorError::InvalidRequest(msg)) => {
                assert!(msg.contains("complex content blocks"));
            }
            _ => panic!("Expected InvalidRequest error, got {:?}", result),
        }
    }

    #[test]
    fn test_openai_request_serializes_required_tool_choice() {
        let protocol = OpenAIProtocol::new("test-key");
        let request = ChatRequest::new("gpt-4o-mini")
            .add_message(Message::user("hello"))
            .with_tools(vec![Tool::function(
                "get_weather",
                Some("Get weather".to_string()),
                serde_json::json!({"type":"object"}),
            )])
            .with_tool_choice(ToolChoice::required());

        let req = protocol.build_request(&request).unwrap();
        assert!(req.tools.is_some());
        assert_eq!(req.tool_choice, Some(serde_json::json!("required")));
    }

    #[test]
    fn test_openai_request_serializes_specific_function_tool_choice() {
        let protocol = OpenAIProtocol::new("test-key");
        let request = ChatRequest::new("gpt-4o-mini")
            .add_message(Message::user("hello"))
            .with_tools(vec![Tool::function(
                "get_weather",
                Some("Get weather".to_string()),
                serde_json::json!({"type":"object"}),
            )])
            .with_tool_choice(ToolChoice::function("get_weather"));

        let req = protocol.build_request(&request).unwrap();
        assert!(req.tools.is_some());
        assert_eq!(
            req.tool_choice,
            Some(serde_json::json!({
                "type": "function",
                "function": { "name": "get_weather" }
            }))
        );
    }
}
