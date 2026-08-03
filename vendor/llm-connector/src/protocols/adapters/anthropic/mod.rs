//! Anthropic Claude Protocol Implementation - V2 Architecture
//!
//! This module implements the Anthropic Claude API protocol specification.

use crate::core::Protocol;
use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::ProviderCapabilities;
use crate::types::{
    AnthropicToolChoice, AnthropicToolDefinition, ChatRequest, ChatResponse, Choice,
    FunctionCall, Message, Role, ToolCall, ToolChoice, Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Anthropic Claudeprotocolimplementation
#[derive(Clone, Debug)]
pub struct AnthropicProtocol {
    api_key: String,
}

impl AnthropicProtocol {
    /// Create new Anthropic Protocol instance
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
        }
    }

    /// GetAPI key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

#[async_trait]
impl Protocol for AnthropicProtocol {
    type Request = AnthropicRequest;
    type Response = AnthropicResponse;

    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::anthropic()
    }

    fn chat_endpoint(&self, base_url: &str, _model: &str) -> String {
        let base = base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/messages", base)
        } else {
            format!("{}/v1/messages", base)
        }
    }

    fn build_request(&self, request: &ChatRequest) -> Result<Self::Request, LlmConnectorError> {
        // Anthropic API requires separating system messages
        let mut system_message = None;
        let mut messages = Vec::new();

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    // Anthropic only supports one system message, placed in a separate field
                    let text = msg.content_as_text();
                    if system_message.is_none() {
                        system_message = Some(text);
                    } else {
                        // If there are multiple system messages, merge them
                        let existing = system_message.take().unwrap_or_default();
                        system_message = Some(format!("{}\n\n{}", existing, text));
                    }
                }
                Role::User => {
                    // Anthropic always uses array format
                    let content = serde_json::to_value(&msg.content).unwrap();
                    messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                Role::Assistant => {
                    // Anthropic uses content blocks and expresses tool calls as tool_use blocks
                    let mut content = serde_json::to_value(&msg.content).unwrap_or_else(|_| {
                        serde_json::Value::Array(vec![serde_json::json!({
                            "type": "text",
                            "text": msg.content_as_text()
                        })])
                    });
                    if !content.is_array() {
                        content = serde_json::Value::Array(vec![serde_json::json!({
                            "type": "text",
                            "text": msg.content_as_text()
                        })]);
                    }
                    if let (Some(tool_calls), Some(content_arr)) =
                        (&msg.tool_calls, content.as_array_mut())
                    {
                        for (index, tool_call) in tool_calls.iter().enumerate() {
                            let tool_id = if tool_call.id.is_empty() {
                                format!("toolu_{}", index)
                            } else {
                                tool_call.id.clone()
                            };
                            let input = tool_call.arguments_value().unwrap_or_else(|_| {
                                serde_json::json!({
                                    "_raw": tool_call.function.arguments
                                })
                            });
                            content_arr.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tool_id,
                                "name": tool_call.function.name,
                                "input": input
                            }));
                        }
                    }
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
                Role::Tool => {
                    // Anthropic expects tool outputs as user tool_result blocks
                    let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                    let text = msg.content_as_text();
                    messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: serde_json::json!([{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": text
                        }]),
                    });
                }
            }
        }

        let reasoning_parts = crate::protocols::common::thinking::map_reasoning_request_parts(
            request,
            self.capabilities(),
        );

        let mut max_tokens = request.max_tokens.unwrap_or(1024);
        let thinking = if let Some(budget) = reasoning_parts.thinking_budget {
            if max_tokens <= budget {
                max_tokens = budget + 4096;
            }
            Some(AnthropicThinking {
                thinking_type: "enabled".to_string(),
                budget_tokens: budget,
            })
        } else {
            None
        };

        Ok(AnthropicRequest {
            model: request.model.clone(),
            max_tokens,
            messages,
            system: system_message,
            temperature: request.temperature,
            top_p: request.top_p,
            stream: request.stream,
            thinking,
            tools: request.anthropic_tools.clone().or_else(|| {
                request.tools.as_ref().map(|tools| {
                    tools
                        .iter()
                        .map(map_generic_tool_to_anthropic)
                        .collect()
                })
            }),
            tool_choice: request
                .anthropic_tool_choice
                .clone()
                .or_else(|| request.tool_choice.as_ref().and_then(map_tool_choice_to_anthropic)),
        })
    }

    fn parse_response(&self, response: &str) -> Result<ChatResponse, LlmConnectorError> {
        let anthropic_response: AnthropicResponse =
            serde_json::from_str(response).map_err(|e| {
                LlmConnectorError::ParseError(format!("Failed to parse Anthropic response: {}", e))
            })?;

        // Extract content blocks and thinking
        let mut message_blocks = Vec::new();
        let mut thinking_content = String::new();
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for block in &anthropic_response.content {
            match block.content_type.as_str() {
                "text" => {
                    if let Some(text) = &block.text {
                        message_blocks.push(crate::types::MessageBlock::text(text));
                        text_content.push_str(text);
                    }
                }
                "thinking" => {
                    if let Some(thinking) = &block.thinking {
                        thinking_content.push_str(thinking);
                        message_blocks.push(crate::types::MessageBlock::Thinking {
                            thinking: thinking.clone(),
                            signature: block.signature.clone(),
                        });
                    }
                }
                "tool_use" => {
                    if let (Some(id), Some(name)) = (&block.id, &block.name) {
                        let arguments = block
                            .input
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "{}".to_string());
                        tool_calls.push(ToolCall {
                            id: id.clone(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: name.clone(),
                                arguments,
                                thought_signature: None,
                            },
                            index: Some(tool_calls.len()),
                            thought_signature: None,
                        });
                    }
                }
                _ => {
                    // Ignore unknown blocks
                }
            }
        }

        let thinking = if !thinking_content.is_empty() {
            Some(thinking_content)
        } else {
            None
        };

        let choices = vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: message_blocks,
                name: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
                reasoning_content: None,
                reasoning: None,
                thought: None,
                thinking,
            },
            finish_reason: Some(
                anthropic_response
                    .stop_reason
                    .map(|reason| {
                        if reason == "tool_use" {
                            "tool_calls".to_string()
                        } else {
                            reason
                        }
                    })
                    .unwrap_or_else(|| "stop".to_string()),
            ),
            logprobs: None,
        }];

        let usage = Some(Usage {
            prompt_tokens: anthropic_response.usage.input_tokens,
            completion_tokens: anthropic_response.usage.output_tokens,
            total_tokens: anthropic_response.usage.input_tokens
                + anthropic_response.usage.output_tokens,
            completion_tokens_details: None,
            prompt_cache_hit_tokens: anthropic_response.usage.cache_read_input_tokens,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: Some(crate::types::PromptTokensDetails {
                cached_tokens: anthropic_response.usage.cache_read_input_tokens,
                cache_read_input_tokens: anthropic_response.usage.cache_read_input_tokens,
                cache_creation_input_tokens: anthropic_response.usage.cache_creation_input_tokens,
            }),
        });

        Ok(ChatResponse {
            id: anthropic_response.id,
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: anthropic_response.model,
            choices,
            content: text_content,
            reasoning_content: None,
            usage,
            system_fingerprint: None,
        })
    }

    fn map_error(&self, status: u16, body: &str) -> LlmConnectorError {
        let error_info = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error").cloned())
            .unwrap_or_else(|| serde_json::json!({"message": body}));

        let message = error_info
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown Anthropic error");

        let error_type = error_info
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let msg = format!("Anthropic: {}", message);

        // Detect context length exceeded
        if error_type == "invalid_request_error"
            && (message.contains("too long")
                || message.contains("maximum")
                || message.contains("context"))
        {
            return LlmConnectorError::ContextLengthExceeded(msg);
        }

        match status {
            400 => LlmConnectorError::InvalidRequest(msg),
            401 => LlmConnectorError::AuthenticationError(msg),
            403 => LlmConnectorError::PermissionError(msg),
            429 => LlmConnectorError::RateLimitError(msg),
            500..=599 => LlmConnectorError::ServerError(msg),
            _ => LlmConnectorError::ApiError(format!("Anthropic HTTP {}: {}", status, message)),
        }
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        let mut headers =
            crate::protocols::common::auth::api_key_header(&self.api_key, "x-api-key");
        headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        headers
    }

    fn build_auth_headers_for_override(&self, api_key: &str) -> Vec<(String, String)> {
        crate::protocols::common::auth::api_key_header(api_key, "x-api-key")
    }

    /// Parse Anthropic streamingresponse
    ///
    /// Anthropic uses different streaming format:
    /// - message_start: Contains message object (with id)
    /// - content_block_start: Start content chunk
    /// - content_block_delta: contentdelta（Contains text）
    /// - content_block_stop: End content chunk
    /// - message_delta: Message delta (contains usage)
    /// - message_stop: Message end
    #[cfg(feature = "streaming")]
    async fn parse_stream_response(
        &self,
        response: reqwest::Response,
    ) -> Result<crate::types::ChatStream, LlmConnectorError> {
        use std::sync::{Arc, Mutex};

        let message_id = Arc::new(Mutex::new(String::new()));

        Ok(crate::protocols::common::streamers::map_sse_json_stream(
            response,
            move |json_str| {
                let message_id = message_id.clone();
                let event = serde_json::from_str::<serde_json::Value>(&json_str).map_err(|e| {
                    LlmConnectorError::ParseError(format!(
                        "Failed to parse Anthropic streaming event: {}. JSON: {}",
                        e, json_str
                    ))
                })?;

                let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match event_type {
                    "message_start" => {
                        if let Some(msg_id) = event
                            .get("message")
                            .and_then(|m| m.get("id"))
                            .and_then(|id| id.as_str())
                            && let Ok(mut id) = message_id.lock()
                        {
                            *id = msg_id.to_string();
                        }
                        Ok(None)
                    }
                    "content_block_start" | "content_block_delta" | "message_delta" => {
                        let id = message_id.lock().map(|id| id.clone()).unwrap_or_default();
                        crate::protocols::common::streamers::interpret_anthropic_event(&event, &id)
                    }
                    _ => Ok(None),
                }
            },
        ))
    }
}

