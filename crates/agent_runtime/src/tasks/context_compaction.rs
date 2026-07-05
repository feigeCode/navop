use crate::error::RuntimeError;
use crate::history::{HistoryItem, RuntimeHistory};
use crate::ids::TurnId;
use crate::model::ModelRequest;
use crate::runtime::RuntimeEvent;
use crate::runtime::{RuntimeServices, Session};
use llm_connector::types::Message;
use tokio_util::sync::CancellationToken;

const DEFAULT_TRIGGER_CHARS: usize = 48_000;
const DEFAULT_KEEP_LAST_ITEMS: usize = 32;
const DEFAULT_MAX_SUMMARY_TOKENS: u32 = 1200;
const FALLBACK_SUMMARY_MAX_CHARS: usize = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContextCompactionPolicy {
    pub trigger_chars: usize,
    pub keep_last_items: usize,
    pub max_summary_tokens: u32,
}

impl Default for ContextCompactionPolicy {
    fn default() -> Self {
        Self {
            trigger_chars: DEFAULT_TRIGGER_CHARS,
            keep_last_items: DEFAULT_KEEP_LAST_ITEMS,
            max_summary_tokens: DEFAULT_MAX_SUMMARY_TOKENS,
        }
    }
}

pub(crate) async fn compact_session_context_if_needed(
    session: &Session,
    turn_id: &TurnId,
    services: &RuntimeServices,
    policy: ContextCompactionPolicy,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeError> {
    if cancellation.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    let history = session.history_snapshot();
    if estimated_history_chars(&history) < policy.trigger_chars {
        return Ok(false);
    }
    let Some(prefix) = history.compaction_prefix(policy.keep_last_items) else {
        return Ok(false);
    };
    emit_compaction_status(session, turn_id, "正在压缩上下文...", false);
    let summary = summarize_prefix(prefix, services, policy, cancellation).await?;
    if cancellation.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    let compacted = session.compact_history(summary, policy.keep_last_items);
    if compacted {
        emit_compaction_status(session, turn_id, "上下文压缩完成", true);
    }
    Ok(compacted)
}

fn emit_compaction_status(session: &Session, turn_id: &TurnId, title: &str, is_done: bool) {
    session.emit(RuntimeEvent::Status {
        session_id: session.id().clone(),
        turn_id: turn_id.clone(),
        title: title.to_string(),
        is_done,
    });
}

async fn summarize_prefix(
    prefix: Vec<HistoryItem>,
    services: &RuntimeServices,
    policy: ContextCompactionPolicy,
    cancellation: &CancellationToken,
) -> Result<String, RuntimeError> {
    if cancellation.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    let fallback_history = RuntimeHistory::from_items(prefix.clone());
    let request =
        ModelRequest::new(compaction_messages(prefix)).with_max_tokens(policy.max_summary_tokens);
    let response = services.model.complete(request).await?;
    if let Some(summary) = response
        .text
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    {
        return Ok(summary);
    }

    tracing::warn!("上下文压缩模型未生成摘要，使用本地截断摘要继续任务");
    Ok(fallback_summary(&fallback_history))
}

fn compaction_messages(prefix: Vec<HistoryItem>) -> Vec<Message> {
    let mut messages = vec![Message::system(compaction_system_prompt())];
    let transcript = compaction_transcript(&RuntimeHistory::from_items(prefix));
    if !transcript.is_empty() {
        messages.push(Message::user(transcript));
    }
    messages
}

fn compaction_transcript(history: &RuntimeHistory) -> String {
    history
        .items()
        .iter()
        .map(compaction_item_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn fallback_summary(history: &RuntimeHistory) -> String {
    let transcript = compaction_transcript(history);
    let transcript = truncate_chars(&transcript, FALLBACK_SUMMARY_MAX_CHARS);
    format!(
        "模型未返回上下文压缩摘要。以下为本地保留的历史转录截断摘要，用于继续当前任务：\n{transcript}"
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str("\n...[已截断]");
            return output;
        }
        output.push(ch);
    }
    output
}

fn compaction_item_text(item: &HistoryItem) -> String {
    match item {
        HistoryItem::User { text, images } => {
            let suffix = if images.is_empty() {
                String::new()
            } else {
                format!("\n[图片数量: {}]", images.len())
            };
            format!("[用户]\n{text}{suffix}")
        }
        HistoryItem::Assistant(text) => format!("[助手]\n{text}"),
        HistoryItem::AssistantWithReasoning { text, reasoning } => {
            format!("[助手]\n{text}\n[reasoning]\n{reasoning}")
        }
        HistoryItem::System(text) => format!("[系统]\n{text}"),
        HistoryItem::ContextSummary {
            text,
            original_items,
        } => format!("[上下文压缩摘要: 原始条目 {original_items}]\n{text}"),
        HistoryItem::ToolCall(call) => format!(
            "[工具调用]\nid: {}\nname: {}\narguments: {}",
            call.call_id, call.tool_name, call.arguments
        ),
        HistoryItem::Observation(obs) => format!("[工具结果]\n{}", obs.model_text(4096)),
    }
}

fn compaction_system_prompt() -> String {
    [
        "你正在执行 Codex 风格的上下文压缩。",
        "请把此前对话压缩为一份可继续执行任务的中文摘要。",
        "必须保留: 用户目标、关键约束、已完成操作、已修改文件、工具结果、当前风险、未完成事项。",
        "不要解决新问题，不要编造未发生的事实。",
    ]
    .join("\n")
}

fn estimated_history_chars(history: &RuntimeHistory) -> usize {
    history.items().iter().map(estimated_item_chars).sum()
}

fn estimated_item_chars(item: &HistoryItem) -> usize {
    match item {
        HistoryItem::User { text, images } => text.len() + images.len() * 1024,
        HistoryItem::Assistant(text) | HistoryItem::System(text) => text.len(),
        HistoryItem::AssistantWithReasoning { text, reasoning } => text.len() + reasoning.len(),
        HistoryItem::ContextSummary { text, .. } => text.len(),
        HistoryItem::ToolCall(call) => {
            call.tool_name.to_string().len() + call.arguments.to_string().len()
        }
        HistoryItem::Observation(obs) => obs.model_text(4096).len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryItem;
    use crate::ids::ToolCallId;
    use crate::model::{MockModelClient, ModelResponse};
    use crate::resource::ResourceContext;
    use crate::runtime::{Runtime, RuntimeServices};
    use crate::tools::{
        ObservationData, ToolCall, ToolName, ToolObservation, ToolRegistry, ToolRouter,
    };
    use llm_connector::types::Role;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn compacts_session_history_when_estimated_context_exceeds_threshold() {
        let model = Arc::new(MockModelClient::new([ModelResponse::text(
            "摘要: 用户要部署 Java 项目，数据库连接已创建。",
        )]));
        let runtime = Runtime::new(RuntimeServices::new(
            model.clone(),
            Arc::new(ToolRouter::new(ToolRegistry::new())),
        ));
        let session = runtime.create_session(ResourceContext::new());
        session.record_user_input("旧上下文 ".repeat(32));
        session.record_user_input("最近要继续部署");

        let compacted = compact_session_context_if_needed(
            &session,
            &TurnId::new(),
            runtime.services(),
            ContextCompactionPolicy {
                trigger_chars: 16,
                keep_last_items: 1,
                max_summary_tokens: 256,
            },
            &CancellationToken::new(),
        )
        .await
        .expect("context compaction should run");

        assert!(compacted);
        let history = session.history_snapshot();
        assert!(matches!(
            history.items()[0],
            HistoryItem::ContextSummary { .. }
        ));
        assert!(matches!(history.items()[1], HistoryItem::User { .. }));
        let request = model.received_requests().remove(0);
        assert_eq!(Role::System, request.messages[0].role);
        assert!(request.messages[0].content_as_text().contains("上下文压缩"));
        assert!(request.messages[1].content_as_text().contains("旧上下文"));
    }

    #[tokio::test]
    async fn empty_compaction_response_uses_local_fallback_summary() {
        let model = Arc::new(MockModelClient::new([ModelResponse::default()]));
        let runtime = Runtime::new(RuntimeServices::new(
            model,
            Arc::new(ToolRouter::new(ToolRegistry::new())),
        ));
        let session = runtime.create_session(ResourceContext::new());
        session.record_user_input("旧上下文 ".repeat(32));
        session.record_user_input("继续处理");

        let compacted = compact_session_context_if_needed(
            &session,
            &TurnId::new(),
            runtime.services(),
            ContextCompactionPolicy {
                trigger_chars: 16,
                keep_last_items: 1,
                max_summary_tokens: 256,
            },
            &CancellationToken::new(),
        )
        .await
        .expect("empty summary should not fail the turn");

        assert!(compacted);
        match &session.history_snapshot().items()[0] {
            HistoryItem::ContextSummary { text, .. } => {
                assert!(text.contains("模型未返回上下文压缩摘要"));
                assert!(text.contains("旧上下文"));
            }
            other => panic!("expected fallback context summary, got {other:?}"),
        }
    }

    #[test]
    fn compaction_messages_render_tool_history_as_plain_text() {
        let call_a = tool_call("call_a", "echo", serde_json::json!({"text": "a"}));
        let call_b = tool_call("call_b", "echo", serde_json::json!({"text": "b"}));
        let observation = ToolObservation::success(
            ToolCallId::from_string("call_a"),
            ToolName::new("echo"),
            "echo: a",
            ObservationData::Text("echo: a".into()),
        );

        let messages = compaction_messages(vec![
            HistoryItem::User {
                text: "调用两个工具".into(),
                images: Vec::new(),
            },
            HistoryItem::ToolCall(call_a),
            HistoryItem::ToolCall(call_b),
            HistoryItem::Observation(observation),
        ]);

        assert_eq!(2, messages.len());
        assert_eq!(Role::User, messages[1].role);
        assert!(messages.iter().all(|message| message.tool_calls.is_none()));
        assert!(
            messages
                .iter()
                .all(|message| message.tool_call_id.is_none())
        );
        let transcript = messages[1].content_as_text();
        assert!(transcript.contains("[工具调用]"));
        assert!(transcript.contains("call_a"));
        assert!(transcript.contains("call_b"));
        assert!(transcript.contains("[工具结果]"));
        assert!(transcript.contains("echo: a"));
    }

    fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: ToolCallId::from_string(id),
            tool_name: ToolName::new(name),
            arguments,
            resource_id: None,
        }
    }
}
