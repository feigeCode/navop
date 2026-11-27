//! `/responses` 与 `/chat/completions` 之间的互转。
//!
//! 供应商设置里选择「Responses」接口后，上层（bridge / agent）仍然只用
//! `ChatRequest` / `ChatResponse` 与模型交互；由这里把 chat 请求改写成 Responses
//! 形态，并把 Responses 的响应与流式事件还原成 chat 结构，上层无需分叉。

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::error::LlmConnectorError;
use crate::types::{
    ChatRequest, ChatResponse, Choice, Delta, FunctionCall, Message, MessageBlock, ResponseFormat,
    ResponsesRequest, ResponsesResponse, ResponsesStreamEvent, ResponsesUsage, Role,
    StreamingChoice, StreamingResponse, Tool, ToolCall, ToolChoice, Usage,
};

/// 把 chat 请求改写成 Responses 请求。
///
/// 语义映射：
/// - system 消息合并成 `instructions`；
/// - user / assistant 文本与图片 → `input` 里的 `message` item；
/// - assistant 的 `tool_calls` → 独立 `function_call` item；
/// - tool 消息 → `function_call_output` item；
/// - `tools` / `tool_choice` 转成 Responses 的扁平结构；
/// - `reasoning_effort` → `reasoning.effort`，`response_format` → `text.format`。
pub fn chat_request_to_responses_request(
    request: &ChatRequest,
) -> Result<ResponsesRequest, LlmConnectorError> {
    let mut instructions = Vec::new();
    let mut input = Vec::new();

    for message in &request.messages {
        map_chat_message(message, &mut instructions, &mut input);
    }

    let instructions = if instructions.is_empty() {
        None
    } else {
        Some(instructions.join("\n"))
    };

    let tools = request
        .tools
        .as_ref()
        .map(|tools| Value::Array(tools.iter().map(map_tool_to_responses).collect::<Vec<_>>()));
    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(map_tool_choice_to_responses);

    let mut extra = HashMap::new();
    if let Some(effort) = request.reasoning_effort {
        extra.insert(
            "reasoning".to_string(),
            json!({ "effort": effort.as_str() }),
        );
    }
    if let Some(format) = request
        .response_format
        .as_ref()
        .and_then(map_response_format_to_responses)
    {
        extra.insert("text".to_string(), json!({ "format": format }));
    }

    Ok(ResponsesRequest {
        model: request.model.clone(),
        input: if input.is_empty() {
            None
        } else {
            Some(Value::Array(input))
        },
        instructions,
        temperature: request.temperature,
        top_p: request.top_p,
        max_output_tokens: request.max_tokens,
        stream: request.stream,
        tools,
        tool_choice,
        previous_response_id: None,
        metadata: None,
        api_key: request.api_key.clone(),
        base_url: request.base_url.clone(),
        extra_headers: request.extra_headers.clone(),
        extra,
    })
}

fn map_chat_message(message: &Message, instructions: &mut Vec<String>, input: &mut Vec<Value>) {
    match message.role {
        Role::System => {
            let text = message.content_as_text();
            if !text.is_empty() {
                instructions.push(text);
            }
        }
        Role::Tool => {
            let call_id = message.tool_call_id.clone().unwrap_or_default();
            input.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.content_as_text(),
            }));
        }
        Role::Assistant
            if message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty()) =>
        {
            let text = message.content_as_text();
            if !text.is_empty() {
                input.push(message_item(Role::Assistant, &text, message));
            }
            for call in message.tool_calls.iter().flatten() {
                input.push(json!({
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.function.name,
                    "arguments": call.function.arguments,
                }));
            }
        }
        role => {
            let text = message.content_as_text();
            input.push(message_item(role, &text, message));
        }
    }
}

