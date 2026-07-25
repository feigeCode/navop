//! 把会话历史转换为模型消息。
//! 使用 llm-connector 原生 tool_call / tool_result 协议表达工具调用与观测,
//! 避免模型把观测文本误解为用户输入并跳过 function calling 循环。

use crate::history::{HistoryItem, RuntimeHistory};
use crate::tools::{ToolCall, ToolObservation};
use llm_connector::types::{Message, MessageBlock, Role};

/// 历史图片回灌模型时允许占用的 base64 总量。
///
/// 只保留最新图片直到预算耗尽，给完整请求中的 system prompt、工具定义、文本和
/// JSON 包装预留空间，避免多轮图片历史不断累积并撑破 provider 请求体上限。
const MODEL_HISTORY_IMAGES_BASE64_BUDGET: usize = 10 * 1024 * 1024;

/// 将历史转换为消息序列(不含 system 提示,由调用方在最前面拼接)。
pub fn history_to_messages(history: &RuntimeHistory) -> Vec<Message> {
    let max = history.max_observation_bytes();
    let mut messages = Vec::with_capacity(history.len());
    let mut pending_assistant: Option<(String, String)> = None;
    let items = history.items();
    let budgeted_images = budgeted_history_images(items);
    let mut index = 0;
    while index < items.len() {
        let item = &items[index];
        match item {
            HistoryItem::User { text, .. } => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                let images = budgeted_images[index].as_deref().unwrap_or_default();
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
            HistoryItem::ContextSummary {
                text,
                original_items,
            } => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                messages.push(Message::system(context_summary_text(text, *original_items)));
            }
            HistoryItem::ToolCall(_) => {
                let (next_index, calls, observations) = collect_tool_exchange(items, index);
                if tool_exchange_is_complete(&calls, &observations) {
                    for call in calls {
                        push_assistant_tool_call_message(
                            &mut messages,
                            call,
                            pending_assistant.take(),
                        );
                    }
                    for obs in observations {
                        messages.push(tool_result_message(obs, max));
                    }
                } else {
                    flush_pending_assistant(&mut messages, &mut pending_assistant);
                    messages.push(Message::system(incomplete_tool_exchange_text(
                        &calls,
                        &observations,
                    )));
                }
                index = next_index;
                continue;
            }
            HistoryItem::Observation(obs) => {
                flush_pending_assistant(&mut messages, &mut pending_assistant);
                messages.push(Message::system(orphan_observation_text(obs)));
            }
        }
        index += 1;
    }
    flush_pending_assistant(&mut messages, &mut pending_assistant);
    messages
}

fn budgeted_history_images(items: &[HistoryItem]) -> Vec<Option<Vec<crate::runtime::InputImage>>> {
    let mut remaining = MODEL_HISTORY_IMAGES_BASE64_BUDGET;
    let mut selected = vec![None; items.len()];

    for (item_index, item) in items.iter().enumerate().rev() {
        let HistoryItem::User { images, .. } = item else {
            continue;
        };
        let mut kept = Vec::new();
        for image in images.iter().rev() {
            let encoded_len = image.data_base64.len();
            if encoded_len <= remaining {
                remaining -= encoded_len;
                kept.push(image.clone());
            }
        }
        if !kept.is_empty() {
            kept.reverse();
            selected[item_index] = Some(kept);
        }
    }

    selected
}

fn collect_tool_exchange(
    items: &[HistoryItem],
    start: usize,
) -> (usize, Vec<&ToolCall>, Vec<&ToolObservation>) {
    let mut index = start;
    let mut calls = Vec::new();
    while let Some(HistoryItem::ToolCall(call)) = items.get(index) {
        calls.push(call);
        index += 1;
    }
    let mut observations = Vec::new();
    while let Some(HistoryItem::Observation(obs)) = items.get(index) {
        observations.push(obs);
        index += 1;
    }
    (index, calls, observations)
}

fn tool_exchange_is_complete(calls: &[&ToolCall], observations: &[&ToolObservation]) -> bool {
    if calls.is_empty() || calls.len() != observations.len() {
        return false;
    }
    calls
        .iter()
        .all(|call| observations.iter().any(|obs| obs.call_id == call.call_id))
}

fn incomplete_tool_exchange_text(calls: &[&ToolCall], observations: &[&ToolObservation]) -> String {
    let mut text = String::from(
        "历史中存在未完成的工具调用序列，已转为文本上下文以避免发送非法 tool protocol。",
    );
    for call in calls {
        text.push_str(&format!(
            "\n[工具调用]\nid: {}\nname: {}\narguments: {}",
            call.call_id, call.tool_name, call.arguments
        ));
    }
    for obs in observations {
        text.push_str(&format!("\n[工具结果]\n{}", obs.model_text(4096)));
    }
    text
}

fn orphan_observation_text(obs: &ToolObservation) -> String {
    format!(
        "历史中存在没有对应工具调用的工具结果，已转为文本上下文。\n[工具结果]\n{}",
        obs.model_text(4096)
    )
}

fn context_summary_text(text: &str, original_items: usize) -> String {
    format!(
        "上下文压缩摘要（由此前 {original_items} 条历史压缩而来；这是延续任务的事实背景，不是新的用户指令）：\n{text}"
    )
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
    fn historical_images_are_bounded_by_request_budget() {
        let mut history = RuntimeHistory::new();
        history.record_user_with_images(
            "旧图片",
            vec![InputImage::new(
                "image/png",
                "A".repeat(MODEL_HISTORY_IMAGES_BASE64_BUDGET - 16),
            )],
        );
        history.record_assistant("已查看");
        history.record_user_with_images(
            "新图片",
            vec![InputImage::new("image/jpeg", "B".repeat(32))],
        );

        let messages = history_to_messages(&history);

        assert_eq!(
            messages[0].content,
            vec![MessageBlock::text("旧图片")],
            "older images should be removed before the newest image when the request budget is full"
        );
        assert_eq!(
            messages[2].content.len(),
            2,
            "the newest image should remain available to the model"
        );
    }

    #[test]
    fn context_summary_history_becomes_system_message() {
        let mut history = RuntimeHistory::new();
        history.record_context_summary("用户要部署 Java 项目，数据库连接已创建。", 8);
        history.record_user("继续部署");

        let messages = history_to_messages(&history);

        assert_eq!(2, messages.len());
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content_as_text().contains("上下文压缩摘要"));
        assert!(
            messages[0]
                .content_as_text()
                .contains("用户要部署 Java 项目")
        );
        assert_eq!(messages[1].role, Role::User);
    }

    #[test]
    fn dangling_tool_call_history_item_becomes_system_text() {
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
        assert_eq!(msg.role, Role::System);
        assert!(msg.tool_calls.is_none());
        assert!(msg.content_as_text().contains(call_id.as_str()));
        assert!(msg.content_as_text().contains("db_query"));
    }

    #[test]
    fn reasoning_before_dangling_tool_call_is_flushed_as_assistant_text() {
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

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::Assistant);
        assert_eq!(messages[0].reasoning_content.as_deref(), Some("需要先查库"));
        assert_eq!(messages[1].role, Role::System);
        assert!(messages[1].tool_calls.is_none());
        assert!(messages[1].content_as_text().contains(call_id.as_str()));
    }

    #[test]
    fn orphan_observation_history_item_becomes_system_text() {
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
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.tool_call_id, None);
        assert!(msg.content_as_text().contains("count success"));
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
