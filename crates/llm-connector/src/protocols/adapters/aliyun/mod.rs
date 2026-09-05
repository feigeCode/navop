//! Aliyun DashScope Protocol Implementation
//!
//! This module provides the private Aliyun DashScope API protocol.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::ProviderCapabilities;
#[cfg(feature = "streaming")]
use crate::protocols::common::streamers::map_sse_json_stream;
use crate::protocols::common::transport::resolve_prefixed_endpoint;
use crate::types::{ChatRequest, ChatResponse, EmbedRequest, EmbedResponse};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Aliyun DashScope private protocol implementation
#[derive(Debug, Clone)]
pub struct AliyunProtocol {
    api_key: String,
}

impl AliyunProtocol {
    /// Create new Aliyun Protocol instance
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
        }
    }

    /// Get API key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Get streaming request additional headers
    pub fn streaming_headers(&self) -> Vec<(String, String)> {
        vec![("X-DashScope-SSE".to_string(), "enable".to_string())]
    }
}

#[async_trait]
impl Protocol for AliyunProtocol {
    type Request = AliyunRequest;
    type Response = crate::protocols::formats::chat_completions::ChatCompletionsResponse;

    fn name(&self) -> &str {
        "aliyun"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::aliyun()
    }

    fn chat_endpoint(&self, base_url: &str, _model: &str) -> String {
        resolve_prefixed_endpoint(
            base_url,
            "/api/v1",
            "/services/aigc/text-generation/generation",
        )
    }

    fn embed_endpoint(&self, base_url: &str, _model: &str) -> Option<String> {
        Some(resolve_prefixed_endpoint(
            base_url,
            "/api/v1",
            "/services/embeddings/text-embedding/text-embedding",
        ))
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
        let aliyun_messages =
            crate::protocols::common::request::openai_message_converter(&request.messages);
        let reasoning_parts = crate::protocols::common::thinking::map_reasoning_request_parts(
            request,
            self.capabilities(),
        );

        Ok(AliyunRequest {
            model: request.model.clone(),
            input: AliyunInput {
                messages: aliyun_messages,
            },
            parameters: AliyunParameters {
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                top_p: request.top_p,
                result_format: "message".to_string(),
                incremental_output: if request.stream.unwrap_or(false) {
                    Some(true)
                } else {
                    None
                },
                enable_thinking: reasoning_parts.enable_thinking,
                tools: request.tools.clone(),
                tool_choice: request.tool_choice.clone(),
            },
        })
    }

    fn build_embed_request(
        &self,
        request: &EmbedRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        let req = AliyunEmbedRequest {
            model: request.model.clone(),
            input: AliyunEmbedInput {
                texts: request.input.clone(),
            },
            parameters: request
                .encoding_format
                .as_deref()
                .map(|f| AliyunEmbedParameters {
                    text_type: f.to_string(),
                }),
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

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        Ok(map_sse_json_stream(response, |json_str| {
            parse_aliyun_stream_event(&json_str).map(Some)
        }))
    }

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        crate::protocols::formats::chat_completions::parse_chat_completions_chat_response(
            response,
            self.name(),
            self.capabilities().stream_reasoning_strategy,
        )
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        let body_lower = body.to_lowercase();
        if body_lower.contains("context_length_exceeded")
            || body_lower.contains("maximum context length")
            || body_lower.contains("input is too long")
        {
            return LlmConnectorError::ContextLengthExceeded(format!("Aliyun: {}", body));
        }
        LlmConnectorError::from_status_code(status, format!("Aliyun API error: {}", body))
    }
}