fn message_item(role: Role, text: &str, message: &Message) -> Value {
    let original_blocks = message
        .content
        .iter()
        .filter(|block| !block.is_text())
        .map(|block| map_content_block(role, block))
        .collect::<Vec<_>>();

    let content_type = if role == Role::Assistant {
        "output_text"
    } else {
        "input_text"
    };
    let mut content = Vec::new();
    if !text.is_empty() || original_blocks.is_empty() {
        content.push(json!({ "type": content_type, "text": text }));
    }
    content.extend(original_blocks);

    json!({
        "type": "message",
        "role": role_str(role),
        "content": content,
    })
}

/// 非文本块转成 Responses 的 `input_image` 等结构；Responses 不支持的块直接丢弃。
fn map_content_block(role: Role, block: &MessageBlock) -> Value {
    let _ = role;
    match block {
        MessageBlock::Image { source } => match source {
            crate::types::ImageSource::Base64 { media_type, data } => json!({
                "type": "input_image",
                "image_url": format!("data:{media_type};base64,{data}"),
            }),
            crate::types::ImageSource::Url { url } => json!({
                "type": "input_image",
                "image_url": url,
            }),
        },
        MessageBlock::ImageUrl { image_url } => json!({
            "type": "input_image",
            "image_url": image_url.url,
        }),
        _ => Value::Null,
    }
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn map_tool_to_responses(tool: &Tool) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("type".to_string(), json!("function"));
    object.insert("name".to_string(), json!(tool.function.name));
    if let Some(description) = tool.function.description.clone() {
        object.insert("description".to_string(), json!(description));
    }
    object.insert("parameters".to_string(), tool.function.parameters.clone());
    Value::Object(object)
}

fn map_tool_choice_to_responses(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Mode(mode) => json!(mode),
        ToolChoice::Function { function, .. } => json!({
            "type": "function",
            "name": function.name,
        }),
    }
}

fn map_response_format_to_responses(format: &ResponseFormat) -> Option<Value> {
    match format.format_type.as_str() {
        "json_schema" => {
            let spec = format.json_schema.as_ref()?;
            let mut object = serde_json::Map::new();
            object.insert("type".to_string(), json!("json_schema"));
            object.insert("name".to_string(), json!(spec.name));
            if let Some(description) = spec.description.clone() {
                object.insert("description".to_string(), json!(description));
            }
            object.insert("schema".to_string(), spec.schema.clone());
            if let Some(strict) = spec.strict {
                object.insert("strict".to_string(), json!(strict));
            }
            Some(Value::Object(object))
        }
        "json_object" => Some(json!({ "type": "json_object" })),
        _ => None,
    }
}

/// 把 Responses 的非流式响应还原成 chat 响应。
pub fn responses_response_to_chat_response(response: &ResponsesResponse) -> ChatResponse {
    let mut text_parts = Vec::new();
    let mut reasoning_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for item in response.output.iter().flatten() {
        match item.item_type.as_str() {
            "message" => {
                for block in item.content.iter().flatten() {
                    if let Some(text) = block.text.as_deref()
                        && !text.is_empty()
                    {
                        text_parts.push(text.to_string());
                    }
                }
            }
            "reasoning" => collect_reasoning_text(item, &mut reasoning_parts),
            "function_call" => {
                tool_calls.push(function_call_to_tool_call(item, tool_calls.len()));
            }
            _ => {}
        }
    }

    if text_parts.is_empty() && !response.output_text.is_empty() {
        text_parts.push(response.output_text.clone());
    }

    let content = text_parts.join("");
    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join("\n"))
    };
    let finish_reason = if !tool_calls.is_empty() {
        "tool_calls"
    } else if response.status.as_deref() == Some("incomplete") {
        "length"
    } else {
        "stop"
    };

    let blocks = if content.is_empty() {
        Vec::new()
    } else {
        vec![MessageBlock::text(content.clone())]
    };
    let mut message = Message::new(Role::Assistant, blocks);
    message.tool_calls = if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    };
    message.reasoning_content = reasoning.clone();

    ChatResponse {
        id: response.id.clone(),
        object: "chat.completion".to_string(),
        created: response.created_at,
        model: response.model.clone().unwrap_or_default(),
        choices: vec![Choice {
            index: 0,
            message,
            finish_reason: Some(finish_reason.to_string()),
            logprobs: None,
        }],
        content,
        reasoning_content: reasoning,
        usage: response.usage.as_ref().map(responses_usage_to_usage),
        system_fingerprint: None,
    }
}

