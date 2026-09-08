//! Google Gemini Protocol Implementation
//!
//! This module provides the Google Gemini API protocol.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::ProviderCapabilities;
use crate::types::{
    ChatRequest, ChatResponse, Choice, DocumentSource, EmbedRequest, EmbedResponse, EmbeddingData,
    FunctionCall, ImageSource, Message, MessageBlock, Role, ToolCall, Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct GoogleProtocol;

impl GoogleProtocol {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Protocol for GoogleProtocol {
    type Request = GoogleRequest;
    type Response = GoogleResponse;

    fn name(&self) -> &str {
        "google"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::google()
    }

    fn chat_endpoint(&self, base_url: &str, model: &str) -> String {
        format!(
            "{}/models/{}:generateContent",
            base_url.trim_end_matches('/'),
            model
        )
    }

    #[cfg(feature = "streaming")]
    fn chat_stream_endpoint(&self, base_url: &str, model: &str) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            base_url.trim_end_matches('/'),
            model
        )
    }

    fn models_endpoint(&self, base_url: &str) -> Option<String> {
        Some(format!("{}/models", base_url.trim_end_matches('/')))
    }

    fn embed_endpoint(&self, base_url: &str, model: &str) -> Option<String> {
        Some(format!(
            "{}/models/{}:batchEmbedContents",
            base_url.trim_end_matches('/'),
            model
        ))
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        Ok(GoogleRequest::from(request))
    }

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        let google_response: GoogleResponse =
            serde_json::from_str(response).map_err(LlmConnectorError::JsonError)?;

        let chat_response: ChatResponse = google_response.into();

        // Populate reasoning content if present in usage_metadata or parts
        // Note: Gemini 2.0 Thinking puts thoughts in usage_metadata.thoughts_token_count
        // but the actual text is usually in a special part or handled by the provider.
        // If the library users use `with_enable_thinking`, we should try to extract it if possible.
        // Currently, our ChatResponse::from(GoogleResponse) handles token counts.

        Ok(chat_response)
    }

    fn parse_models(&self, response: &str) -> Result<Vec<String>, LlmConnectorError> {
        let models_response: GoogleModelsResponse =
            serde_json::from_str(response).map_err(LlmConnectorError::JsonError)?;

        Ok(models_response
            .models
            .into_iter()
            .map(|m| m.name.replace("models/", ""))
            .collect())
    }

    fn build_embed_request(
        &self,
        request: &EmbedRequest,
    ) -> Result<serde_json::Value, LlmConnectorError> {
        let requests: Vec<GoogleEmbedRequest> = request
            .input
            .iter()
            .map(|text| GoogleEmbedRequest {
                model: format!("models/{}", request.model),
                content: GoogleContent {
                    role: String::new(),
                    parts: vec![GooglePart::Text { text: text.clone() }],
                },
            })
            .collect();

        let req_body = GoogleBatchEmbedRequest { requests };
        serde_json::to_value(req_body).map_err(LlmConnectorError::JsonError)
    }

    fn parse_embed_response(&self, response: &str) -> Result<EmbedResponse, LlmConnectorError> {
        let google_response: GoogleBatchEmbedResponse =
            serde_json::from_str(response).map_err(LlmConnectorError::JsonError)?;

        let mut data = Vec::new();
        if let Some(embeddings) = google_response.embeddings {
            for (index, emb) in embeddings.into_iter().enumerate() {
                data.push(EmbeddingData {
                    object: "embedding".to_string(),
                    embedding: emb.values,
                    index: index as u32,
                });
            }
        }

        Ok(EmbedResponse {
            object: "list".to_string(),
            data,
            model: "google".to_string(),
            usage: Usage::default(),
        })
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        LlmConnectorError::ProviderError(format!("Google API error: {} - {}", status, body))
    }

    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        use crate::sse::sse_events;
        use crate::types::{Delta, StreamingChoice, StreamingResponse};
        use futures_util::StreamExt;

        let stream = sse_events(response)
            .scan(false, move |sent_role, event_result| {
                let mapped: Result<Option<StreamingResponse>, LlmConnectorError> =
                    match event_result {
                        Ok(json_str) => {
                            if json_str.trim().is_empty() {
                                Ok(None)
                            } else {
                                let google_resp: GoogleResponse =
                                    match serde_json::from_str(&json_str) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            return std::future::ready(Some(Err(
                                                LlmConnectorError::JsonError(e),
                                            )));
                                        }
                                    };

                                // Extract incremental text, reasoning, and tool calls
                                let (content, reasoning, tool_calls, finish_reason) = google_resp
                                    .candidates
                                    .as_ref()
                                    .and_then(|c| c.first())
                                    .map(|candidate| {
                                        // Text content
                                        let text = candidate
                                            .content
                                            .as_ref()
                                            .and_then(|c| {
                                                c.parts.iter().find_map(|p| match p {
                                                    GooglePart::Text { text } => Some(text.clone()),
                                                    _ => None,
                                                })
                                            })
                                            .unwrap_or_default();

                                        // Reasoning (thought)
                                        let thought = candidate.content.as_ref().and_then(|c| {
                                            c.parts.iter().find_map(|p| match p {
                                                GooglePart::Thought { text, .. } => {
                                                    Some(text.clone())
                                                }
                                                _ => None,
                                            })
                                        });

                                        // Tool calls extraction
                                        let tools: Vec<ToolCall> = candidate
                                            .content
                                            .as_ref()
                                            .map(|c| {
                                                c.parts
                                                    .iter()
                                                    .filter_map(|p| match p {
                                                        GooglePart::FunctionCall {
                                                            function_call,
                                                            thought_signature,
                                                        } => Some(ToolCall {
                                                            id: function_call.name.clone(),
                                                            call_type: "function".to_string(),
                                                            function: FunctionCall {
                                                                name: function_call.name.clone(),
                                                                arguments: function_call
                                                                    .args
                                                                    .to_string(),
                                                                thought_signature:
                                                                    thought_signature.clone(),
                                                            },
                                                            index: None,
                                                            thought_signature: thought_signature
                                                                .clone(),
                                                        }),
                                                        _ => None,
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();

                                        (text, thought, tools, candidate.finish_reason.clone())
                                    })
                                    .unwrap_or_default();

                                let usage = google_resp.usage_metadata.map(|u| Usage {
                                    prompt_tokens: u.prompt_token_count.unwrap_or(0),
                                    completion_tokens: u.candidates_token_count.unwrap_or(0)
                                        + u.thoughts_token_count.unwrap_or(0),
                                    total_tokens: u.total_token_count.unwrap_or(0),
                                    ..Default::default()
                                });

                                if content.is_empty()
                                    && reasoning.is_none()
                                    && finish_reason.is_none()
                                    && usage.is_none()
                                    && tool_calls.is_empty()
                                {
                                    Ok(None)
                                } else {
                                    let role = if !*sent_role {
                                        *sent_role = true;
                                        Some(Role::Assistant)
                                    } else {
                                        None
                                    };

                                    Ok(Some(StreamingResponse {
                                        id: "google".to_string(),
                                        object: "chat.completion.chunk".to_string(),
                                        created: chrono::Utc::now().timestamp() as u64,
                                        model: "google".to_string(),
                                        choices: vec![StreamingChoice {
                                            index: 0,
                                            delta: Delta {
                                                role,
                                                content: if content.is_empty() {
                                                    None
                                                } else {
                                                    Some(content.clone())
                                                },
                                                reasoning_content: reasoning,
                                                tool_calls: if tool_calls.is_empty() {
                                                    None
                                                } else {
                                                    Some(tool_calls)
                                                },
                                                ..Default::default()
                                            },
                                            finish_reason,
                                            logprobs: None,
                                        }],
                                        content,
                                        usage,
                                        ..Default::default()
                                    }))
                                }
                            }
                        }
                        Err(e) => Err(e),
                    };

                std::future::ready(Some(mapped))
            })
            .filter_map(|x| async move {
                match x {
                    Ok(Some(v)) => Some(Ok(v)),
                    Ok(None) => None,
                    Err(e) => Some(Err(e)),
                }
            });

        Ok(Box::pin(stream))
    }
}