// Aliyun-specific data structures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunRequest {
    pub model: String,
    pub input: AliyunInput,
    pub parameters: AliyunParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunInput {
    pub messages: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunEmbedRequest {
    pub model: String,
    pub input: AliyunEmbedInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<AliyunEmbedParameters>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunEmbedInput {
    pub texts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunEmbedParameters {
    pub text_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<crate::types::ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliyunParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub result_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<crate::types::Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<crate::types::ToolChoice>,
}

#[cfg(feature = "streaming")]
fn parse_aliyun_stream_event(
    json_str: &str,
) -> Result<crate::types::StreamingResponse, LlmConnectorError> {
    use crate::types::{Delta, Role, StreamingChoice, StreamingResponse, Usage};

    let raw: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
        LlmConnectorError::ParseError(format!(
            "Failed to parse Aliyun streaming event: {}. Content: {}",
            e, json_str
        ))
    })?;

    let output = raw
        .get("output")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            LlmConnectorError::ParseError(format!(
                "Aliyun streaming event missing output field. Content: {}",
                json_str
            ))
        })?;

    let choices = output
        .get("choices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            LlmConnectorError::ParseError(format!(
                "Aliyun streaming event missing output.choices field. Content: {}",
                json_str
            ))
        })?;

    let first_choice = choices.first().and_then(|v| v.as_object()).ok_or_else(|| {
        LlmConnectorError::ParseError(format!(
            "Aliyun streaming event missing first choice. Content: {}",
            json_str
        ))
    })?;

    let message = first_choice
        .get("message")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            LlmConnectorError::ParseError(format!(
                "Aliyun streaming event missing choice.message field. Content: {}",
                json_str
            ))
        })?;

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let role = match message.get("role").and_then(|v| v.as_str()) {
        Some("system") => Some(Role::System),
        Some("user") => Some(Role::User),
        Some("assistant") => Some(Role::Assistant),
        Some("tool") => Some(Role::Tool),
        _ => None,
    };

    let finish_reason = first_choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .and_then(|reason| match reason {
            "" | "null" => None,
            other => Some(other.to_string()),
        });

    let usage = raw.get("usage").and_then(|value| {
        let prompt_tokens = value
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let completion_tokens = value
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let total_tokens = value
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or((prompt_tokens + completion_tokens) as u64)
            as u32;

        if prompt_tokens > 0 || completion_tokens > 0 || total_tokens > 0 {
            Some(Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
                ..Default::default()
            })
        } else {
            None
        }
    });

    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(StreamingResponse {
        id: raw
            .get("request_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("aliyun-{}", created)),
        object: "chat.completion.chunk".to_string(),
        created,
        model: String::new(),
        choices: vec![StreamingChoice {
            index: 0,
            delta: Delta {
                role,
                content: if content.is_empty() {
                    None
                } else {
                    Some(content.clone())
                },
                tool_calls: None,
                reasoning_content: None,
                reasoning: None,
                thought: None,
                thinking: None,
                thinking_signature: None,
            },
            finish_reason,
            logprobs: None,
        }],
        content,
        reasoning_content: None,
        usage,
        system_fingerprint: None,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_aliyun_stream_event;

    #[cfg(feature = "streaming")]
    #[test]
    fn test_parse_aliyun_stream_event_delta_chunk() {
        let chunk = r#"{"output":{"choices":[{"message":{"content":"Hello","role":"assistant"},"finish_reason":"null"}]},"usage":{"total_tokens":14,"output_tokens":1,"input_tokens":13},"request_id":"req_1"}"#;

        let parsed = parse_aliyun_stream_event(chunk).expect("should parse aliyun stream chunk");
        assert_eq!(parsed.id, "req_1");
        assert_eq!(parsed.content, "Hello");
        assert_eq!(parsed.choices.len(), 1);
        assert_eq!(parsed.choices[0].delta.content.as_deref(), Some("Hello"));
        assert_eq!(parsed.choices[0].finish_reason, None);
        assert_eq!(parsed.usage.as_ref().map(|u| u.prompt_tokens), Some(13));
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_parse_aliyun_stream_event_final_chunk() {
        let chunk = r#"{"output":{"choices":[{"message":{"content":"","role":"assistant"},"finish_reason":"stop"}]},"usage":{"total_tokens":24,"output_tokens":11,"input_tokens":13},"request_id":"req_2"}"#;

        let parsed = parse_aliyun_stream_event(chunk).expect("should parse aliyun final chunk");
        assert_eq!(parsed.id, "req_2");
        assert_eq!(parsed.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.usage.as_ref().map(|u| u.total_tokens), Some(24));
    }
}