fn collect_reasoning_text(item: &crate::types::ResponsesOutputItem, parts: &mut Vec<String>) {
    let mut collected = false;
    if let Some(summary) = item.extra.get("summary").and_then(Value::as_array) {
        for summary_part in summary {
            if let Some(text) = summary_part.get("text").and_then(Value::as_str)
                && !text.is_empty()
            {
                parts.push(text.to_string());
                collected = true;
            }
        }
    }
    if collected {
        return;
    }

    for block in item.content.iter().flatten() {
        if let Some(text) = block.text.as_deref()
            && !text.is_empty()
        {
            parts.push(text.to_string());
        }
    }
}

fn function_call_to_tool_call(item: &crate::types::ResponsesOutputItem, index: usize) -> ToolCall {
    let id = item
        .extra
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| item.id.clone())
        .unwrap_or_default();

    ToolCall {
        id,
        call_type: "function".to_string(),
        function: FunctionCall {
            name: item.name.clone().unwrap_or_default(),
            arguments: item.arguments.clone().unwrap_or_default(),
            thought_signature: None,
        },
        index: Some(index),
        thought_signature: None,
    }
}

fn responses_usage_to_usage(usage: &ResponsesUsage) -> Usage {
    Usage {
        prompt_tokens: usage.input_tokens.unwrap_or(0),
        completion_tokens: usage.output_tokens.unwrap_or(0),
        total_tokens: usage.total_tokens.unwrap_or(0),
        ..Default::default()
    }
}

/// 把 Responses 的流式事件逐条映射成 chat 分片。
///
/// 需要记住状态：`response.created` 里的 id / model，以及 `function_call` 的
/// item id → tool call index 的映射（参数增量事件只带 item id）。
#[derive(Debug, Default)]
pub struct ResponsesChatStreamMapper {
    response_id: String,
    model: String,
    created: u64,
    started: bool,
    finished: bool,
    saw_tool_call: bool,
    tool_index_by_item: HashMap<String, usize>,
    next_tool_index: usize,
}

