//! 把会话历史转换为模型消息。
//! 使用 llm-connector 原生 tool_call / tool_result 协议表达工具调用与观测,
//! 避免模型把观测文本误解为用户输入并跳过 function calling 循环。

use crate::history::{HistoryItem, RuntimeHistory};
use crate::tools::ToolCall;
use llm_connector::types::{Message, MessageBlock, Role};

/// 将历史转换为消息序列(不含 system 提示,由调用方在最前面拼接)。
pub fn history_to_messages(history: &RuntimeHistory) -> Vec<Message> {
    let max = history.max_observation_bytes();
    let mut messages = Vec::with_capacity(history.len());
    let mut pending_assistant: Option<(String, String)> = None;
    for item in history.items() {
        match item {
            HistoryItem::User { text, images } => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                if images.is_empty() {
                    messages.push(Message::user(text.clone()));
                } else {
                    messages.push(user_message_with_images(text, images));
                }
            }
            HistoryItem::Assistant(text) => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                push_assistant_message(&mut messages, text.clone(), String::new());
            }
            HistoryItem::AssistantWithReasoning { text, reasoning } => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                pending_assistant = Some((text.clone(), reasoning.clone()));
            }
            HistoryItem::System(text) => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                messages.push(Message::system(text.clone()));
            }
            HistoryItem::ToolCall(call) => {
                push_assistant_tool_call_message(&mut messages, call, pending_assistant.take());
            }
            HistoryItem::Observation(obs) => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                messages.push(tool_result_message(obs, max));
            }
        }
    }
    flush_pending_assistant(&mut messages, &mut pending_assistant);
    messages
}

fn push_assistant_tool_call_message(
    messages: &mut Vec<Message>,
    call: &ToolCall,
    assistant: Option<(String, String)>,
) {
    if assistant.is_none()
        && let Some(message) = messages.last_mut()
        && message.role == Role::Assistant
        && let Some(tool_calls) = message.tool_calls.as_mut()
    {
        tool_calls.push(llm_tool_call(call));
        return;
    }

    messages.push(assistant_tool_call_message(call, assistant));
}

/// 把 agent_runtime 的工具调用转为原生 assistant tool_calls 消息。
fn assistant_tool_call_message(call: &ToolCall, assistant: Option<(String, String)>) -> Message {
    let (text, reasoning) = assistant.unwrap_or_default();
    Message {
        role: Role::Assistant,
        content: if text.is_empty() {
            Vec::new()
        } else {
            vec![MessageBlock::text(text)]
        },
        tool_calls: Some(vec![llm_tool_call(call)]),
        reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
        ..Default::default()
    }
}

fn llm_tool_call(call: &ToolCall) -> llm_connector::types::ToolCall {
    llm_connector::types::ToolCall {
        id: call.call_id.to_string(),
        call_type: "function".to_string(),
        function: llm_connector::types::FunctionCall {
            name: call.tool_name.to_string(),
            arguments: call.arguments.to_string(),
            thought_signature: None,
        },
        index: None,
        thought_signature: None,
    }
}

fn flush_pending_assistant(messages: &mut Vec<Message>, pending: &mut Option<(String, String)>) {
    if let Some((text, reasoning)) = pending.take() {
        push_assistant_message(messages, text, reasoning);
    }
}

fn push_assistant_message(messages: &mut Vec<Message>, text: String, reasoning: String) {
    if text.is_empty() && reasoning.is_empty() {
        return;
    }
    let mut message = Message::assistant(text);
    if !reasoning.is_empty() {
        message.reasoning_content = Some(reasoning);
    }
    messages.push(message);
}

/// 把 agent_runtime 的观测转为原生 tool result 消息(Role::Tool)。
fn tool_result_message(obs: &crate::tools::ToolObservation, max_bytes: usize) -> Message {
    Message {
        role: Role::Tool,
        content: vec![MessageBlock::text(obs.model_text(max_bytes))],
        tool_call_id: Some(obs.call_id.to_string()),
        ..Default::default()
    }
}