// ============================================================================
// Google API Types
// ============================================================================

#[derive(Serialize, Deserialize)]
pub struct GoogleRequest {
    pub contents: Vec<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GoogleGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GoogleTool>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "toolConfig")]
    pub tool_config: Option<GoogleToolConfig>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleTool {
    #[serde(rename = "functionDeclarations")]
    pub function_declarations: Vec<GoogleFunctionDeclaration>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleFunctionDeclaration {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: GoogleFunctionCallingConfig,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleFunctionCallingConfig {
    pub mode: String, // "AUTO", "ANY", "NONE"
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "allowedFunctionNames")]
    pub allowed_function_names: Vec<String>,
}

impl From<&ChatRequest> for GoogleRequest {
    fn from(req: &ChatRequest) -> Self {
        let reasoning_parts = crate::protocols::common::thinking::map_reasoning_request_parts(
            req,
            crate::protocols::common::capabilities::ProviderCapabilities::google(),
        );

        let contents = req
            .messages
            .iter()
            .map(|msg| {
                let parts = msg
                    .content
                    .iter()
                    .map(|block| match block {
                        MessageBlock::Text { text } => GooglePart::Text { text: text.clone() },
                        MessageBlock::Thinking { thinking, .. } => GooglePart::Text {
                            text: thinking.clone(),
                        },
                        MessageBlock::Image {
                            source: ImageSource::Base64 { media_type, data },
                        } => GooglePart::InlineData {
                            inline_data: GoogleInlineData {
                                mime_type: media_type.clone(),
                                data: data.clone(),
                            },
                        },
                        MessageBlock::Image { .. } => GooglePart::Text {
                            text: "".to_string(),
                        },
                        MessageBlock::Document { source } => match source {
                            DocumentSource::Base64 { media_type, data } => GooglePart::InlineData {
                                inline_data: GoogleInlineData {
                                    mime_type: media_type.clone(),
                                    data: data.clone(),
                                },
                            },
                        },
                        _ => GooglePart::Text {
                            text: "".to_string(),
                        },
                    })
                    .collect::<Vec<_>>();

                let mut final_parts = parts;

                // Handle tool calls in assistant messages
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        final_parts.push(GooglePart::FunctionCall {
                            function_call: GoogleFunctionCall {
                                name: tc.function.name.clone(),
                                args: tc
                                    .arguments_value()
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                            },
                            thought_signature: tc
                                .thought_signature
                                .clone()
                                .or(tc.function.thought_signature.clone()),
                        });
                    }
                }

                // Handle tool responses
                if msg.role == Role::Tool
                    && let Some(id) = &msg.tool_call_id
                {
                    // In Gemini, FunctionResponse name must match the call
                    // We use tool_call_id as the name if possible, or we might need more context
                    final_parts.push(GooglePart::FunctionResponse {
                        function_response: GoogleFunctionResponse {
                            name: id.clone(),
                            response: serde_json::from_str(&msg.content_as_text())
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        },
                    });
                }

                GoogleContent {
                    role: match msg.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "model".to_string(),
                        Role::System => "user".to_string(),
                        Role::Tool => "user".to_string(),
                    },
                    parts: final_parts,
                }
            })
            .collect();

        let tools = req.tools.as_ref().map(|t| {
            vec![GoogleTool {
                function_declarations: t
                    .iter()
                    .map(|tool| GoogleFunctionDeclaration {
                        name: tool.function.name.clone(),
                        description: tool.function.description.clone(),
                        parameters: tool.function.parameters.clone(),
                    })
                    .collect(),
            }]
        });

        let tool_config = req.tool_choice.as_ref().map(|tc| {
            let (mode, allowed) = match tc {
                crate::types::ToolChoice::Mode(m) => match m.as_str() {
                    "none" => ("NONE", vec![]),
                    "auto" => ("AUTO", vec![]),
                    "required" => ("ANY", vec![]),
                    _ => ("AUTO", vec![]),
                },
                crate::types::ToolChoice::Function { function, .. } => {
                    ("ANY", vec![function.name.clone()])
                }
            };
            GoogleToolConfig {
                function_calling_config: GoogleFunctionCallingConfig {
                    mode: mode.to_string(),
                    allowed_function_names: allowed,
                },
            }
        });

        GoogleRequest {
            contents,
            tools,
            tool_config,
            generation_config: Some(GoogleGenerationConfig {
                temperature: req.temperature,
                top_p: req.top_p,
                max_output_tokens: req.max_tokens,
                thinking_config: reasoning_parts
                    .enable_thinking
                    .map(|b| GoogleThinkingConfig {
                        include_thoughts: b,
                    }),
            }),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct GoogleContent {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub parts: Vec<GooglePart>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum GooglePart {
    Thought {
        text: String,
        thought: bool,
    },
    Text {
        text: String,
    },
    InlineData {
        inline_data: GoogleInlineData,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GoogleFunctionCall,
        #[serde(skip_serializing_if = "Option::is_none", rename = "thoughtSignature")]
        thought_signature: Option<String>,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GoogleFunctionResponse,
    },
}

#[derive(Serialize, Deserialize)]
pub struct GoogleFunctionCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleFunctionResponse {
    pub name: String,
    pub response: serde_json::Value,
}

impl GooglePart {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct GoogleInlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<GoogleThinkingConfig>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleThinkingConfig {
    pub include_thoughts: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleResponse {
    pub candidates: Option<Vec<GoogleCandidate>>,
    #[serde(rename = "usageMetadata")]
    pub usage_metadata: Option<GoogleUsageMetadata>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleCandidate {
    pub content: Option<GoogleContent>,
    #[serde(rename = "finishReason")]
    pub finish_reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    pub prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<u32>,
    #[serde(rename = "totalTokenCount")]
    pub total_token_count: Option<u32>,
    #[serde(rename = "thoughtsTokenCount")]
    pub thoughts_token_count: Option<u32>,
}

impl From<GoogleResponse> for ChatResponse {
    fn from(value: GoogleResponse) -> Self {
        let mut tool_calls = Vec::new();
        let mut reasoning_content = None;
        let mut final_content = String::new();
        let mut finish_reason = None;

        if let Some(candidates) = value.candidates
            && let Some(candidate) = candidates.into_iter().next()
        {
            finish_reason = candidate.finish_reason;
            if let Some(content) = candidate.content {
                for part in content.parts {
                    match part {
                        GooglePart::Text { text } => {
                            if !final_content.is_empty() {
                                final_content.push('\n');
                            }
                            final_content.push_str(&text);
                        }
                        GooglePart::FunctionCall {
                            function_call,
                            thought_signature,
                        } => {
                            tool_calls.push(crate::types::ToolCall {
                                id: function_call.name.clone(), // Use name as ID
                                call_type: "function".to_string(),
                                function: crate::types::FunctionCall {
                                    name: function_call.name,
                                    arguments: function_call.args.to_string(),
                                    thought_signature: thought_signature.clone(),
                                },
                                index: Some(tool_calls.len()),
                                thought_signature,
                            });
                        }
                        GooglePart::Thought { text, .. } => {
                            reasoning_content = Some(text);
                        }
                        _ => {}
                    }
                }
            }
        }

        let choice = Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: vec![crate::types::MessageBlock::text(final_content.clone())],
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                reasoning_content: reasoning_content.clone(),
                ..Default::default()
            },
            finish_reason,
            logprobs: None,
        };

        let usage = value.usage_metadata.map(|u| Usage {
            prompt_tokens: u.prompt_token_count.unwrap_or(0),
            completion_tokens: u.candidates_token_count.unwrap_or(0)
                + u.thoughts_token_count.unwrap_or(0),
            total_tokens: u.total_token_count.unwrap_or(0),
            ..Default::default()
        });

        ChatResponse {
            id: "google".to_string(),
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp() as u64,
            model: "google".to_string(),
            choices: vec![choice],
            content: final_content,
            reasoning_content,
            usage,
            system_fingerprint: None,
        }
    }
}

#[derive(Deserialize)]
pub struct GoogleModelsResponse {
    pub models: Vec<GoogleModel>,
}

#[derive(Deserialize)]
pub struct GoogleModel {
    pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleBatchEmbedRequest {
    pub requests: Vec<GoogleEmbedRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleEmbedRequest {
    pub model: String,
    pub content: GoogleContent,
}

#[derive(Deserialize)]
pub struct GoogleBatchEmbedResponse {
    pub embeddings: Option<Vec<GoogleEmbedding>>,
}

#[derive(Deserialize)]
pub struct GoogleEmbedding {
    pub values: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn test_google_thinking_config() {
        let req = ChatRequest::new("gemini-2.0-flash")
            .add_message(Message::user("test"))
            .with_enable_thinking(true);

        let google_req = GoogleRequest::from(&req);

        // Verify thinking_config is set
        assert!(google_req.generation_config.is_some());
        let config = google_req.generation_config.unwrap();
        assert!(config.thinking_config.is_some());
        assert!(config.thinking_config.unwrap().include_thoughts);
    }

    #[test]
    fn test_google_thinking_config_disabled() {
        let req = ChatRequest::new("gemini-2.0-flash")
            .add_message(Message::user("test"))
            .with_enable_thinking(false);

        let google_req = GoogleRequest::from(&req);

        // Verify thinking_config is set to false
        assert!(google_req.generation_config.is_some());
        let config = google_req.generation_config.unwrap();
        assert!(config.thinking_config.is_some());
        assert!(!config.thinking_config.unwrap().include_thoughts);
    }

    #[test]
    fn test_google_thinking_config_none() {
        let req = ChatRequest::new("gemini-2.0-flash").add_message(Message::user("test"));

        let google_req = GoogleRequest::from(&req);

        // Verify thinking_config is NOT set
        assert!(google_req.generation_config.is_some());
        let config = google_req.generation_config.unwrap();
        assert!(config.thinking_config.is_none());
    }

    #[tokio::test]
    async fn test_parse_stream_response_tool_call() {
        use futures::stream::TryStreamExt;
        use serde_json::json;

        const TOOL_NAME: &str = "set_light_values";
        let google_response = GoogleResponse {
            candidates: Some(vec![GoogleCandidate {
                content: Some(GoogleContent {
                    role: "model".to_string(),
                    parts: vec![GooglePart::FunctionCall {
                        function_call: GoogleFunctionCall {
                            name: TOOL_NAME.to_string(),
                            args: json!(r#"{"brightness": 25, "color_temp": "warm"}"#),
                        },
                        thought_signature: None,
                    }],
                }),
                finish_reason: None,
            }]),
            usage_metadata: None,
        };
        let google_response_serialized = serde_json::to_string(&google_response).unwrap();
        let resp = reqwest::Response::from(http::response::Response::new(format!(
            "data: {google_response_serialized}\n\n"
        )));
        let protocol = GoogleProtocol::new();
        let streaming_response: Option<crate::types::StreamingResponse> =
            match protocol.parse_stream_response(resp).await {
                Ok(resp) => match resp.try_collect::<Vec<_>>().await {
                    Ok(v) => Some(v[0].clone()),
                    _ => None,
                },
                _ => None,
            };

        // Verify tool calls
        assert!(streaming_response.is_some_and(|sr| {
            sr.choices[0]
                .delta
                .tool_calls
                .as_ref()
                .is_some_and(|tc| tc[0].function.name == TOOL_NAME)
        }))
    }
}