impl ResponsesChatStreamMapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// 映射单个事件；返回 `None` 表示该事件不需要产出分片。
    pub fn map_event(
        &mut self,
        event: &ResponsesStreamEvent,
    ) -> Result<Option<StreamingResponse>, LlmConnectorError> {
        match event.event_type.as_str() {
            "response.created" | "response.in_progress" => {
                self.capture_response(&event.data);
                if self.started {
                    return Ok(None);
                }
                self.started = true;
                Ok(Some(self.chunk(
                    Delta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    String::new(),
                    None,
                    None,
                )))
            }
            "response.output_item.added" => {
                let Some(item) = event.data.get("item") else {
                    return Ok(None);
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return Ok(None);
                }

                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or(item_id.as_str())
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let index = self.next_tool_index;
                self.next_tool_index += 1;
                if !item_id.is_empty() {
                    self.tool_index_by_item.insert(item_id, index);
                }
                self.saw_tool_call = true;

                Ok(Some(self.chunk(
                    Delta {
                        tool_calls: Some(vec![ToolCall {
                            id: call_id,
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name,
                                arguments: String::new(),
                                thought_signature: None,
                            },
                            index: Some(index),
                            thought_signature: None,
                        }]),
                        ..Default::default()
                    },
                    String::new(),
                    None,
                    None,
                )))
            }
            "response.function_call_arguments.delta" => {
                let Some(item_id) = event.data.get("item_id").and_then(Value::as_str) else {
                    return Ok(None);
                };
                let Some(index) = self.tool_index_by_item.get(item_id).copied() else {
                    return Ok(None);
                };
                let delta = event
                    .data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                Ok(Some(self.chunk(
                    Delta {
                        tool_calls: Some(vec![ToolCall {
                            id: String::new(),
                            // 参数分片只携带 arguments：带上 `type` 会让上层
                            // 把它误判成新快照并覆盖掉已捕获的 id/name。
                            call_type: String::new(),
                            function: FunctionCall {
                                name: String::new(),
                                arguments: delta,
                                thought_signature: None,
                            },
                            index: Some(index),
                            thought_signature: None,
                        }]),
                        ..Default::default()
                    },
                    String::new(),
                    None,
                    None,
                )))
            }
            "response.output_text.delta" => {
                let delta = event
                    .data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(self.chunk(
                    Delta {
                        content: Some(delta.clone()),
                        ..Default::default()
                    },
                    delta,
                    None,
                    None,
                )))
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = event
                    .data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(self.chunk(
                    Delta {
                        reasoning_content: Some(delta),
                        ..Default::default()
                    },
                    String::new(),
                    None,
                    None,
                )))
            }
            "response.completed" | "response.incomplete" => {
                self.capture_response(&event.data);
                let finish_reason = if event.event_type == "response.incomplete" {
                    let reason = event
                        .data
                        .get("response")
                        .and_then(|response| response.get("incomplete_details"))
                        .and_then(|details| details.get("reason"))
                        .and_then(Value::as_str);
                    if reason == Some("max_output_tokens") {
                        "length"
                    } else {
                        "stop"
                    }
                } else if self.saw_tool_call {
                    "tool_calls"
                } else {
                    "stop"
                };
                let usage = self.usage_from_data(&event.data);
                self.finished = true;

                Ok(Some(self.chunk(
                    Delta::default(),
                    String::new(),
                    Some(finish_reason.to_string()),
                    usage,
                )))
            }
            "response.failed" => {
                let message = event
                    .data
                    .get("response")
                    .and_then(|response| response.get("error"))
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("responses stream failed")
                    .to_string();
                Err(LlmConnectorError::InvalidRequest(message))
            }
            _ => Ok(None),
        }
    }

    fn capture_response(&mut self, data: &HashMap<String, Value>) {
        let Some(response) = data.get("response") else {
            return;
        };
        if let Some(id) = response.get("id").and_then(Value::as_str) {
            self.response_id = id.to_string();
        }
        if let Some(model) = response.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(created) = response.get("created_at").and_then(Value::as_u64) {
            self.created = created;
        }
    }

    fn usage_from_data(&self, data: &HashMap<String, Value>) -> Option<Usage> {
        let usage = data.get("response")?.get("usage")?.as_object()?;
        Some(Usage {
            prompt_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            completion_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            total_tokens: usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            ..Default::default()
        })
    }

    fn chunk(
        &self,
        delta: Delta,
        content: String,
        finish_reason: Option<String>,
        usage: Option<Usage>,
    ) -> StreamingResponse {
        StreamingResponse {
            id: self.response_id.clone(),
            object: "chat.completion.chunk".to_string(),
            created: self.created,
            model: self.model.clone(),
            choices: vec![StreamingChoice {
                index: 0,
                delta,
                finish_reason,
                logprobs: None,
            }],
            content,
            reasoning_content: None,
            usage,
            system_fingerprint: None,
        }
    }
}

/// 把 Responses 流适配成 chat 流，供上层继续按 chat 语义消费。
#[cfg(feature = "streaming")]
pub fn responses_stream_to_chat_stream(
    stream: crate::types::ResponsesStream,
) -> crate::types::ChatStream {
    use futures_util::StreamExt;

    let mut mapper = ResponsesChatStreamMapper::new();
    Box::pin(stream.filter_map(move |item| {
        let mapped = match item {
            Ok(event) => mapper.map_event(&event),
            Err(error) => Err(error),
        };

        async move {
            match mapped {
                Ok(Some(chunk)) => Some(Ok(chunk)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        }
    }))
}