/// 构造含图片的多模态用户消息(文本块 + 各图片的 base64 块)。
fn user_message_with_images(text: &str, images: &[crate::runtime::InputImage]) -> Message {
    let mut blocks: Vec<MessageBlock> = Vec::with_capacity(images.len() + 1);
    if !text.is_empty() {
        blocks.push(MessageBlock::text(text));
    }
    for image in images {
        blocks.push(MessageBlock::image_base64(
            image.mime.clone(),
            image.data_base64.clone(),
        ));
    }
    Message::new(Role::User, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::RuntimeHistory;
    use crate::ids::ToolCallId;
    use crate::runtime::InputImage;
    use crate::tools::{ToolCall, ToolName, ToolObservation};

    #[test]
    fn plain_user_history_becomes_text_message() {
        let mut history = RuntimeHistory::new();
        history.record_user("你好");
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content.len(), 1);
        assert!(messages[0].content[0].is_text());
    }

    #[test]
    fn assistant_reasoning_history_sends_only_visible_text_to_model() {
        let mut history = RuntimeHistory::new();
        history.record_assistant_with_reasoning("最终回答", "内部推理");
        let messages = history_to_messages(&history);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::Assistant);
        assert_eq!(messages[0].content_as_text(), "最终回答");
        assert_eq!(messages[0].reasoning_content.as_deref(), Some("内部推理"));
    }

    #[test]
    fn reasoning_only_history_is_sent_as_protocol_metadata() {
        let mut history = RuntimeHistory::new();
        history.record_assistant_with_reasoning("", "内部推理");
        let messages = history_to_messages(&history);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::Assistant);
        assert_eq!(messages[0].content_as_text(), "");
        assert_eq!(messages[0].reasoning_content.as_deref(), Some("内部推理"));
    }

    #[test]
    fn user_history_with_images_becomes_multimodal_message() {
        let mut history = RuntimeHistory::new();
        history.record_user_with_images("看这张图", vec![InputImage::new("image/png", "AAAABBBB")]);
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.role, Role::User);
        // 文本块 + 图片块。
        assert_eq!(msg.content.len(), 2);
        assert!(msg.content[0].is_text());
        assert!(!msg.content[1].is_text(), "第二块应为图片块");
    }

    #[test]
    fn tool_call_history_item_becomes_assistant_tool_call() {
        let call_id = ToolCallId::from_string("call_test_1");
        let tool_call = ToolCall {
            call_id: call_id.clone(),
            tool_name: ToolName::new("db_query"),
            arguments: serde_json::json!({
                "target": "5",
                "database": "ai_app",
                "sql": "SELECT COUNT(*) AS total FROM information_schema.TABLES WHERE TABLE_SCHEMA = 'ai_app'"
            }),
            resource_id: None,
        };

        let mut history = RuntimeHistory::new();
        history.record_tool_call(tool_call);
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.role, Role::Assistant);
        assert!(msg.tool_calls.is_some());
        let tool_calls = msg
            .tool_calls
            .as_ref()
            .expect("assistant tool message should carry tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, call_id.to_string());
        assert_eq!(tool_calls[0].function.name, "db_query");
    }

    #[test]
    fn reasoning_before_tool_call_is_attached_to_tool_call_message() {
        let call_id = ToolCallId::from_string("call_reasoning_tool");
        let tool_call = ToolCall {
            call_id: call_id.clone(),
            tool_name: ToolName::new("db_query"),
            arguments: serde_json::json!({"sql": "select 1"}),
            resource_id: None,
        };

        let mut history = RuntimeHistory::new();
        history.record_assistant_with_reasoning("", "需要先查库");
        history.record_tool_call(tool_call);
        let messages = history_to_messages(&history);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::Assistant);
        assert_eq!(messages[0].reasoning_content.as_deref(), Some("需要先查库"));
        let tool_calls = messages[0].tool_calls.as_ref().expect("tool call");
        assert_eq!(tool_calls[0].id, call_id.to_string());
    }

    #[test]
    fn observation_history_item_becomes_tool_role_with_call_id() {
        let call_id = ToolCallId::from_string("call_test_2");
        let observation = ToolObservation::success(
            call_id,
            ToolName::new("db_query"),
            "count success",
            crate::tools::ObservationData::Text("1".into()),
        );
        let mut history = RuntimeHistory::new();
        history.record_observation(observation);
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id, Some("call_test_2".to_string()));
        assert_eq!(msg.content.len(), 1);
        assert!(msg.content[0].is_text());
    }

    #[test]
    fn tool_call_and_observation_generate_distinct_roles() {
        let call_id = ToolCallId::from_string("call_test_3");
        let tool_call = ToolCall {
            call_id: call_id.clone(),
            tool_name: ToolName::new("db_query"),
            arguments: serde_json::json!({
                "target": "5",
                "database": "ai_app",
                "sql": "SELECT COUNT(*) AS total FROM information_schema.TABLES WHERE TABLE_SCHEMA = 'ai_app'"
            }),
            resource_id: None,
        };
        let observation = ToolObservation::success(
            call_id,
            ToolName::new("db_query"),
            "count success",
            crate::tools::ObservationData::Text("1".into()),
        );

        let mut history = RuntimeHistory::new();
        history.record_tool_call(tool_call);
        history.record_observation(observation);
        let messages = history_to_messages(&history);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::Assistant);
        assert!(messages[0].tool_calls.is_some());
        assert_eq!(messages[1].role, Role::Tool);
        assert_eq!(messages[1].tool_call_id, Some("call_test_3".to_string()));
    }
}
