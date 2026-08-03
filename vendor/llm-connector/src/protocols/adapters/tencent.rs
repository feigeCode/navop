//! Tencent Cloud Protocol Implementation
//!
//! Implements Tencent Cloud API v3 signature (TC3-HMAC-SHA256).
//! Reference: https://cloud.tencent.com/document/api/1729/101837

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::ProviderCapabilities;
#[cfg(feature = "streaming")]
use crate::types::ChatStream;
use crate::types::{ChatRequest, ChatResponse, Choice, Message, Role, Usage};
use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "streaming")]
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Tencent Cloud Native Protocol
#[derive(Debug, Clone)]
pub struct TencentNativeProtocol {
    secret_id: String,
    secret_key: String,
    region: String,
}

impl TencentNativeProtocol {
    pub fn new(secret_id: &str, secret_key: &str) -> Self {
        Self {
            secret_id: secret_id.to_string(),
            secret_key: secret_key.to_string(),
            region: "ap-guangzhou".to_string(), // Default region
        }
    }

    pub fn with_region(mut self, region: &str) -> Self {
        self.region = region.to_string();
        self
    }

    /// Calculate TC3-HMAC-SHA256 signature
    pub fn calculate_signature(
        &self,
        host: &str,
        payload: &str,
        timestamp: i64,
        date: &str,
    ) -> Result<String, LlmConnectorError> {
        let service = "hunyuan";
        let algorithm = "TC3-HMAC-SHA256";

        // 1. Concatenate CanonicalRequest
        let http_request_method = "POST";
        let canonical_uri = "/";
        let canonical_query_string = "";
        let canonical_headers = format!("content-type:application/json\nhost:{}\n", host);
        let signed_headers = "content-type;host";

        let mut hasher = Sha256::new();
        hasher.update(payload);
        let hashed_request_payload = hex::encode(hasher.finalize());

        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            http_request_method,
            canonical_uri,
            canonical_query_string,
            canonical_headers,
            signed_headers,
            hashed_request_payload
        );

        // 2. String to sign
        let credential_scope = format!("{}/{}/tc3_request", date, service);

        let mut hasher = Sha256::new();
        hasher.update(canonical_request.as_bytes());
        let hashed_canonical_request = hex::encode(hasher.finalize());

        let string_to_sign = format!(
            "{}\n{}\n{}\n{}",
            algorithm, timestamp, credential_scope, hashed_canonical_request
        );

        // 3. Calculate Signature
        let k_key = format!("TC3{}", self.secret_key);
        let k_date = hmac_sha256(k_key.as_bytes(), date.as_bytes())?;
        let k_service = hmac_sha256(&k_date, service.as_bytes())?;
        let k_signing = hmac_sha256(&k_service, "tc3_request".as_bytes())?;

        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes())?);

        // 4. Build Authorization Header
        Ok(format!(
            "{} Credential={}/{}, SignedHeaders={}, Signature={}",
            algorithm, self.secret_id, credential_scope, signed_headers, signature
        ))
    }
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Result<Vec<u8>, LlmConnectorError> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|e| LlmConnectorError::ProviderError(format!("HMAC init failed: {}", e)))?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[async_trait]
impl Protocol for TencentNativeProtocol {
    type Request = TencentRequest;
    type Response = TencentResponse;

    fn name(&self) -> &str {
        "tencent"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::tencent()
    }

