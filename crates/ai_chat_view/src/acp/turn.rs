use std::collections::HashSet;

use agent_client_protocol::schema::v1::{
    ContentBlock, SessionUpdate, StopReason, ToolCallId, ToolCallStatus,
};
use agent_runtime::TurnId;
use tokio::sync::watch;

/// 一轮的**进展刻度**：等待方拿它判断「agent 还活着吗」。
///
/// 两个量各管一件事：
///
/// - `revision`：每收到一条该会话的通知就 +1。模型在流式吐字、工具在换状态，都算进展；
/// - `inflight_tools`：已开始、尚未收尾的工具调用数。**大于 0 时不判超时**。
///
/// 后者不是保险起见，是实测逼出来的。OpenCode 1.18.30 在整个 `task`（子代理）执行期间
/// **一条通知都不发**：本机会话库里 193 次子代理，142 次超过 5 分钟、104 次超过 10 分钟、
/// 28 次超过 30 分钟，最长 182 分钟。任何按「多久没消息」计的固定阈值都会把它们全杀掉，
/// 所以判据只能是「没有工具在跑，且真的静默」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnProgress {
    pub(crate) revision: u64,
    pub(crate) inflight_tools: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct AcpTurnTracker {
    turn_id: TurnId,
    received_assistant_content: bool,
    received_reasoning: bool,
    received_tool_activity: bool,
    received_plan: bool,
    /// 已开始、尚未收尾的工具调用；见 [`TurnProgress::inflight_tools`]。
    inflight_tools: HashSet<ToolCallId>,
    /// 进展刻度的发送端。接收端由 `AcpConnection::try_prompt` 持有，守着超时。
    progress: watch::Sender<TurnProgress>,
}

impl AcpTurnTracker {
    pub(crate) fn new(turn_id: TurnId) -> Self {
        Self {
            turn_id,
            received_assistant_content: false,
            received_reasoning: false,
            received_tool_activity: false,
            received_plan: false,
            inflight_tools: HashSet::new(),
            progress: watch::channel(TurnProgress::default()).0,
        }
    }

    pub(crate) fn turn_id(&self) -> &TurnId {
        &self.turn_id
    }

    /// 订阅进展刻度。必须在 tracker 装进 `active_turn` **之后**用完就取，否则会漏掉
    /// 取之前到达的通知（`revision` 只增不减，但接收端初始值是 `default`）。
    pub(crate) fn progress(&self) -> watch::Receiver<TurnProgress> {
        self.progress.subscribe()
    }

    pub(crate) fn observe(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                self.received_assistant_content |= content_is_non_empty(&chunk.content);
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                self.received_reasoning |= content_is_non_empty(&chunk.content);
            }
            SessionUpdate::ToolCall(call) => {
                self.received_tool_activity = true;
                self.track_tool(&call.tool_call_id, call.status);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.received_tool_activity = true;
                // 只有**带状态**的更新才动在飞集合。工具输出是增量推送的，那些更新
                // 不带状态，不能因为收到它们就把工具当成结束了。
                if let Some(status) = update.fields.status {
                    self.track_tool(&update.tool_call_id, status);
                }
            }
            SessionUpdate::Plan(_) => self.received_plan = true,
            _ => {}
        }
        self.publish_progress();
    }

    pub(crate) fn finish_success(self, stop_reason: StopReason) -> TurnOutcome {
        if stop_reason == StopReason::Cancelled {
            return TurnOutcome::Cancelled;
        }
        if self.has_output() {
            TurnOutcome::Completed
        } else {
            TurnOutcome::EmptyResponse
        }
    }

    fn has_output(&self) -> bool {
        self.received_assistant_content
            || self.received_reasoning
            || self.received_tool_activity
            || self.received_plan
    }

    fn track_tool(&mut self, id: &ToolCallId, status: ToolCallStatus) {
        if tool_is_running(status) {
            self.inflight_tools.insert(id.clone());
        } else {
            self.inflight_tools.remove(id);
        }
    }

    fn publish_progress(&self) {
        let inflight = self.inflight_tools.len();
        self.progress.send_modify(|progress| {
            progress.revision = progress.revision.wrapping_add(1);
            progress.inflight_tools = inflight;
        });
    }
}

/// 这个工具状态算不算「还在跑」。
///
/// `Pending` 也算：它的定义是「还没开始跑，因为入参还在流、或者正在等审批」——两种情况
/// agent 都还在等外部条件，不是卡死。`ToolCallStatus` 是 `#[non_exhaustive]`，未来新增的
/// 变体一律**不算在跑**：宁可让超时早一点生效（用户还能按停止），也不要因为认不出一个新
/// 状态而把这一轮永远挂在「正在响应」。
fn tool_is_running(status: ToolCallStatus) -> bool {
    matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    Completed,
    Cancelled,
    EmptyResponse,
}

fn content_is_non_empty(content: &ContentBlock) -> bool {
    match content {
        ContentBlock::Text(text) => !text.text.is_empty(),
        _ => true,
    }
}
