use crate::error::RuntimeError;
use crate::history::{HistoryItem, RuntimeHistory};
use crate::model::ModelRequest;
use crate::planner::history_to_messages;
use crate::runtime::{RuntimeServices, Session};
use llm_connector::types::Message;
use tokio_util::sync::CancellationToken;

const DEFAULT_TRIGGER_CHARS: usize = 48_000;
const DEFAULT_KEEP_LAST_ITEMS: usize = 32;
const DEFAULT_MAX_SUMMARY_TOKENS: u32 = 1200;

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
    let summary = summarize_prefix(prefix, services, policy, cancellation).await?;
    if cancellation.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    Ok(session.compact_history(summary, policy.keep_last_items))
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
    let request =
        ModelRequest::new(compaction_messages(prefix)).with_max_tokens(policy.max_summary_tokens);
    let response = services.model.complete(request).await?;
    response
        .text
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| RuntimeError::model("上下文压缩没有生成摘要"))
}

fn compaction_messages(prefix: Vec<HistoryItem>) -> Vec<Message> {
    let mut messages = vec![Message::system(compaction_system_prompt())];
    messages.extend(history_to_messages(&RuntimeHistory::from_items(prefix)));
    messages
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
    use crate::model::{MockModelClient, ModelResponse};
    use crate::resource::ResourceContext;
    use crate::runtime::{Runtime, RuntimeServices};
    use crate::tools::{ToolRegistry, ToolRouter};
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
}