    fn chat_endpoint(&self, _base_url: &str, _model: &str) -> String {
        // Native API uses a fixed endpoint usually, but we respect base_url if provided
        // Official endpoint: https://hunyuan.tencentcloudapi.com
        if _base_url.is_empty() || _base_url == "https://api.openai.com" {
            "https://hunyuan.tencentcloudapi.com".to_string()
        } else if !_base_url.contains("://") {
            format!("https://{}", _base_url)
        } else {
            _base_url.to_string()
        }
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        // Native signature requires body, handled in Provider
        vec![]
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        let reasoning_parts = crate::protocols::common::thinking::map_reasoning_request_parts(
            request,
            self.capabilities(),
        );

        // Convert to Tencent format
        Ok(TencentRequest {
            model: Some(request.model.clone()),
            messages: request
                .messages
                .iter()
                .map(|m| TencentMessage {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        Role::System => "system".to_string(),
                        _ => "user".to_string(),
                    },
                    content: {
                        let tb = m.thinking_blocks_concat();
                        let body = m.content_as_text();
                        if tb.is_empty() {
                            body
                        } else if body.is_empty() {
                            tb
                        } else {
                            format!("{tb}\n{body}")
                        }
                    },
                })
                .collect(),
            temperature: request.temperature,
            top_p: request.top_p,
            stream: request.stream,
            enable_thinking: reasoning_parts.enable_thinking,
        })
    }

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        let resp: TencentResponse = serde_json::from_str(response)
            .map_err(|e| LlmConnectorError::InvalidRequest(format!("Parse error: {}", e)))?;

        if let Some(err) = resp.response.error {
            return Err(LlmConnectorError::ApiError(format!(
                "{}: {}",
                err.code, err.message
            )));
        }

        let task = resp.response; // Inner wrapper

        let content = task
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let choice = Choice {
            index: 0,
            message: Message::text(Role::Assistant, &content),
            finish_reason: Some("stop".to_string()),
            logprobs: None,
        };

        Ok(ChatResponse {
            id: task.request_id.unwrap_or_default(),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp() as u64,
            model: "hunyuan".to_string(), // Model usually not in response
            choices: vec![choice],
            content,
            reasoning_content: None,
            usage: task.usage.map(|u| Usage {
                prompt_tokens: u.prompt_tokens as u32,
                completion_tokens: u.completion_tokens as u32,
                total_tokens: u.total_tokens as u32,
                prompt_cache_hit_tokens: None,
                prompt_cache_miss_tokens: None,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
            system_fingerprint: None,
        })
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        LlmConnectorError::from_status_code(status, body.to_string())
    }

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ChatStream, LlmConnectorError> {
        use crate::sse::sse_events;
        use crate::types::{Delta, StreamingChoice, StreamingResponse};

        let event_stream = sse_events(response);

        let stream = event_stream
            .map(|event_result| {
                match event_result {
                    Ok(json_str) => {
                        // Skip [DONE]
                        if json_str.trim() == "[DONE]" {
                            return None;
                        }

                        // Parse Tencent-specific PascalCase JSON
                        match serde_json::from_str::<TencentStreamingResponse>(&json_str) {
                            Ok(tencent_resp) => {
                                let content = tencent_resp
                                    .choices
                                    .first()
                                    .and_then(|c| c.delta.content.clone())
                                    .unwrap_or_default();

                                let role = tencent_resp
                                    .choices
                                    .first()
                                    .and_then(|c| c.delta.role.clone())
                                    .map(|r| match r.as_str() {
                                        "user" => Role::User,
                                        "assistant" => Role::Assistant,
                                        "system" => Role::System,
                                        _ => Role::Assistant,
                                    });

                                let finish_reason = tencent_resp
                                    .choices
                                    .first()
                                    .and_then(|c| c.finish_reason.clone());

                                // Convert to unified StreamingResponse
                                Some(Ok(StreamingResponse {
                                    id: tencent_resp.id.unwrap_or_default(),
                                    object: "chat.completion.chunk".to_string(),
                                    created: tencent_resp
                                        .created
                                        .unwrap_or_else(|| Utc::now().timestamp())
                                        as u64,
                                    model: "hunyuan".to_string(),
                                    choices: vec![StreamingChoice {
                                        index: 0,
                                        delta: Delta {
                                            role,
                                            content: Some(content.clone()),
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
                                    content, // Convenience field
                                    usage: tencent_resp.usage.map(|u| Usage {
                                        prompt_tokens: u.prompt_tokens as u32,
                                        completion_tokens: u.completion_tokens as u32,
                                        total_tokens: u.total_tokens as u32,
                                        prompt_cache_hit_tokens: None,
                                        prompt_cache_miss_tokens: None,
                                        prompt_tokens_details: None,
                                        completion_tokens_details: None,
                                    }),
                                    reasoning_content: None,
                                    system_fingerprint: None,
                                }))
                            }
                            Err(e) => Some(Err(LlmConnectorError::ParseError(format!(
                                "Failed to parse Tencent stream chunk: {} | Original: {}",
                                e, json_str
                            )))),
                        }
                    }
                    Err(e) => Some(Err(e)),
                }
            })
            .filter_map(std::future::ready);

        Ok(Box::pin(stream))
    }
}

// Internal Types
#[derive(Debug, Serialize, Deserialize)]
pub struct TencentRequest {
    #[serde(rename = "Model")]
    pub model: Option<String>,
    #[serde(rename = "Messages")]
    pub messages: Vec<TencentMessage>,
    #[serde(rename = "Temperature", skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(rename = "TopP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(rename = "Stream", skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(rename = "EnableThinking", skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TencentMessage {
    #[serde(rename = "Role")]
    pub role: String,
    #[serde(rename = "Content")]
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct TencentResponse {
    #[serde(rename = "Response")]
    pub response: TencentResponseInner,
}

#[derive(Debug, Deserialize)]
pub struct TencentResponseInner {
    #[serde(rename = "RequestId")]
    pub request_id: Option<String>,
    #[serde(rename = "Error")]
    pub error: Option<TencentError>,
    #[serde(rename = "Choices")]
    pub choices: Option<Vec<TencentChoice>>,
    #[serde(rename = "Usage")]
    pub usage: Option<TencentUsage>,
}

#[derive(Debug, Deserialize)]
pub struct TencentError {
    #[serde(rename = "Code")]
    pub code: String,
    #[serde(rename = "Message")]
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct TencentChoice {
    #[serde(rename = "Message")]
    pub message: TencentMessage,
    #[serde(rename = "FinishReason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TencentUsage {
    #[serde(rename = "PromptTokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "CompletionTokens")]
    pub completion_tokens: i64,
    #[serde(rename = "TotalTokens")]
    pub total_tokens: i64,
}

#[derive(Debug, Deserialize)]
pub struct TencentStreamingResponse {
    #[serde(rename = "Note")]
    pub note: Option<String>,
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "Created")]
    pub created: Option<i64>,
    #[serde(rename = "Choices")]
    pub choices: Vec<TencentStreamingChoice>,
    #[serde(rename = "Usage")]
    pub usage: Option<TencentUsage>,
}

#[derive(Debug, Deserialize)]
pub struct TencentStreamingChoice {
    #[serde(rename = "Delta")]
    pub delta: TencentStreamingDelta,
    #[serde(rename = "FinishReason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TencentStreamingDelta {
    #[serde(rename = "Role")]
    pub role: Option<String>,
    #[serde(rename = "Content")]
    pub content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_streaming_response() {
        let json_1 = r#"{"Note":"Example","Id":"123","Created":1234567890,"Choices":[{"Delta":{"Role":"assistant","Content":"Hello"},"FinishReason":""}]}"#;
        let resp_1: TencentStreamingResponse = serde_json::from_str(json_1).unwrap();
        assert_eq!(resp_1.choices[0].delta.content.as_deref(), Some("Hello"));
        assert_eq!(resp_1.choices[0].delta.role.as_deref(), Some("assistant"));

        let json_2 = r#"{"Id":"123","Created":1234567890,"Choices":[{"Delta":{"Content":" world"},"FinishReason":""}]}"#;
        let resp_2: TencentStreamingResponse = serde_json::from_str(json_2).unwrap();
        assert_eq!(resp_2.choices[0].delta.content.as_deref(), Some(" world"));

        let json_3 = r#"{"Id":"123","Created":1234567890,"Choices":[{"Delta":{"Content":""},"FinishReason":"stop"}],"Usage":{"PromptTokens":10,"CompletionTokens":20,"TotalTokens":30}}"#;
        let resp_3: TencentStreamingResponse = serde_json::from_str(json_3).unwrap();
        assert_eq!(resp_3.usage.unwrap().total_tokens, 30);
        assert_eq!(resp_3.choices[0].finish_reason.as_deref(), Some("stop"));
    }
}