// Anthropicrequesttype
#[derive(Serialize, Deserialize, Debug)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AnthropicThinking {
    #[serde(rename = "type")]
    pub thinking_type: String,
    pub budget_tokens: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: serde_json::Value, // Support String or Array
}

// Anthropicresponsetype
#[derive(Deserialize, Debug)]
pub struct AnthropicResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<AnthropicContent>,
    pub stop_reason: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
    pub thinking: Option<String>,
    pub signature: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
}

fn map_tool_choice_to_anthropic(tool_choice: &ToolChoice) -> Option<AnthropicToolChoice> {
    match tool_choice {
        ToolChoice::Mode(mode) => match mode.as_str() {
            "none" => Some(AnthropicToolChoice::none()),
            "required" => Some(AnthropicToolChoice::any()),
            "auto" => Some(AnthropicToolChoice::auto()),
            _ => Some(AnthropicToolChoice::auto()),
        },
        ToolChoice::Function { function, .. } => Some(AnthropicToolChoice::tool(
            function.name.clone(),
        )),
    }
}

fn map_generic_tool_to_anthropic(tool: &crate::types::Tool) -> AnthropicToolDefinition {
    AnthropicToolDefinition {
        name: Some(tool.function.name.clone()),
        description: tool.function.description.clone(),
        input_schema: Some(tool.function.parameters.clone()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnthropicToolDefinition, MessageBlock, Tool};

    #[test]
    fn test_anthropic_parsing_with_thinking_and_text() {
        let response_json = r#"
        {
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "This is a thinking block",
                    "signature": "sig_123"
                },
                {
                    "type": "text",
                    "text": "Hello there, nice to meet."
                }
            ],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20
            }
        }
        "#;

        let protocol = AnthropicProtocol::new("test-key");
        let result = protocol.parse_response(response_json).unwrap();

        assert_eq!(result.content, "Hello there, nice to meet.");
        assert_eq!(result.choices[0].message.content.len(), 2);
        match &result.choices[0].message.content[0] {
            MessageBlock::Thinking { thinking, signature } => {
                assert_eq!(thinking, "This is a thinking block");
                assert_eq!(signature.as_deref(), Some("sig_123"));
            }
            _ => panic!("expected thinking block"),
        }
        assert_eq!(
            result.choices[0].message.content[1].as_text().unwrap(),
            "Hello there, nice to meet."
        );
        assert_eq!(
            result.choices[0].message.thinking.as_deref(),
            Some("This is a thinking block")
        );
    }

    #[test]
    fn test_anthropic_thinking_signature_round_trip_build_request() {
        let protocol = AnthropicProtocol::new("test-key");
        let response_json = r#"
        {
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "step",
                    "signature": "sig_xyz"
                },
                {
                    "type": "text",
                    "text": "Hi"
                }
            ],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 2 }
        }
        "#;
        let parsed = protocol.parse_response(response_json).unwrap();
        let assistant = parsed.choices[0].message.clone();

        let req = ChatRequest::new("claude-3")
            .add_message(Message::user("ping"))
            .add_message(assistant);
        let mapped = protocol.build_request(&req).unwrap();
        let arr = mapped.messages[1].content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "thinking");
        assert_eq!(arr[0]["thinking"], "step");
        assert_eq!(arr[0]["signature"], "sig_xyz");
        assert_eq!(arr[1]["text"], "Hi");
    }

    #[test]
    fn test_anthropic_parsing_only_thinking() {
        let response_json = r#"
        {
            "id": "msg_124",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Just thinking...",
                    "signature": "sig_124"
                }
            ],
            "stop_reason": "max_tokens",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20
            }
        }
        "#;

        let protocol = AnthropicProtocol::new("test-key");
        let result = protocol.parse_response(response_json).unwrap();

        assert_eq!(result.content, "");
        assert_eq!(result.choices[0].message.content.len(), 1);
        match &result.choices[0].message.content[0] {
            MessageBlock::Thinking { thinking, signature } => {
                assert_eq!(thinking, "Just thinking...");
                assert_eq!(signature.as_deref(), Some("sig_124"));
            }
            _ => panic!("expected thinking block"),
        }
        assert_eq!(
            result.choices[0].message.thinking.as_deref(),
            Some("Just thinking...")
        );
    }

    #[test]
    fn test_anthropic_request_single_tool_with_tool_use_and_result() {
        let protocol = AnthropicProtocol::new("test-key");
        let tool = Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }),
        );
        let assistant_with_tool = Message::assistant_with_tool_calls(vec![ToolCall {
            id: "toolu_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"city":"beijing"}"#.to_string(),
                thought_signature: None,
            },
            index: Some(0),
            thought_signature: None,
        }]);
        let tool_result = Message::tool(r#"{"temp":26}"#, "toolu_1");
        let request = ChatRequest::new("claude-3-5-sonnet")
            .with_tools(vec![tool])
            .with_tool_choice(ToolChoice::function("get_weather"))
            .add_message(Message::user("weather?"))
            .add_message(assistant_with_tool)
            .add_message(tool_result);

        let mapped = protocol.build_request(&request).unwrap();
        assert_eq!(mapped.tools.as_ref().map(|v| v.len()), Some(1));
        assert_eq!(
            mapped.tool_choice.as_ref().map(|c| c.choice_type.as_str()),
            Some("tool")
        );
        assert_eq!(
            mapped.tool_choice.as_ref().and_then(|c| c.name.as_deref()),
            Some("get_weather")
        );
        let assistant_content = mapped.messages[1].content.as_array().unwrap();
        assert_eq!(assistant_content[0]["type"], "tool_use");
        assert_eq!(assistant_content[0]["id"], "toolu_1");
        assert_eq!(assistant_content[0]["name"], "get_weather");
        assert_eq!(assistant_content[0]["input"]["city"], "beijing");

        let tool_result_content = mapped.messages[2].content.as_array().unwrap();
        assert_eq!(tool_result_content[0]["type"], "tool_result");
        assert_eq!(tool_result_content[0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn test_anthropic_request_multi_tools_and_auto_choice() {
        let protocol = AnthropicProtocol::new("test-key");
        let request = ChatRequest::new("claude-3-5-sonnet")
            .with_tools(vec![
                Tool::function(
                    "get_weather",
                    Some("Get weather".to_string()),
                    serde_json::json!({"type":"object"}),
                ),
                Tool::function(
                    "get_time",
                    Some("Get time".to_string()),
                    serde_json::json!({"type":"object"}),
                ),
            ])
            .with_tool_choice(ToolChoice::auto())
            .add_message(Message::user("hi"));

        let mapped = protocol.build_request(&request).unwrap();
        assert_eq!(mapped.tools.as_ref().map(|v| v.len()), Some(2));
        assert_eq!(
            mapped.tool_choice.as_ref().map(|c| c.choice_type.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn test_anthropic_request_required_tool_choice_serialization() {
        let protocol = AnthropicProtocol::new("test-key");
        let request = ChatRequest::new("claude-3-5-sonnet")
            .with_tools(vec![Tool::function(
                "get_weather",
                Some("Get weather".to_string()),
                serde_json::json!({"type":"object"}),
            )])
            .with_tool_choice(ToolChoice::required())
            .add_message(Message::user("hi"));

        let mapped = protocol.build_request(&request).unwrap();
        assert_eq!(mapped.tools.as_ref().map(|v| v.len()), Some(1));
        assert_eq!(
            mapped.tool_choice.as_ref().map(|c| c.choice_type.as_str()),
            Some("any")
        );
        assert_eq!(
            mapped.tool_choice.as_ref().and_then(|c| c.name.as_deref()),
            None
        );
    }

    #[test]
    fn test_anthropic_request_none_tool_choice_serialization() {
        let protocol = AnthropicProtocol::new("test-key");
        let request = ChatRequest::new("claude-3-5-sonnet")
            .with_tools(vec![Tool::function(
                "get_weather",
                Some("Get weather".to_string()),
                serde_json::json!({"type":"object"}),
            )])
            .with_tool_choice(ToolChoice::none())
            .add_message(Message::user("hi"));

        let mapped = protocol.build_request(&request).unwrap();
        assert_eq!(
            mapped.tool_choice.as_ref().map(|c| c.choice_type.as_str()),
            Some("none")
        );
    }

    #[test]
    fn test_anthropic_request_serializes_native_tool_payload_without_loss() {
        let protocol = AnthropicProtocol::new("test-key");
        let request = ChatRequest::new("claude-opus-4-6")
            .with_anthropic_tools(vec![AnthropicToolDefinition {
                tool_type: Some("custom".to_string()),
                name: Some("write_file".to_string()),
                description: Some("Write a file to disk".to_string()),
                input_schema: Some(serde_json::json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                })),
                custom_input_schema: Some(serde_json::json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "mode": { "enum": ["overwrite", "append"] }
                    }
                })),
                input_examples: Some(serde_json::json!([
                    {"path": "/tmp/demo.txt", "mode": "overwrite"}
                ])),
                strict: Some(true),
                allowed_callers: Some(serde_json::json!(["claude_code"])),
                defer_loading: Some(true),
                cache_control: Some(serde_json::json!({"type": "ephemeral"})),
                eager_input_streaming: Some(true),
                extra: std::collections::HashMap::from([(
                    "mcp_toolset".to_string(),
                    serde_json::json!({"server": "filesystem"}),
                )]),
            }])
            .with_anthropic_tool_choice(AnthropicToolChoice::tool("write_file"))
            .add_message(Message::user("write a file"));

        let mapped = protocol.build_request(&request).unwrap();
        let payload = serde_json::to_value(&mapped).unwrap();
        let tool = &payload["tools"][0];

        assert_eq!(tool["type"], "custom");
        assert_eq!(tool["name"], "write_file");
        assert_eq!(tool["custom_input_schema"]["$schema"], "https://json-schema.org/draft/2020-12/schema");
        assert_eq!(tool["strict"], true);
        assert_eq!(tool["allowed_callers"][0], "claude_code");
        assert_eq!(tool["defer_loading"], true);
        assert_eq!(tool["eager_input_streaming"], true);
        assert_eq!(tool["mcp_toolset"]["server"], "filesystem");
        assert_eq!(payload["tool_choice"]["type"], "tool");
        assert_eq!(payload["tool_choice"]["name"], "write_file");
    }

    #[test]
    fn test_anthropic_parse_non_streaming_tool_use_to_tool_calls() {
        let response_json = r#"
        {
            "id": "msg_125",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "get_weather",
                    "input": {"city":"beijing"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20
            }
        }
        "#;

        let protocol = AnthropicProtocol::new("test-key");
        let result = protocol.parse_response(response_json).unwrap();
        let tool_calls = result.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_1");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(tool_calls[0].function.arguments, r#"{"city":"beijing"}"#);
        assert_eq!(
            result.choices[0].finish_reason.as_deref(),
            Some("tool_calls")
        );
    }

    #[test]
    fn test_anthropic_parse_streaming_tool_use_deltas() {
        let start_event = serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "get_weather",
                "input": {}
            }
        });
        let start_chunk = crate::protocols::common::streamers::interpret_anthropic_event(
            &start_event,
            "msg_stream",
        )
        .unwrap()
        .unwrap();
        let start_tool_calls = start_chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(start_tool_calls[0].id, "toolu_1");
        assert_eq!(start_tool_calls[0].function.name, "get_weather");
        assert_eq!(start_tool_calls[0].index, Some(0));

        let delta_event = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"city\":\"bei"
            }
        });
        let delta_chunk = crate::protocols::common::streamers::interpret_anthropic_event(
            &delta_event,
            "msg_stream",
        )
        .unwrap()
        .unwrap();
        let delta_tool_calls = delta_chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(delta_tool_calls[0].index, Some(0));
        assert_eq!(delta_tool_calls[0].function.arguments, "{\"city\":\"bei");
    }
}
