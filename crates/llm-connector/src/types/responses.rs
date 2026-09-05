use crate::error::LlmConnectorError;
use crate::types::{ChatRequest, ChatResponse, Message, Role, Tool, ToolChoice};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "streaming")]
use futures_util::Stream;
#[cfg(feature = "streaming")]
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    #[serde(skip_serializing)]
    pub base_url: Option<String>,
    #[serde(skip_serializing)]
    pub extra_headers: Option<HashMap<String, String>>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ResponsesOutputContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesResponse {
    pub id: String,
    pub object: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<ResponsesOutputItem>>,
    #[serde(default)]
    pub output_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponsesUsage>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ResponsesResponse {
    pub fn populate_output_text(&mut self) {
        if !self.output_text.is_empty() {
            return;
        }

        let mut merged = Vec::new();
        if let Some(output) = &self.output {
            for item in output {
                if let Some(content) = &item.content {
                    for block in content {
                        if let Some(text) = &block.text
                            && !text.is_empty()
                        {
                            merged.push(text.clone());
                        }
                    }
                }
            }
        }

        if !merged.is_empty() {
            self.output_text = merged.join("");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: HashMap<String, serde_json::Value>,
}

impl ResponsesStreamEvent {
    pub fn response_created(response_id: impl Into<String>, model: Option<String>) -> Self {
        let mut data = HashMap::new();
        data.insert(
            "response".to_string(),
            serde_json::json!({
                "id": response_id.into(),
                "object": "response",
                "model": model,
                "status": "in_progress",
            }),
        );
        Self {
            event_type: "response.created".to_string(),
            data,
        }
    }

    pub fn output_text_delta(response_id: impl Into<String>, delta: impl Into<String>) -> Self {
        let mut data = HashMap::new();
        data.insert(
            "response_id".to_string(),
            serde_json::json!(response_id.into()),
        );
        data.insert("delta".to_string(), serde_json::json!(delta.into()));
        Self {
            event_type: "response.output_text.delta".to_string(),
            data,
        }
    }

    pub fn response_completed(
        response_id: impl Into<String>,
        usage: Option<ResponsesUsage>,
        model: Option<String>,
    ) -> Self {
        let mut data = HashMap::new();
        data.insert(
            "response".to_string(),
            serde_json::json!({
                "id": response_id.into(),
                "object": "response",
                "model": model,
                "status": "completed",
                "usage": usage,
            }),
        );
        Self {
            event_type: "response.completed".to_string(),
            data,
        }
    }
}

#[cfg(feature = "streaming")]
pub type ResponsesStream = Pin<
    Box<dyn Stream<Item = Result<ResponsesStreamEvent, crate::error::LlmConnectorError>> + Send>,
>;

pub fn responses_request_to_chat_request(
    request: &ResponsesRequest,
) -> Result<ChatRequest, LlmConnectorError> {
    let mut messages = Vec::new();

    if let Some(instructions) = &request.instructions
        && !instructions.trim().is_empty()
    {
        messages.push(Message::system(instructions.clone()));
    }

    if let Some(input) = &request.input {
        append_input_messages(&mut messages, input)?;
    }

    let tools = if let Some(raw_tools) = request.tools.as_ref() {
        Some(
            serde_json::from_value::<Vec<Tool>>(raw_tools.clone()).map_err(|e| {
                LlmConnectorError::InvalidRequest(format!("Failed to map responses.tools: {}", e))
            })?,
        )
    } else {
        None
    };

    let tool_choice = if let Some(raw_choice) = request.tool_choice.as_ref() {
        Some(
            serde_json::from_value::<ToolChoice>(raw_choice.clone()).map_err(|e| {
                LlmConnectorError::InvalidRequest(format!(
                    "Failed to map responses.tool_choice: {}",
                    e
                ))
            })?,
        )
    } else {
        None
    };

    Ok(ChatRequest {
        model: request.model.clone(),
        messages,
        temperature: request.temperature,
        top_p: request.top_p,
        max_tokens: request.max_output_tokens,
        stream: request.stream,
        tools,
        tool_choice,
        api_key: request.api_key.clone(),
        base_url: request.base_url.clone(),
        extra_headers: request.extra_headers.clone(),
        ..Default::default()
    })
}

pub fn chat_response_to_responses_response(chat: &ChatResponse) -> ResponsesResponse {
    let text = if !chat.content.is_empty() {
        chat.content.clone()
    } else {
        chat.choices
            .first()
            .map(|choice| choice.message.content_as_text())
            .unwrap_or_default()
    };

    let usage = chat.usage.as_ref().map(|u| ResponsesUsage {
        input_tokens: Some(u.prompt_tokens),
        output_tokens: Some(u.completion_tokens),
        total_tokens: Some(u.total_tokens),
        extra: HashMap::new(),
    });

    ResponsesResponse {
        id: chat.id.clone(),
        object: "response".to_string(),
        created_at: chat.created,
        model: Some(chat.model.clone()),
        status: Some("completed".to_string()),
        output: Some(vec![ResponsesOutputItem {
            item_type: "message".to_string(),
            id: Some(format!("msg_{}", chat.id)),
            role: Some("assistant".to_string()),
            content: Some(vec![ResponsesOutputContent {
                content_type: "output_text".to_string(),
                text: Some(text.clone()),
                extra: HashMap::new(),
            }]),
            name: None,
            arguments: None,
            extra: HashMap::new(),
        }]),
        output_text: text,
        usage,
        extra: HashMap::new(),
    }
}

fn append_input_messages(
    messages: &mut Vec<Message>,
    input: &serde_json::Value,
) -> Result<(), LlmConnectorError> {
    match input {
        serde_json::Value::Null => {}
        serde_json::Value::String(text) => {
            if !text.is_empty() {
                messages.push(Message::user(text.clone()));
            }
        }
        serde_json::Value::Array(items) => {
            if let Ok(parsed) = serde_json::from_value::<Vec<Message>>(input.clone()) {
                messages.extend(parsed);
                return Ok(());
            }

            for item in items {
                if let Some(msg) = map_input_item_to_message(item) {
                    messages.push(msg);
                }
            }
        }
        serde_json::Value::Object(_) => {
            if let Ok(parsed) = serde_json::from_value::<Message>(input.clone()) {
                messages.push(parsed);
                return Ok(());
            }

            if let Some(msg) = map_input_item_to_message(input) {
                messages.push(msg);
            }
        }
        _ => {
            return Err(LlmConnectorError::InvalidRequest(
                "responses.input must be string/object/array".to_string(),
            ));
        }
    }

    Ok(())
}

fn map_input_item_to_message(value: &serde_json::Value) -> Option<Message> {
    let obj = value.as_object()?;

    if let Some(content) = obj.get("content")
        && content.is_string()
    {
        let role = parse_role(obj.get("role"));
        return Some(Message::text(role, content.as_str().unwrap_or_default()));
    }

    if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
        let role = parse_role(obj.get("role"));
        return Some(Message::text(role, text));
    }

    if obj.get("type").and_then(|v| v.as_str()) == Some("input_text")
        && let Some(text) = obj.get("text").and_then(|v| v.as_str())
    {
        return Some(Message::user(text));
    }

    if let Some(content) = obj.get("content").and_then(|v| v.as_array()) {
        let mut joined = Vec::new();
        for part in content {
            if part.get("type").and_then(|v| v.as_str()) == Some("input_text")
                && let Some(text) = part.get("text").and_then(|v| v.as_str())
            {
                joined.push(text.to_string());
            }
        }

        if !joined.is_empty() {
            let role = parse_role(obj.get("role"));
            return Some(Message::text(role, joined.join("\n")));
        }
    }

    None
}

fn parse_role(raw: Option<&serde_json::Value>) -> Role {
    match raw.and_then(|v| v.as_str()).unwrap_or("user") {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_string_input_to_chat_message() {
        let req = ResponsesRequest {
            model: "gpt-4.1".to_string(),
            input: Some(serde_json::json!("hello")),
            ..Default::default()
        };

        let chat = responses_request_to_chat_request(&req).expect("map should succeed");
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, Role::User);
        assert_eq!(chat.messages[0].content_as_text(), "hello");
    }

    #[test]
    fn test_map_instructions_to_system_message() {
        let req = ResponsesRequest {
            model: "gpt-4.1".to_string(),
            instructions: Some("be concise".to_string()),
            input: Some(serde_json::json!("hello")),
            ..Default::default()
        };

        let chat = responses_request_to_chat_request(&req).expect("map should succeed");
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, Role::System);
        assert_eq!(chat.messages[0].content_as_text(), "be concise");
    }

    #[test]
    fn test_chat_response_to_responses_response() {
        let chat = ChatResponse {
            id: "chatcmpl_1".to_string(),
            object: "chat.completion".to_string(),
            created: 1,
            model: "gpt-4.1".to_string(),
            content: "ok".to_string(),
            ..Default::default()
        };

        let resp = chat_response_to_responses_response(&chat);
        assert_eq!(resp.object, "response");
        assert_eq!(resp.output_text, "ok");
        assert_eq!(resp.model.as_deref(), Some("gpt-4.1"));
    }
}
