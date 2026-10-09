//! 可运行的 Agent 聊天视图。
//!
//! 本视图把 `agent_runtime` 的事件流、`AgentInput` 和通用消息列表接起来:
//! 提交用户输入后用 `run_turn_blocking` 驱动一轮任务,事件泵持续把
//! `RuntimeEvent` 归约进 `AgentTranscript`。
//!
//! 作为输入框的"上层"集成点:把 [`ResourceContext`] 映射为输入框展示用的
//! [`AgentComposerContext`],注入模型 / 工具执行模式的下拉选项,并处理输入框
//! emit 的选择事件(目标轮换、模型 / 模式切换)。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    AvailableCommandInput, ContentBlock, ImageContent, TextContent,
};
use agent_runtime::{
    AgentResourceScope, HistoryItem, ResourceCatalog, ResourceContext, ResourceId, ResourceKind,
    ResourceRef, Runtime, RuntimeEvent, RuntimeEventReceiver, SessionId, TaskKind, ToolCallId,
    ToolExecutionMode, ToolRegistry, TurnId, UserInput,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, MouseButton, NavigationDirection, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, WindowExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    popover::Popover,
    spinner::Spinner,
    v_flex,
};
use one_assets::IconName;
#[cfg(not(test))]
use one_core::gpui_tokio::Tokio;
use one_core::llm::{GlobalProviderState, LlmConnector, LlmProvider, ProviderConfig};
use one_core::settings::{AiChatToolExecutionMode, AppSettings};
use one_core::sidebar_contribution::SidebarPlacement;
use one_ui::{IconButton, IconButtonRole, PanelHeader, PanelHeaderVariant};
use rust_i18n::t;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// 仅测试用例使用：给子代理详情回放造一个合成轮次 id。
#[cfg(test)]
use crate::acp::detail_turn_id_for;
/// 仅后台探测路径使用（测试构建下探测被禁用以避免真实子进程）。
#[cfg(not(test))]
use crate::acp::{AcpAgentConfig, AcpProbeRecord, acp_probe_cache, probe_fingerprint};
use crate::acp::{
    AcpAgentEntry, AcpAgentProbe, AcpClientProviders, AcpConnectOutcome, AcpConnection,
    AcpConnectionPhase, AcpElicitationEnvelope, AcpElicitationMessage, AcpElicitationOutcome,
    AcpError, AcpErrorKind, AcpModelInfo, AcpPendingConnection, AcpPermissionEnvelope,
    AcpPermissionMessage, AcpPermissionOutcome, AcpPromptStartError, AcpPublicMcpApprovalEnvelope,
    AcpPublicMcpApprovalMessage, AcpPublicMcpApprovalOutcome, AcpPublicMcpApprovalProvider,
    AcpRecoveryAction, AcpSessionContinuity, AcpSessionState, AcpSessionSummary, AcpUsage,
    acp_elicitation_channel, acp_permission_channel, acp_public_mcp_approval_channel,
    acp_session_list_supported, acp_session_summaries, acquire_acp_permission_grant,
    build_acp_agent_entries, current_acp_tool_mode, detail_session_id_for, is_detail_session_id,
    set_current_acp_tool_mode,
};
use crate::acp_agent_config::{AcpAgentConfigEvent, acp_agent_config_notifier};
use crate::agent_cards::{
    ApproveToolCall, OpenFileInReview, OpenSubagentDetail, PlanCardData, RejectToolCall,
    SelectAcpPermissionOption, SubAgentCardData,
};
use crate::agent_skills::AgentSkillState;
use crate::agent_transcript::AgentTranscript;
use crate::bridge::build_runtime_from_llm_provider;
use crate::code_block::{CodeBlockAction, CodeBlockActionRegistry};
use crate::expansion_state::ExpansionState;
use crate::find_shortcut::{
    AI_CHAT_SEARCH_CONTEXT, FindNextInTranscript, FindPreviousInTranscript, ToggleTranscriptFind,
};
use crate::input::{
    AgentComposerContext, AgentInput, AgentInputEvent, ComposerAgentOption, ComposerBranchOption,
    ComposerMenuOption, ComposerModelOption, ComposerPlanItem, ComposerResourcePoolItem,
    ComposerResourcePoolSummary, ComposerResourceSourceOption, ComposerResourceTypeFilter,
    ComposerScope, ComposerSkillItem, ComposerSkillSummary, ComposerSubAgentItem, ComposerTarget,
    ComposerWorkspaceInfo, ComposerWorkspaceOption, ComposerWorktreeState, MentionItem,
    QueuedPromptPreview, SlashCommandItem,
};
use crate::message::ChatMessageUI;
use crate::message_turn_view::{
    MessageListAction, MessageListActionHandler, MessageListContext, render_message_list,
};
use crate::message_view::{MessageListLayout, render_running_activity};
use crate::pending_submission::{PendingSubmission, PendingSubmissions};
use crate::persistence;
use crate::resource_display::first_visible_alias;
use crate::session_shortcut::{NavigateSessionBack, NavigateSessionForward, ToggleSessionSwitcher};
use crate::session_sidebar::{self, SessionRowStyle, SessionSummary};
use crate::theme::{AgentChatTheme, resolve_agent_chat_theme, sp};
use crate::transcript_scroll::TranscriptScrollState;
use crate::transcript_search::TranscriptSearch;
use crate::turn::{TurnOutcome, TurnTimings};
use crate::usage::{ContextUsage, format_local_usage, model_context_window};

mod acp_options;
pub(crate) mod acp_sessions;
mod acp_ui;
mod decision_dock;
mod findbar;
mod session_navigation;
mod session_switcher;

use acp_options::{
    agent_option_disabled, composer_agent_options, composer_agent_options_with_status,
    current_agent_label,
};
use acp_sessions::{acp_session_placeholder, acp_session_row, acp_session_section_header};
use acp_ui::AcpConnectOperation;
use session_navigation::SessionNavigation;
use session_switcher::SessionSwitcherUi;

/// Agent 聊天视图事件。
#[derive(Clone, Debug)]
pub enum AgentChatViewEvent {
    /// 关闭面板。
    Close,
    /// 请求宿主把面板移动到指定位置。
    MoveTo(SidebarPlacement),
    TurnStarted {
        session_id: String,
        turn_id: String,
    },
    TurnFinished {
        session_id: String,
        turn_id: String,
        success: bool,
    },
    /// 用户点击某轮页脚的「回到这一轮」：请求宿主把工作区状态退回该轮结束时。
    ///
    /// 视图只转发请求——它不知道工作区有没有快照、也没有能力改文件；实际回滚由
    /// 宿主的工作区浏览器执行。
    RestoreTurn {
        session_id: String,
        turn_id: String,
    },
    /// 用户点了改动摘要里的某个文件：请求宿主在审阅面板里打开**该文件的改动**。
    ///
    /// 与 [`Self::RestoreTurn`] 同理,视图只转发定位信息 —— 打开文档需要窗口,
    /// 面板在哪、这份 diff 从哪一轮的快照里裁,都由宿主决定。
    ///
    /// `turn_id` 就是「裁哪一轮」：点历史轮的改动文件时，要的是**那一刻**的 diff。
    /// 给不出来（历史恢复的轮次没有 turn id）时为 `None`，宿主退到最近一轮。
    OpenFileInReview {
        session_id: String,
        path: String,
        turn_id: Option<String>,
    },
    /// 用户点了子代理卡片上的「查看推理过程」：请求宿主把子代理详情面板切到前台。
    ///
    /// 视图只转发请求，面板建在哪、怎么开由宿主决定（与 [`Self::OpenFileInReview`] 同理）。
    /// 数据本身不走这里——面板从面板层按 `detail_session_id` 现取，见
    /// [`AgentChatView::subagent_detail_messages`]。
    SubagentDetailRequested {
        /// 子会话的**协议** id（`session/load` 的地址，也是面板的稳定标识）。
        acp_session_id: String,
        /// 卡片标题（工具调用标题），面板头部用它标明「看的是哪只子代理」。
        title: String,
    },
    /// 子代理详情转录有了新内容：面板据此重绘。
    ///
    /// 单独一个事件而不是让面板 observe 整个聊天视图：主会话每个 token 都会
    /// `cx.notify()`，observe 会把详情面板一起拖着重绘（一次 `render_messages`
    /// 覆盖上千条消息），代价直接落在流式输出上。
    SubagentDetailUpdated {
        detail_session_id: String,
    },
}

/// 根据模型选项构建对应运行时。
pub type AgentRuntimeFactory =
    Arc<dyn Fn(&ComposerModelOption) -> anyhow::Result<Arc<Runtime>> + Send + Sync + 'static>;

const MAX_CACHED_SESSION_TRANSCRIPTS: usize = 32;
const MAX_RUNTIME_EVENT_BATCH_SIZE: usize = 64;

fn runtime_event_matches_session(event: &RuntimeEvent, session_filter: Option<&SessionId>) -> bool {
    session_filter.is_none_or(|session_id| {
        event.session_id() == session_id
            // 子代理详情会话的事件也归这条连接的泵：它们同属一个 agent 进程，只是
            // 另一条会话。挡在这里等于把整段子代理推理丢掉 —— 而且丢得无声无息。
            || is_detail_session_id(&event.session_id().to_string())
    })
}

/// 这条事件是不是子代理详情会话的？是就返回它的详情会话 id。
fn subagent_detail_uid(event: &RuntimeEvent) -> Option<String> {
    let session_id = event.session_id().to_string();
    is_detail_session_id(&session_id).then_some(session_id)
}

/// 一批已就绪的事件，外加「取这批的过程中被通道挤掉了多少条」。
#[derive(Default)]
struct RuntimeEventBatch {
    /// 本批实际拿到、且属于目标会话的事件。
    events: Vec<RuntimeEvent>,
    /// 广播通道挤掉的事件数。
    ///
    /// 必须往上报，不能吞掉：被挤掉的是**任意**事件，其中可能正好是某一轮的
    /// 终态。终态是清「正在响应」和推进排队消息的唯一触发点，丢了就再没有任何
    /// 后续事件会补上——界面会永久停在工作中，而且不留任何痕迹
    /// （日志里只有驱动侧那行，无从判断事件到没到）。见
    /// [`AgentChatView::on_runtime_events_dropped`]。
    skipped: u64,
}

fn collect_ready_runtime_events(
    rx: &mut RuntimeEventReceiver,
    first: RuntimeEvent,
    session_filter: Option<&SessionId>,
) -> RuntimeEventBatch {
    let mut batch = RuntimeEventBatch::default();
    if runtime_event_matches_session(&first, session_filter) {
        batch.events.push(first);
    }

    for _ in batch.events.len()..MAX_RUNTIME_EVENT_BATCH_SIZE {
        match rx.try_recv() {
            Ok(event) if runtime_event_matches_session(&event, session_filter) => {
                if batch.events.len() == 1 {
                    batch.events.reserve(MAX_RUNTIME_EVENT_BATCH_SIZE - 1);
                }
                batch.events.push(event);
            }
            // 别的会话的事件：本泵不认，丢给对应会话的泵。不算 lag。
            Ok(_) => {}
            Err(TryRecvError::Lagged(skipped)) => {
                // 游标已被挪到最旧的可读位置，继续取就是「还在的那批」里最旧的。
                // 丢掉的条数单独记下来，由调用方决定怎么补。
                batch.skipped = batch.skipped.saturating_add(skipped);
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
    batch
}

/// 当前驱动后端:自研内核(One_Agent)或外部 ACP agent。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    /// 自研内核(默认)。
    Local,
    /// 外部 ACP agent。
    Acp,
}

/// composer「执行模式」下拉里的一项：任务类型 + 工具策略。
///
/// 早期任务类型与工具策略是两个独立下拉，后来合并成一个；这里把「问答」这个任务
/// 类型选择保留下来，避免合并之后 `TaskKind::Ask` 彻底不可达。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct ExecutionSelection {
    task: TaskKind,
    tool: ToolExecutionMode,
}

impl ExecutionSelection {
    fn new(task: TaskKind, tool: ToolExecutionMode) -> Self {
        Self { task, tool }
    }

    /// 普通轮次：任务类型固定 Agent，只挑工具策略。
    fn tool(tool: ToolExecutionMode) -> Self {
        Self {
            task: TaskKind::Agent,
            tool,
        }
    }

    /// 下拉里展示的文案。
    fn label(self) -> String {
        match self.task {
            TaskKind::Ask => t!("AgentUi.ask_mode").to_string(),
            TaskKind::Plan => t!("AgentUi.plan_mode").to_string(),
            TaskKind::Agent => tool_execution_mode_label(self.tool),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AcpTurnOwner {
    event_session_id: SessionId,
    session_uid: String,
    turn_id: TurnId,
    /// 这一轮是否已被转入后台：新建对话/切走会话时置位。
    ///
    /// 后台轮次的输出照常回流并落进对应会话的转录缓存，但不再驱动当前界面的
    /// 「正在响应」状态；取消（停止按钮）也只作用于前台轮次。
    backgrounded: bool,
    cancel_requested: bool,
}

impl AcpTurnOwner {
    fn mark_cancel_requested(&mut self, session_uid: &str, has_connection: bool) -> bool {
        if !has_connection
            || self.session_uid != session_uid
            || self.cancel_requested
            || self.backgrounded
        {
            return false;
        }
        self.cancel_requested = true;
        true
    }
}

/// 正在放行的**历史回放**：`session/load` 把整段历史重放成一批 `session/update`。
///
/// 与 [`AcpTurnOwner`] 分开，是因为回放**不属于任何一轮**——它不是跑出来的输出，没有
/// prompt、没有终态，`is_running` 那些「有一轮在跑」的判断也不该被它影响。它只需要
/// 一道门：这批事件必须落进转录（否则用户点开一条历史会话什么都看不到），但又只能落
/// 进**它自己那条会话**。
///
/// 所以窗口只认「哪条连接事件流 + 哪个合成轮次」，两者都对上才放行；其余一律丢弃——
/// 迟到的输出绝不能算进别人的会话。
///
/// 窗口不需要精确关闭：连接那边（[`AcpConnection::end_history_replay`]）在 `load` 响应
/// 回来时就把回放轮次从 `session/update` 上摘掉了，之后不会再有任何事件带这个 id。
/// 所以这里留到下一次打开会话时被覆盖、或连接被收掉时清掉（[`Self::reset_acp_client_session`]）
/// 都不会多放任何一个事件进来。
///
/// 反过来，**不能在 `finish_acp_session_open` 里关闭它**：那批事件此时已经发进事件通道，
/// 事件泵未必已经把尾部几条交给转录，先关窗口就是白丢几条历史。
#[derive(Clone, Debug, PartialEq, Eq)]
struct AcpHistoryReplay {
    event_session_id: SessionId,
    session_uid: String,
    turn_id: TurnId,
}

impl AcpHistoryReplay {
    /// 这条事件是不是这次回放的。
    fn accepts(&self, event: &RuntimeEvent) -> bool {
        event.session_id() == &self.event_session_id
            && runtime_event_turn_id(event) == &self.turn_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AcpOperationToken(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpSessionTransitionPhase {
    Creating,
    /// 正在拉历史会话列表。
    ///
    /// 复用同一个状态机而不是另起一个标志位：渲染层靠它在列表在飞的时候把提交
    /// 排成 `RetryLater`（见 [`submission_start_for_acp_availability`]）——
    /// 此时连接被 `take()` 走了，若不放行排队就会弹「未连接」误报。
    Listing,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AcpSessionTransition {
    operation: AcpOperationToken,
    agent_id: SharedString,
    session_uid: String,
    phase: AcpSessionTransitionPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmissionStart {
    Started,
    RetryLater,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAdvance {
    Started,
    Blocked,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpStopAction {
    /// 发 `session/cancel`,等 agent 回终态。
    CancelActivePrompt,
    /// 取消已发过、agent 仍不回终态：本地结账，不再等。
    ForceLocalStop,
    ReturnToLocal,
    AbandonFailedTransition,
    ClearQueueOnly,
}

/// 发出取消后等 agent 回终态的上限。超时就本地结账。
///
/// `session/cancel` 只是一条通知，协议里 agent 可以不回终态：实测 OpenCode 在
/// provider 卡死时 `session/prompt` 二十多分钟不返回，而它的 `stopReason` 又永远
/// 是 `end_turn`（从不回 `cancelled`）。没有这个上限，界面就永远停在「正在响应」，
/// 再点停止也毫无反应。
const ACP_CANCEL_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

fn acp_stop_action(
    owns_current_turn: bool,
    has_connection: bool,
    cancel_outstanding: bool,
    connecting: bool,
    authentication_pending: bool,
    session_transition: Option<AcpSessionTransitionPhase>,
) -> AcpStopAction {
    if owns_current_turn && has_connection {
        // 已经发过一次取消、agent 还没回终态：用户再点就是「不等了」。
        return if cancel_outstanding {
            AcpStopAction::ForceLocalStop
        } else {
            AcpStopAction::CancelActivePrompt
        };
    }
    if connecting
        || authentication_pending
        || session_transition == Some(AcpSessionTransitionPhase::Creating)
    {
        return AcpStopAction::ReturnToLocal;
    }
    if session_transition == Some(AcpSessionTransitionPhase::Failed) {
        return if has_connection {
            AcpStopAction::AbandonFailedTransition
        } else {
            AcpStopAction::ReturnToLocal
        };
    }
    AcpStopAction::ClearQueueOnly
}

fn submission_start_for_acp_error(error: AcpPromptStartError) -> SubmissionStart {
    match error {
        AcpPromptStartError::AlreadyRunning | AcpPromptStartError::NotReady => {
            SubmissionStart::RetryLater
        }
        AcpPromptStartError::ImageUnsupported => SubmissionStart::Rejected,
    }
}

fn acp_terminal_allows_queue_advance(phase: Option<&AcpConnectionPhase>) -> bool {
    matches!(phase, Some(AcpConnectionPhase::Ready))
}

fn acp_connection_is_unavailable(phase: Option<&AcpConnectionPhase>) -> bool {
    matches!(
        phase,
        Some(AcpConnectionPhase::Failed { .. } | AcpConnectionPhase::Closed)
    )
}

/// 自动重连上限。超过后不再拉起子进程，改为把决定权交回用户。
const ACP_RECONNECT_MAX_ATTEMPTS: u32 = 3;
/// 首次重连延迟；之后按 2 倍退避，避免 agent 反复崩溃时打转。
const ACP_RECONNECT_BASE_DELAY_MS: u64 = 600;
/// 空闲连接的健康检查间隔。agent 进程空闲退出不会产生轮次事件，只能靠轮询发现。
#[cfg_attr(test, allow(dead_code))]
const ACP_HEALTH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpReconnectDecision {
    Reconnect,
    GiveUp,
    Idle,
}

/// 是否应该自动重连。纯决策，便于单测；不触碰任何进程或 GPUI 状态。
///
/// 只有「ACP 后端 + 连接确实不可用 + 没有正在进行的连接动作 + 有明确目标」才重连，
/// 且受尝试次数上限约束。用户主动切走（`has_target == false`）时永不重连。
fn acp_reconnect_decision(
    is_acp_backend: bool,
    connection_unavailable: bool,
    busy: bool,
    has_target: bool,
    attempts: u32,
) -> AcpReconnectDecision {
    if !is_acp_backend || !connection_unavailable || busy || !has_target {
        return AcpReconnectDecision::Idle;
    }
    if attempts >= ACP_RECONNECT_MAX_ATTEMPTS {
        AcpReconnectDecision::GiveUp
    } else {
        AcpReconnectDecision::Reconnect
    }
}

/// 第 `attempts` 次（从 0 起）重连前的等待时长。
fn acp_reconnect_delay(attempts: u32) -> std::time::Duration {
    let shift = attempts.min(3);
    std::time::Duration::from_millis(ACP_RECONNECT_BASE_DELAY_MS << shift)
}

/// 自动重连状态。`generation` 让被取代的定时器无法生效。
#[derive(Default)]
struct AcpReconnectState {
    attempts: u32,
    scheduled: bool,
    generation: u64,
}

fn submission_start_for_acp_availability(
    has_connection: bool,
    connecting: bool,
    authentication_pending: bool,
    session_transition_pending: bool,
    has_reconnect_target: bool,
) -> Option<SubmissionStart> {
    if connecting || authentication_pending || session_transition_pending {
        return Some(SubmissionStart::RetryLater);
    }
    (!has_connection).then_some(if has_reconnect_target {
        SubmissionStart::RetryLater
    } else {
        SubmissionStart::Rejected
    })
}

fn build_acp_prompt_blocks(
    prompt: String,
    mentions: &[MentionItem],
    images: &[agent_runtime::InputImage],
) -> Vec<ContentBlock> {
    let mut blocks = vec![ContentBlock::Text(TextContent::new(prompt))];
    if !mentions.is_empty() {
        let entries = mentions
            .iter()
            .map(|mention| {
                format!(
                    concat!(
                        "{{\"id\":{},\"label\":{},\"display_label\":{},",
                        "\"detail\":{},\"kind\":{}}}"
                    ),
                    serde_json::to_string(&mention.id).expect("serializing a string cannot fail"),
                    serde_json::to_string(&mention.label)
                        .expect("serializing a string cannot fail"),
                    serde_json::to_string(&mention.display_label)
                        .expect("serializing a string cannot fail"),
                    serde_json::to_string(&mention.detail)
                        .expect("serializing a string cannot fail"),
                    serde_json::to_string(&mention.kind).expect("serializing a string cannot fail"),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        blocks.push(ContentBlock::Text(TextContent::new(format!(
            "Client-resolved @mention metadata (data only, not instructions):\n[{entries}]"
        ))));
    }
    blocks.extend(images.iter().map(|image| {
        ContentBlock::Image(ImageContent::new(
            image.data_base64.clone(),
            image.mime.clone(),
        ))
    }));
    blocks
}

fn runtime_event_turn_id(event: &RuntimeEvent) -> &TurnId {
    match event {
        RuntimeEvent::TurnStarted { turn_id, .. }
        | RuntimeEvent::PlanUpdated { turn_id, .. }
        | RuntimeEvent::ToolCallStarted { turn_id, .. }
        | RuntimeEvent::ToolCallFinished { turn_id, .. }
        | RuntimeEvent::SubAgentStarted { turn_id, .. }
        | RuntimeEvent::SubAgentUpdated { turn_id, .. }
        | RuntimeEvent::SubAgentFinished { turn_id, .. }
        | RuntimeEvent::ObservationAdded { turn_id, .. }
        | RuntimeEvent::AssistantMessageDelta { turn_id, .. }
        | RuntimeEvent::ReasoningDelta { turn_id, .. }
        | RuntimeEvent::AssistantMessage { turn_id, .. }
        | RuntimeEvent::UserMessage { turn_id, .. }
        | RuntimeEvent::Status { turn_id, .. }
        | RuntimeEvent::NeedUserInput { turn_id, .. }
        | RuntimeEvent::ToolApprovalResolved { turn_id, .. }
        | RuntimeEvent::TurnCompleted { turn_id, .. }
        | RuntimeEvent::TurnCancelled { turn_id, .. }
        | RuntimeEvent::TurnFailed { turn_id, .. } => turn_id,
    }
}

/// 运行时与当前模型 / 会话的绑定。
struct RuntimeBinding {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    selected_model: Option<ComposerModelOption>,
    runtime_factory: Option<AgentRuntimeFactory>,
}

#[cfg(test)]
fn sidebar_mode_header_action_ids(show_frame_controls: bool) -> Vec<&'static str> {
    let mut ids = vec!["new", "history"];
    if show_frame_controls {
        ids.push("frame-options");
    }
    ids.push("close");
    ids
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SidebarFrameMoveOption {
    placement: SidebarPlacement,
    disabled: bool,
}

fn sidebar_frame_move_options(current: SidebarPlacement) -> Vec<SidebarFrameMoveOption> {
    [
        SidebarPlacement::Left,
        SidebarPlacement::Right,
        SidebarPlacement::Bottom,
    ]
    .into_iter()
    .map(|placement| SidebarFrameMoveOption {
        placement,
        disabled: placement == current,
    })
    .collect()
}

fn sidebar_placement_label(placement: SidebarPlacement) -> &'static str {
    match placement {
        SidebarPlacement::Left => "Left",
        SidebarPlacement::Right => "Right",
        SidebarPlacement::Bottom => "Bottom",
    }
}

fn sidebar_placement_icon(placement: SidebarPlacement) -> IconName {
    match placement {
        SidebarPlacement::Left => IconName::PanelLeft,
        SidebarPlacement::Right => IconName::PanelRight,
        SidebarPlacement::Bottom => IconName::PanelBottom,
    }
}

fn build_sidebar_frame_options_menu(
    menu: PopupMenu,
    view: Entity<AgentChatView>,
    placement: SidebarPlacement,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let move_view = view.clone();
    let close_view = view.clone();
    menu.min_w(px(220.0))
        .submenu_with_icon(
            Some(IconName::PanelRight.into()),
            t!("AgentUi.move_to").to_string(),
            window,
            cx,
            move |submenu, _window, _cx| {
                sidebar_frame_move_options(placement).into_iter().fold(
                    submenu,
                    |submenu, option| {
                        let view = move_view.clone();
                        submenu.item(
                            PopupMenuItem::new(sidebar_placement_label(option.placement))
                                .icon(sidebar_placement_icon(option.placement))
                                .checked(option.disabled)
                                .disabled(option.disabled)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |_this, cx| {
                                        cx.emit(AgentChatViewEvent::MoveTo(option.placement));
                                    });
                                }),
                        )
                    },
                )
            },
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("AgentUi.remove_from_sidebar").to_string())
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    close_view.update(cx, |_this, cx| {
                        cx.emit(AgentChatViewEvent::Close);
                    });
                }),
        )
}

/// 侧边栏头部图标按钮统一样式：前景色跟随 Agent 主题。
///
/// `Button` 渲染时会用变体前景色覆盖 `Styled::text_color`，ghost
/// 变体读全局应用主题，在自定义 Agent 配色下图标会变成黑色。
fn agent_header_icon_variant(theme: &AgentChatTheme, cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .foreground(theme.foreground)
        .hover(theme.foreground.opacity(0.12))
        .active(theme.foreground.opacity(0.12))
}

fn agent_history_title(show_archived: bool) -> String {
    if show_archived {
        t!("AgentUi.archived_tasks").to_string()
    } else {
        t!("AgentUi.history_tasks").to_string()
    }
}

fn current_agent_task_title() -> String {
    t!("AgentUi.current_agent_task").to_string()
}

fn persistence_title_from_input(text: &str) -> String {
    const MAX_TITLE_CHARS: usize = 40;

    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let first_line = first_line.trim();
    if first_line.is_empty() {
        return current_agent_task_title();
    }
    if first_line.chars().count() <= MAX_TITLE_CHARS {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(MAX_TITLE_CHARS).collect();
        format!("{truncated}…")
    }
}

/// 会话切换前是否要停掉在飞轮次。
///
/// 历史上 ACP 在这里返回 `true`（切走即取消）；后台轮次方案后 ACP 轮次改为
/// 转入后台继续跑，本地运行时从不需要切换前强停——两侧都返回 `false`，
/// 保留这个函数是为了让切换守卫语义集中在一处，也留着给未来需要强停的后端。
fn should_stop_task_before_session_switch(backend: Backend) -> bool {
    let _ = backend;
    false
}

/// 这次会话切换是怎么发起的——决定导航栈（后退/前进）怎么记账。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionSwitchOrigin {
    /// 用户主动访问（侧栏点击 / 切换器提交 / 公共 API）：把来源压入 back 栈。
    Visit,
    /// 后退导航：栈已在调用前移动，切换本身不再记账。
    Back,
    /// 前进导航：同上。
    Forward,
}

/// 输入框的一次性草稿载荷：文字 + 图片附件。
///
/// 文字来自 Runtime 会话的 `draft`（可落盘）；图片来自
/// [`AgentChatView::session_draft_attachments`]（只在进程内，见字段注释）。
/// 由 [`AgentChatView::stage_session_draft`] 组装、`render` 时
/// [`AgentChatView::apply_pending_input_draft`] 写进输入框。
#[derive(Default)]
struct ComposerDraft {
    text: String,
    images: Vec<crate::ImageAttachment>,
}

fn merge_live_session_summaries(
    persisted: Vec<SessionSummary>,
    live: &[SessionSummary],
    current_session: &str,
    running_sessions: &HashSet<String>,
    show_archived: bool,
) -> Vec<SessionSummary> {
    if show_archived {
        return persisted;
    }

    let persisted_ids: HashSet<_> = persisted
        .iter()
        .map(|summary| summary.id.as_str())
        .collect();
    let mut live_by_id = HashMap::new();
    let mut summaries = Vec::with_capacity(persisted.len() + live.len());
    for summary in live {
        if summary.id == current_session || running_sessions.contains(&summary.id) {
            if !persisted_ids.contains(summary.id.as_str()) {
                summaries.push(summary.clone());
            }
            live_by_id.insert(summary.id.as_str(), summary);
        }
    }

    summaries.extend(persisted.into_iter().map(|summary| {
        // 实时摘要只提供「还在内存里变」的字段：标题与活跃时间。
        //
        // 工作区归属与外部 agent 来源是**落盘快照**的事实，不能被内存副本冲掉：
        // 内存那份归属来自 `session_roots`，而它可能压根没有这个会话的记录
        // （例如进程启动时建的那个初始会话从没进过表）。整体 `clone()` 覆盖
        // 会让侧栏那一行从它的工作区分组掉进「未分组」，用户点一下别的会话
        // 它又跳回来——「点一下分组就变了 / 顺序就变了」。
        match live_by_id.get(summary.id.as_str()) {
            Some(live) => SessionSummary {
                name: live.name.clone(),
                updated_at: live.updated_at,
                ..summary
            },
            None => summary,
        }
    }));
    summaries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    summaries
}

fn themed_session_row_style(theme: &AgentChatTheme) -> SessionRowStyle {
    SessionRowStyle {
        foreground: theme.foreground,
        muted_foreground: theme.muted_foreground,
        selected_background: theme.selection_background(),
        selected_foreground: theme.foreground,
        hover_background: theme.hover_background(),
    }
}

fn running_session_indicator_color(selected: bool, style: SessionRowStyle) -> gpui::Hsla {
    if selected {
        style.selected_foreground
    } else {
        style.foreground
    }
}

fn running_session_animation_id(uid: &str) -> SharedString {
    SharedString::from(format!("agent-session-running-animation-{uid}"))
}

impl RuntimeBinding {
    fn new(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        selected_model: Option<ComposerModelOption>,
        runtime_factory: Option<AgentRuntimeFactory>,
    ) -> Self {
        let session = runtime.create_session(resources);
        Self {
            runtime,
            session_id: session.id().clone(),
            selected_model,
            runtime_factory,
        }
    }

    /// 切换模型：换 Runtime，但**保留当前会话**。
    ///
    /// 本地会话的对话历史只活在 Runtime 内存里（落盘的是快照副本），直接
    /// `create_session` 开一个空会话等于把上下文丢掉——用户看到的就是"切完
    /// 模型它不记得刚才聊过什么"。这里先把当前会话冻结成快照，再在新 Runtime
    /// 上用同一个 session id 恢复，历史 / 计划 / 系统指令 / 技能一起延续。
    ///
    /// 返回 `Ok(true)` 表示已切换（`session_id` 不变即历史已延续），`Ok(false)`
    /// 表示没有可用的 Runtime 工厂、调用方应保持原状。
    fn switch_model(
        &mut self,
        option: &ComposerModelOption,
        resources: &ResourceContext,
    ) -> anyhow::Result<bool> {
        let Some(factory) = &self.runtime_factory else {
            return Ok(false);
        };
        // 先取快照再建 Runtime：工厂报错时 `self` 原封不动，旧 Runtime 仍可用。
        let snapshot = self
            .runtime
            .session(&self.session_id)
            .map(|session| session.snapshot());
        let runtime = factory(option)?;
        let session = match snapshot {
            Some(snapshot) => runtime.restore_session(snapshot),
            None => runtime.create_session(resources.clone()),
        };
        self.runtime = runtime;
        self.session_id = session.id().clone();
        self.selected_model = Some(option.clone());
        Ok(true)
    }
}

/// 工作台外壳注入工具条的侧栏开关：状态读取与切换动作都由外壳提供。
///
/// 图标状态在每次渲染时经 `nav_collapsed` / `right_open` 闭包实时求值，
/// 动作闭包经外壳弱引用回调——外壳与面板互不强持有，销毁顺序无关。
/// 面板会暂存注入值：内层视图是异步构建的，注入可能早于视图存在。
#[derive(Clone)]
pub struct WorkbenchSidebarToggles {
    /// 会话导航栏是否处于收起态。
    pub nav_collapsed: std::sync::Arc<dyn Fn(&gpui::App) -> bool + 'static>,
    /// 切换会话导航栏。
    pub toggle_nav: std::sync::Arc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>,
    /// 右侧标签组是否处于展开态（有标签且未被收起，放大视为展开）。
    pub right_open: std::sync::Arc<dyn Fn(&gpui::App) -> bool + 'static>,
    /// 切换右侧标签组。
    pub toggle_right: std::sync::Arc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>,
}

/// 输入框下方上下文栏的数据快照：工作区 / 分支 / Worktree。
///
/// 宿主每次推送前现算一次（分支要走 git），因此是一次性的值而不是 getter。
#[derive(Clone, Debug, Default)]
pub struct ComposerContextSnapshot {
    pub workspace: ComposerWorkspaceInfo,
    pub workspace_options: Vec<ComposerWorkspaceOption>,
    pub branches: Vec<ComposerBranchOption>,
    pub worktree: ComposerWorktreeState,
}

/// 宿主注入的上下文栏数据源与动作。
///
/// 与 [`WorkbenchSidebarToggles`] 同构：视图只负责展示与转发，真实语义
/// （切工作区、切分支、建 worktree）全在宿主侧，视图不依赖任何业务 crate。
#[derive(Clone)]
pub struct ComposerContextSource {
    /// 取当前快照。每次 [`AgentChatView::sync_composer`] 调用一次。
    pub snapshot: std::sync::Arc<dyn Fn(&mut gpui::App) -> ComposerContextSnapshot + 'static>,
    /// 选中工作区。宿主决定「会话已有消息则在目标工作区新建对话」。
    pub select_workspace:
        std::sync::Arc<dyn Fn(&std::path::Path, &mut gpui::Window, &mut gpui::App) + 'static>,
    /// 「选择其它目录…」：宿主弹自己的目录选择器。
    pub browse_workspace: std::sync::Arc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>,
    /// 切换分支（分支名，可能与远程分支同名）。
    pub select_branch: std::sync::Arc<dyn Fn(&SharedString, &mut gpui::App) + 'static>,
    /// 切换「本会话跑在独立 worktree 上」。
    ///
    /// 勾选只是记下意图（宿主在快照里回 `ComposerWorktreeState::pending`），
    /// 真正的创建发生在第一次提交之前，见 [`Self::prepare_worktree`]。
    pub toggle_worktree: std::sync::Arc<dyn Fn(bool, &mut gpui::App) + 'static>,
    /// 创建待建的 worktree（首次提交前调用，见 [`AgentChatView::defer_submission_for_worktree`]）。
    ///
    /// 返回的 task 在**失败**时给出原因，视图据此中止这次发送并把内容还回输入框；
    /// 成功时它只代表 `git worktree add` 跑完了 —— 换根要经过
    /// `set_root_manually` → `RootChanged` 的**延后派发**，真正落地在
    /// [`AgentChatView::set_workspace_root`]，视图在那里接着发这条被拦下的提交。
    /// 若在这里就继续提交，ACP 会话仍绑在旧根上，等于没切。
    pub prepare_worktree:
        std::sync::Arc<dyn Fn(&mut gpui::App) -> Task<Result<(), SharedString>> + 'static>,
    /// 告诉用户「worktree 没建起来、这条消息没发出去」。
    ///
    /// 由宿主弹，而不是视图自己调组件库的通知：通知需要组件库的窗口状态，
    /// 视图只负责把话递出去（与 `browse_workspace` 弹宿主的目录选择器同一套路）。
    pub report_worktree_failure:
        std::sync::Arc<dyn Fn(&SharedString, &mut gpui::Window, &mut gpui::App) + 'static>,
}

/// 创建 [`AgentChatView`] 所需的配置。
pub struct AgentChatViewConfig {
    pub runtime: Arc<Runtime>,
    pub resources: ResourceContext,
    pub available_resources: Vec<ResourceRef>,
    pub mentions: Vec<MentionItem>,
    pub model_options: Vec<ComposerModelOption>,
    pub selected_model_id: Option<SharedString>,
    pub runtime_factory: Option<AgentRuntimeFactory>,
    /// 以「侧边栏视图」(窄面板)模式渲染:头部走新建对话 / 历史记录 Popover,
    /// 不常驻左侧会话列表。默认 `false`(普通 tab 全宽视图)。
    ///
    /// **重要**：侧边栏模式下 ResourceContext 固定为当前连接，不支持切换。
    pub sidebar_mode: bool,
    /// 侧边栏模式是否渲染内部头部。嵌入到已有外层面板 frame 时可关闭。
    pub show_sidebar_header: bool,
    /// 侧边栏模式是否在内部头部显示宿主 frame 控制入口。
    pub show_sidebar_frame_controls: bool,
    /// 宿主 frame 当前所在位置,用于禁用移动菜单里的当前位置。
    pub sidebar_frame_placement: SidebarPlacement,
    /// 可接入的外部 ACP agent(自定义命令)。非空时头部显示后端切换控件。
    pub acp_agents: Vec<AcpAgentEntry>,
    /// 可选的局部聊天主题。用于终端侧边栏等嵌入场景,普通 Agent tab 保持应用主题。
    pub theme: Option<AgentChatTheme>,
    /// ACP、资源浏览和工作区工具共用根目录。
    pub workspace_root: Option<std::path::PathBuf>,
}

impl AgentChatViewConfig {
    pub fn new(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
    ) -> Self {
        let option = static_runtime_model_option(&runtime);
        let available_resources = resources.resources.clone();
        Self {
            runtime,
            resources,
            available_resources,
            mentions,
            model_options: vec![option.clone()],
            selected_model_id: Some(option.id),
            runtime_factory: None,
            sidebar_mode: false,
            show_sidebar_header: true,
            show_sidebar_frame_controls: false,
            sidebar_frame_placement: SidebarPlacement::Right,
            acp_agents: Vec::new(),
            theme: None,
            workspace_root: None,
        }
    }

    pub fn new_with_scope(
        runtime: Arc<Runtime>,
        scope: AgentResourceScope,
        catalog: ResourceCatalog,
        mentions: Vec<MentionItem>,
    ) -> Self {
        let resources = scope.to_resource_context();
        let mut config = Self::new(runtime, resources, mentions);
        config.available_resources = catalog.resources;
        config
    }

    /// 切换为「侧边栏视图」(窄面板)模式。
    pub fn sidebar_mode(mut self, enabled: bool) -> Self {
        self.sidebar_mode = enabled;
        self
    }

    pub fn show_sidebar_header(mut self, visible: bool) -> Self {
        self.show_sidebar_header = visible;
        self
    }

    pub fn show_sidebar_frame_controls(
        mut self,
        visible: bool,
        placement: SidebarPlacement,
    ) -> Self {
        self.show_sidebar_frame_controls = visible;
        self.sidebar_frame_placement = placement;
        self
    }

    /// 注入可接入的外部 ACP agent 列表。
    pub fn with_acp_agents(mut self, agents: Vec<AcpAgentEntry>) -> Self {
        self.acp_agents = agents;
        self
    }

    /// 注入局部聊天主题。
    pub fn with_theme(mut self, theme: AgentChatTheme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn with_workspace_root(mut self, root: std::path::PathBuf) -> Self {
        self.workspace_root = Some(root);
        self
    }

    pub fn with_available_resources(mut self, resources: Vec<ResourceRef>) -> Self {
        self.available_resources = resources;
        self
    }

    pub fn with_models(
        mut self,
        model_options: Vec<ComposerModelOption>,
        selected_model_id: Option<SharedString>,
        runtime_factory: AgentRuntimeFactory,
    ) -> Self {
        self.model_options = model_options;
        self.selected_model_id = selected_model_id;
        self.runtime_factory = Some(runtime_factory);
        self
    }

    /// 用正式 provider 配置创建 Agent tab 配置。
    ///
    /// 适用于普通 provider；`Navop` 这类需要 `GlobalProviderState` 的 provider 请使用
    /// [`AgentChatViewConfig::from_provider_state`]。
    pub fn from_provider_configs(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        provider_configs: Vec<ProviderConfig>,
        registry: ToolRegistry,
    ) -> anyhow::Result<Self> {
        let specs = runtime_specs_from_provider_configs(provider_configs, registry)?;
        Self::from_runtime_specs(resources, mentions, specs)
    }

    /// 用 `GlobalProviderState` 创建 Agent tab 配置,支持 Navop provider。
    pub async fn from_provider_state(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        provider_configs: Vec<ProviderConfig>,
        registry: ToolRegistry,
        provider_state: GlobalProviderState,
    ) -> anyhow::Result<Self> {
        let specs =
            runtime_specs_from_provider_state(provider_configs, registry, provider_state).await?;
        Self::from_runtime_specs(resources, mentions, specs)
    }

    fn from_runtime_specs(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        specs: Vec<RuntimeBuildSpec>,
    ) -> anyhow::Result<Self> {
        let initial = specs
            .iter()
            .find(|spec| spec.is_default)
            .or_else(|| specs.first())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!(t!("AgentUi.no_model_config").to_string()))?;
        let runtime = initial.build()?;
        let selected_model_id = selected_provider_model_id(&specs);
        let model_options = specs.iter().map(|spec| spec.option.clone()).collect();
        let spec_map: Arc<HashMap<String, RuntimeBuildSpec>> = Arc::new(
            specs
                .into_iter()
                .map(|spec| (spec.option.id.to_string(), spec))
                .collect(),
        );
        let runtime_factory: AgentRuntimeFactory = Arc::new(move |option| {
            let spec = spec_map
                .get(option.id.as_ref())
                .ok_or_else(|| anyhow::anyhow!("unknown agent model option: {}", option.id))?;
            spec.build()
        });

        Ok(Self::new(runtime, resources, mentions).with_models(
            model_options,
            selected_model_id,
            runtime_factory,
        ))
    }
}

#[derive(Clone)]
struct RuntimeBuildSpec {
    option: ComposerModelOption,
    provider: Arc<dyn LlmProvider>,
    model: String,
    registry: ToolRegistry,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    is_default: bool,
}

impl RuntimeBuildSpec {
    fn build(&self) -> anyhow::Result<Arc<Runtime>> {
        build_runtime_from_llm_provider(
            self.provider.clone(),
            self.model.clone(),
            self.registry.clone(),
            self.temperature,
            self.max_tokens,
        )
    }
}

#[derive(Default)]
struct AutoScrollState {
    pending_bottom_scroll_frames: usize,
}

impl AutoScrollState {
    fn request(&mut self) {
        self.request_frames(2);
    }

    fn request_settle(&mut self) {
        self.request_frames(5);
    }

    fn request_frames(&mut self, frames: usize) {
        self.pending_bottom_scroll_frames = self.pending_bottom_scroll_frames.max(frames);
    }

    fn take_pending_for_render(&mut self) -> bool {
        if self.pending_bottom_scroll_frames == 0 {
            return false;
        }
        self.pending_bottom_scroll_frames -= 1;
        true
    }
}

/// Runtime 驱动的 Agent 聊天面板。
pub struct AgentChatView {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    workspace_root: std::path::PathBuf,
    resources: ResourceContext,
    available_resources: Vec<ResourceRef>,
    transcript: AgentTranscript,
    input: Entity<AgentInput>,
    /// 会话切换后待写入输入框的草稿；`Some` 才有动作，在下一次 `render` 应用。
    ///
    /// 之所以不直接写输入框：`InputState::set_value` 需要 `&mut Window`，
    /// 而 `switch_session` / `start_fresh_session` 全链路都没有 window（改签名
    /// 会波及十几处调用点与公共 API）。渲染恰好在切换后必然发生，把「换输入框
    /// 内容」推迟到那一帧，语义等价且不动任何调用方。
    pending_input_draft: Option<ComposerDraft>,
    /// 每个会话自己的**图片**草稿（文字走 Runtime 会话的 `draft`，见
    /// [`Self::capture_session_draft`]）。
    ///
    /// 图片不进会话快照——几 MB 的像素数据会让快照膨胀到不可接受——所以只在
    /// 进程内有效：切换会话时图片跟着会话走、互不串台，重启后只恢复文字。
    session_draft_attachments: HashMap<String, Vec<crate::ImageAttachment>>,
    /// 上一次离开的「空白会话」，供「新建会话」复用（见 [`Self::new_session`]）。
    ///
    /// 只在**本地后端**记：ACP 的转录空白不代表 agent 那边没有上下文
    /// （`session/resume` 就是不回放历史的，转录看着空、聊天记录在 agent 手里）。
    /// 内存态，不持久化：重启后这些白纸本来就没了。
    draft_session: Option<String>,
    /// 会话后退/前进导航栈（见 [`session_navigation`] 模块文档）。
    session_navigation: SessionNavigation,
    /// 最近会话切换器的状态机（见 [`session_switcher`] 模块文档）。
    session_switcher: SessionSwitcherUi,
    /// 切换器 overlay 的焦点句柄：切换器打开时把焦点收进来，这样
    /// `AgentSessionSwitcher` 上下文里的键（tab / enter / escape）才派发得到。
    session_switcher_focus: FocusHandle,
    sessions: Vec<SessionSummary>,
    /// 会话 id → 归属工作区。创建/载入时定格，落盘时随快照写出。
    session_roots: HashMap<String, String>,
    /// 尚未完全由持久化历史覆盖的实时会话摘要（当前会话和后台运行会话）。
    live_sessions: Vec<SessionSummary>,
    /// 非当前会话的实时转录，切换回来时可继续看到流式进度。
    session_transcripts: HashMap<String, AgentTranscript>,
    /// 非当前会话转录的 LRU 顺序，队首为最久未访问项。
    session_transcript_order: VecDeque<String>,
    /// 子代理**详情会话**的转录，键是详情会话 id（`acp-sub:<子会话 id>`）。
    ///
    /// 与 [`Self::session_transcripts`] 分开存是有意的：那个 map 装的是「别的**真实**
    /// 会话」，会进 LRU、会落盘、会出现在会话列表里；详情会话只是某张卡片的一次回放，
    /// 既不落盘也不该被 LRU 挤掉——用户正看着它。
    subagent_details: HashMap<String, AgentTranscript>,
    /// 详情会话加载失败的文案，键同 [`Self::subagent_details`]。
    subagent_detail_errors: HashMap<String, String>,
    /// 已经发出过 `session/load` 的子会话协议 id。
    ///
    /// 用于防重复：每 load 一次，agent 就把整段子代理历史重放一遍；子代理动辄上千条
    /// part，重复触发既浪费也会把转录灌花。
    subagent_loads: HashSet<String>,
    /// 当前 Runtime 中仍在执行的会话集合。
    running_sessions: HashSet<String>,
    /// 本地 stop 后不再允许影响后续轮次状态的旧 turn。
    ignored_local_turns: HashSet<TurnId>,
    /// 每个本地会话当前异步提交/审批操作的代次，用于丢弃迟到回调。
    local_operation_generations: HashMap<String, u64>,
    /// 已删除或归档的会话 tombstone，用于丢弃异步迟到事件/回调。
    closed_sessions: HashSet<String>,
    /// 按会话隔离、等待下一轮执行的用户提交。
    pending_submissions: PendingSubmissions,
    current_session: String,
    sidebar_collapsed: bool,
    /// 工作台外壳接管左侧会话栏时整块隐藏内建侧栏（不渲染折叠 rail）。
    sidebar_suppressed: bool,
    /// 工作台外壳注入的侧栏开关；`Some` 时工具条在 agent 切换器两侧渲染
    /// 导航栏开关（左）与右侧标签组开关（右），替代外壳顶栏。
    workbench_toggles: Option<WorkbenchSidebarToggles>,
    /// 宿主注入的输入框下方上下文栏数据源（工作区 / 分支 / Worktree）。
    /// `None` 时底栏只渲染「权限级别」一项，其余入口不出现。
    composer_context_source: Option<ComposerContextSource>,
    /// 被「worktree 待创建」拦下、等根切过去之后再发的提交（先来先发）。
    ///
    /// 用队列而不是单个槽位：创建要秒级（`git worktree add` 是子进程），这期间
    /// 用户完全可能再发一条；单槽位会把上一条静默覆盖掉。
    gated_submissions: VecDeque<PendingSubmission>,
    /// 滚动三态（跟随尾巴 / 阅读历史 / 锚点跳转中）。
    scroll: TranscriptScrollState,
    /// 过程块展开态覆盖表；按稳定 id 记录用户的显式展开/收起。
    expansion: ExpansionState,
    /// 轮次起止时间；折叠头里的「用时 Ns」只来自这里，缺失就不显示。
    turn_timings: TurnTimings,
    /// 会话 id → 该会话里「已捕获快照、可以回滚」的轮次 id。
    ///
    /// 快照存在工作区浏览器的 git ref 里，视图拿不到也不该去查，所以这张表完全由
    /// 宿主推送。按会话分桶是为了切换会话时不会把上一会话的轮次当成可回滚——那会渲染
    /// 出一个点了没反应的按钮。
    restorable_turns: HashMap<String, HashSet<String>>,
    /// 决策栏里**显式展开详情**的那一项 id；`None` 表示全部收起。
    ///
    /// 详情展开是纯展示动作，不取键盘焦点——绝不让「用户正在打字」时 Enter 变成「允许」。
    decision_details: Option<String>,
    /// 会话内搜索状态（纯逻辑）；渲染层只读它。
    search: TranscriptSearch,
    /// 上次重算命中时的转录版本号；正文没变不重算。
    search_revision: u64,
    /// findbar 是否打开。
    findbar_open: bool,
    /// findbar 的查询输入框。
    findbar_input: Entity<InputState>,
    /// 侧边栏是否显示「已归档」会话(否则显示活跃会话)。
    show_archived: bool,
    /// 侧边栏视图(窄面板)模式:头部走新建对话 / 历史记录紧凑布局,不常驻会话列表。
    sidebar_mode: bool,
    /// 侧边栏视图是否显示内部头部。
    show_sidebar_header: bool,
    /// 侧边栏视图是否显示宿主 frame 控制入口。
    show_sidebar_frame_controls: bool,
    /// 宿主 frame 当前所在位置。
    sidebar_frame_placement: SidebarPlacement,
    /// 侧边栏视图下「历史记录」Popover 的开合状态。
    history_popover_open: bool,
    /// 当前驱动后端(默认 One_Agent)。
    backend: Backend,
    /// 可接入的外部 ACP agent 列表。
    acp_agents: Vec<AcpAgentEntry>,
    /// 每个 ACP agent 的一次性探测结果；缺失表示尚未探测。
    acp_probes: HashMap<SharedString, AcpAgentProbe>,
    /// 正在探测的 agent id；用于渲染「探测中」并防止重复启动。
    acp_probe_inflight: HashSet<SharedString>,
    /// 本地 Codex-style Skill 管理状态。
    skills: AgentSkillState,
    /// 已建立的 ACP 连接(backend == Acp 时存在)。
    acp: Option<AcpConnection>,
    /// 在飞轮次的 UI 侧记账：哪个内置会话、哪条连接事件流、哪一轮。
    ///
    /// 后台轮次方案后同一时刻可以有多条（各会话各自一轮）：按 turn_id 路由，
    /// 不再是单槽 `Option`。
    acp_turn_owners: Vec<AcpTurnOwner>,
    /// 正在放行的**历史回放**窗口（`session/load` 重放出来的那批 `session/update`）。
    acp_history_replay: Option<AcpHistoryReplay>,
    /// 历史回放期间事件流被挤掉（lagged）后的重载补全状态。
    ///
    /// `session/load` 的整段历史靠 broadcast 事件流回放，一旦订阅端 lagged，
    /// 这段历史就永久缺一块（后续不会再有事件来补）。补法是再 `load` 一次
    /// 同一条协议会话；只补一次，重放仍然 lagged 就认了（日志 warn），
    /// 避免在慢 agent 上无限重载。
    acp_replay_lagged: bool,
    acp_replay_retries: u8,
    /// 等待用户选择鉴权方式的 ACP 连接。
    acp_pending: Option<AcpPendingConnection>,
    /// 当前 pending 连接公布的鉴权方式。
    acp_auth_methods: Vec<String>,
    /// 当前选中的 ACP agent id(用于头部切换控件高亮)。
    current_acp_id: Option<SharedString>,
    /// 连接前选中的模型值；连接成功后自动补一次 `session/set_config_option`。
    ///
    /// 没有它，「先挑模型再连接」会变成点了没反应的假控件。
    pending_acp_model: Option<String>,
    /// 断线自动重连的尝试计数、待触发标记与代次。
    acp_reconnect: AcpReconnectState,
    /// 空闲 ACP 连接的健康检查任务；随连接建立与失效启停。
    _acp_health_task: Option<Task<()>>,
    /// 正在连接 ACP agent(拉起子进程中)。
    acp_connecting: bool,
    /// 正在连接的 ACP agent id,用于忽略已取消连接的异步回调。
    acp_connecting_id: Option<SharedString>,
    /// 当前 ACP connect/auth 操作发起时的会话，用于切换会话后仍按原队列恢复。
    acp_connect_origin_session: Option<String>,
    /// ACP connect/auth/new-session 的全局代次，用于隔离同一 agent 的迟到回调。
    acp_operation_generation: u64,
    /// ACP 新会话创建中的操作，或创建失败后等待重试的操作。
    acp_session_transition: Option<AcpSessionTransition>,
    /// 从零重开一条外部 agent 会话时，连接就绪后要 `load` 回来的协议会话 id。
    ///
    /// `resume` 只把上下文接上，不回放历史；agent 支持 `session/load` 时再主动
    /// load 一次，用户点开侧栏那行才能看到原来那段对话。只在下一次连接就绪时消费，
    /// 且 id 对得上才真去 load：用户中途改成连别的 agent 的话，这个值自然作废。
    acp_reopen_pending: Option<String>,
    /// 从 agent 拉到的历史会话列表（仅在 agent 声明支持 `session/list` 时有意义）。
    acp_sessions: Vec<AcpSessionSummary>,
    /// 拉列表/开会话失败的可见原因。清空表示上次是成功的——不拿空列表冒充失败。
    acp_sessions_error: Option<String>,
    /// 列表请求在飞。渲染层据此区分「还没拉」和「拉完了是空的」。
    acp_sessions_loading: bool,
    /// 列表请求的代次；迟到响应靠它判归属（不变量 5）。
    acp_sessions_generation: u64,
    /// 当前 agent 声明支持 `session/list`。连接被 take 走做请求时也保持稳定——
    /// 否则列表会在刷新过程中整个闪掉（不变量 11 说的是「能力缺失就不显示」，
    /// 不是「连接不在就不显示」）。
    acp_sessions_supported: bool,
    /// 当前 ACP 连接尚未响应的权限请求。
    pending_acp_permissions: HashMap<String, AcpPermissionEnvelope>,
    /// 当前 ACP 连接尚未回答的 agent 提问。同一时刻最多一条，新的会顶掉旧的。
    pending_acp_elicitations: HashMap<String, AcpElicitationEnvelope>,
    /// 安全确认模式下，实际 Public MCP 调用尚未响应的二次审批。
    pending_public_mcp_approvals: HashMap<String, AcpPublicMcpApprovalEnvelope>,
    /// 把匹配的 Public MCP 审批请求路由回当前 ACP 消息流。
    acp_public_mcp_approval_provider: Option<AcpPublicMcpApprovalProvider>,
    scroll_handle: ScrollHandle,
    auto_scroll: AutoScrollState,
    /// 当前工具执行模式。由 AI Chat 设置恢复，并用于后续提交。
    tool_execution_mode: ToolExecutionMode,
    /// 本轮的任务类型（问答 / 常规 / 计划）。与工具策略合成一个下拉。
    task_kind: TaskKind,
    /// 当前模型。切换时通过 runtime_factory 重建 Runtime,影响后续提交。
    selected_model: Option<ComposerModelOption>,
    model_options: Vec<ComposerModelOption>,
    tool_options: Vec<ComposerMenuOption>,
    runtime_factory: Option<AgentRuntimeFactory>,
    is_running: bool,
    /// 当前系统提示词追加指令。
    ///
    /// 来自全局设置或外部显式调用 [`Self::set_system_instruction`]（如终端侧边栏
    /// 的连接上下文指令），两者都追加在固定的系统提示词模板之后。
    system_instruction: Option<String>,
    /// 当前指令是否来自全局设置（而非外部显式注入）。
    ///
    /// 来自设置时，新会话/切换模型会跟随设置的最新值刷新（包括清空）；外部显式注入
    /// 的指令保持不变。默认来源为设置，即使当前设置为空。
    system_instruction_from_settings: bool,
    /// 代码块操作注册表。
    code_block_actions: CodeBlockActionRegistry,
    /// 可选的局部聊天主题。
    theme: Option<AgentChatTheme>,
    /// 是否侧边栏模式。
    _subscriptions: Vec<Subscription>,
    _event_task: Task<()>,
    /// 当前 ACP 连接的权限请求泵；切换连接时丢弃以隔离旧连接请求。
    _acp_permission_task: Option<Task<()>>,
    _acp_elicitation_task: Option<Task<()>>,
    /// 当前 ACP 连接的 Public MCP 二次审批泵。
    _acp_public_mcp_approval_task: Option<Task<()>>,
}

impl AgentChatView {
    pub fn refresh_models(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        selected_model_id: Option<SharedString>,
        runtime_factory: Option<AgentRuntimeFactory>,
        tool_registry: agent_runtime::ToolRegistry,
        cx: &mut Context<Self>,
    ) {
        self.runtime
            .services()
            .tools
            .replace_registry(tool_registry);
        let previous_id = self.selected_model.as_ref().map(|model| model.id.clone());
        let (selected, retained) = refreshed_model_selection(
            previous_id.as_ref(),
            selected_model_id.as_ref(),
            &model_options,
        );
        self.model_options = model_options;
        self.runtime_factory = runtime_factory;
        let model_options = self.model_options.clone();
        let tool_options = self.tool_options.clone();
        self.input.update(cx, |input, cx| {
            input.set_menu_options(model_options, tool_options, cx);
        });
        if let Some(retained) = retained {
            self.selected_model = Some(retained);
            self.sync_composer(cx);
            cx.notify();
            return;
        }
        if let Some(selected) = selected {
            self.select_model(
                selected.id.as_ref(),
                selected.provider_id.as_ref(),
                selected.model.as_ref(),
                cx,
            );
        }
    }

    /// 创建视图实体。
    pub fn view(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::view_with_config(
            AgentChatViewConfig::new(runtime, resources, mentions),
            window,
            cx,
        )
    }

    /// 从配置创建视图实体。
    pub fn view_with_config(
        config: AgentChatViewConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(config, window, cx))
    }

    pub(crate) fn new(
        config: AgentChatViewConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_model = selected_model_from_config(&config);
        let sidebar_mode = config.sidebar_mode;
        let show_sidebar_header = config.show_sidebar_header;
        let show_sidebar_frame_controls = config.show_sidebar_frame_controls;
        let sidebar_frame_placement = config.sidebar_frame_placement;
        let theme = config.theme;
        let acp_agents = config.acp_agents;
        let workspace_root = config.workspace_root.unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        });
        let resources = config.resources;
        let available_resources = config.available_resources;
        let mentions = config.mentions;
        let model_options = config.model_options;
        let binding = RuntimeBinding::new(
            config.runtime,
            resources.clone(),
            selected_model,
            config.runtime_factory,
        );
        let runtime = binding.runtime;
        let session_id = binding.session_id;
        let selected_model = binding.selected_model;
        let runtime_factory = binding.runtime_factory;
        let input = cx.new(|cx| {
            AgentInput::with_mentions(
                mentions,
                t!("AgentUi.input_placeholder").to_string(),
                window,
                cx,
            )
        });
        Self::register_approval_actions(cx);
        if let Some(theme) = theme.clone() {
            input.update(cx, |input, cx| input.set_theme(Some(theme), cx));
        }
        if sidebar_mode {
            input.update(cx, |input, cx| input.set_edge_to_edge(true, cx));
        }

        let stored_mode = AppSettings::current(cx).ai_chat.tool_execution_mode;
        let tool_execution_mode = runtime_tool_execution_mode(stored_mode);
        let task_kind = task_kind_from_settings(stored_mode);
        // 默认来源为全局设置：初始化时用当前设置作种子值，之后新会话/切换模型都会
        // 跟随设置刷新（包括配置被清空）。外部显式注入会通过 set_system_instruction
        // 覆盖来源并保持不变。
        let seed_system_instruction = AppSettings::current(cx)
            .ai_chat
            .effective_custom_system_prompt();
        let system_instruction_from_settings = true;
        let tool_options = default_execution_mode_options();

        let skills = AgentSkillState::load_for_workspace(&workspace_root);
        let init_ctx = build_composer_context(
            &resources,
            ExecutionSelection::new(task_kind, tool_execution_mode),
            selected_model.as_ref(),
            None,
            &[],
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
            &available_resources,
            skills.summary(),
            skills.items(),
            // 构造时还没有任何模型采样,计量自然为空;之后的 sync_composer 会带上。
            None,
        );
        let target_options: Vec<ComposerTarget> = resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        input.update(cx, |inp, cx| {
            inp.set_target_options(target_options, cx);
            inp.set_menu_options(model_options.clone(), tool_options.clone(), cx);
            inp.set_context(init_ctx, cx);
        });

        // 会话内搜索的查询框。
        //
        // **不**设 `clean_on_escape()`：那会让 `InputBaseState::escape` 走 `clean()` 分支
        // 提前 `return`（不再 `cx.propagate()`），findbar 容器上的 `escape` 关闭动作就永远收不到。
        // 保留查询、由 findbar 自己决定关闭后的语义。
        let findbar_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("AgentUi.findbar_placeholder").to_string())
        });

        // 会话切换器 overlay 的焦点句柄（打开时收焦点，让 tab/enter/escape 到位）。
        let session_switcher_focus = cx.focus_handle();

        let mut subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            cx.subscribe(
                &findbar_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    this.on_findbar_input_event(event, cx);
                },
            ),
        ];
        // 设置页改完 ACP agent 列表/启用状态/当前选中项后，活着的聊天面板要跟着走。
        if let Some(notifier) = acp_agent_config_notifier(cx) {
            subscriptions.push(cx.subscribe(
                &notifier,
                |this: &mut Self, _, _: &AcpAgentConfigEvent, cx| {
                    this.on_acp_agent_config_changed(cx);
                },
            ));
        }
        let event_task = Self::spawn_event_pump(runtime.subscribe(), None, cx);
        let current_session = session_id.to_string();
        let mut transcript = AgentTranscript::new();
        transcript.set_resource_context(&resources);

        // 活跃列表立即展示当前实时会话；持久化历史和后台任务随后统一合并。
        let live_sessions = vec![SessionSummary::new(
            current_session.clone(),
            current_agent_task_title(),
            now_secs(),
        )];
        let running_sessions = HashSet::new();
        let sessions = merge_live_session_summaries(
            persistence::list_summaries(cx),
            &live_sessions,
            &current_session,
            &running_sessions,
            false,
        );

        // 初始会话在构造时就归属当前工作区（与 `start_fresh_session` 同一约定）。
        // 漏了它，这条会话在第一次落盘之前没有任何归属可查，侧栏会先把它摆进
        // 「未分组」，落盘后才跳回分组——看起来就像「点一下分组自己变了」。
        let initial_session_roots = HashMap::from([(
            current_session.clone(),
            workspace_root.to_string_lossy().into_owned(),
        )]);

        let new_view = Self {
            runtime,
            session_id,
            resources,
            available_resources,
            transcript,
            input,
            pending_input_draft: None,
            session_draft_attachments: HashMap::new(),
            draft_session: None,
            session_navigation: SessionNavigation::default(),
            session_switcher: SessionSwitcherUi::new(),
            session_switcher_focus,
            sessions,
            live_sessions,
            session_transcripts: HashMap::new(),
            session_transcript_order: VecDeque::new(),
            subagent_details: HashMap::new(),
            subagent_detail_errors: HashMap::new(),
            subagent_loads: HashSet::new(),
            running_sessions,
            ignored_local_turns: HashSet::new(),
            local_operation_generations: HashMap::new(),
            closed_sessions: HashSet::new(),
            pending_submissions: PendingSubmissions::default(),
            current_session,
            sidebar_collapsed: false,
            sidebar_suppressed: false,
            workbench_toggles: None,
            composer_context_source: None,
            gated_submissions: VecDeque::new(),
            scroll: TranscriptScrollState::default(),
            expansion: ExpansionState::default(),
            turn_timings: TurnTimings::new(),
            restorable_turns: HashMap::new(),
            decision_details: None,
            search: TranscriptSearch::new(),
            search_revision: 0,
            findbar_open: false,
            findbar_input,
            show_archived: false,
            sidebar_mode,
            show_sidebar_header,
            show_sidebar_frame_controls,
            sidebar_frame_placement,
            history_popover_open: false,
            backend: Backend::Local,
            acp_agents,
            acp_probes: HashMap::new(),
            acp_probe_inflight: HashSet::new(),
            skills,
            acp: None,
            acp_turn_owners: Vec::new(),
            acp_history_replay: None,
            acp_replay_lagged: false,
            acp_replay_retries: 0,
            acp_pending: None,
            acp_auth_methods: Vec::new(),
            current_acp_id: None,
            pending_acp_model: None,
            acp_reconnect: AcpReconnectState::default(),
            _acp_health_task: None,
            acp_connecting: false,
            acp_connecting_id: None,
            acp_connect_origin_session: None,
            acp_operation_generation: 0,
            acp_session_transition: None,
            acp_reopen_pending: None,
            acp_sessions: Vec::new(),
            acp_sessions_error: None,
            acp_sessions_loading: false,
            acp_sessions_generation: 0,
            acp_sessions_supported: false,
            pending_acp_permissions: HashMap::new(),
            pending_acp_elicitations: HashMap::new(),
            pending_public_mcp_approvals: HashMap::new(),
            acp_public_mcp_approval_provider: None,
            scroll_handle: ScrollHandle::new(),
            auto_scroll: AutoScrollState::default(),
            tool_execution_mode,
            task_kind,
            selected_model,
            model_options,
            tool_options,
            runtime_factory,
            is_running: false,
            session_roots: initial_session_roots,
            system_instruction: seed_system_instruction,
            system_instruction_from_settings,
            theme,
            code_block_actions: CodeBlockActionRegistry::new(),
            _subscriptions: subscriptions,
            _event_task: event_task,
            _acp_permission_task: None,
            _acp_elicitation_task: None,
            _acp_public_mcp_approval_task: None,
            workspace_root,
        };
        if new_view.system_instruction.is_some() {
            new_view.apply_system_instruction_to_current_session();
        }
        new_view
    }

    fn register_approval_actions(cx: &mut Context<Self>) {
        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &ApproveToolCall, cx: &mut App| {
            let call_id = action.call_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_tool_action(call_id, true, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });

        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &RejectToolCall, cx: &mut App| {
            let call_id = action.call_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_tool_action(call_id, false, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });

        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &SelectAcpPermissionOption, cx: &mut App| {
            let request_id = action.request_id.clone();
            let option_id = action.option_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_acp_permission(request_id, option_id, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });

        // 工具卡片里的 diff 文件头只有一个「打开」按钮,它不知道审阅面板在哪,
        // 也不知道自己在哪一轮 —— 只把路径与自己的消息 id 发出来。这里用消息 id
        // 反查轮次，再往上转一手,由宿主决定落位与裁哪一轮的快照。
        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &OpenFileInReview, cx: &mut App| {
            let path = action.path.clone();
            let message_id = action.message_id.clone();
            let _ = view.update(cx, |this, cx| {
                let turn_id = this.turn_id_for_message(&message_id);
                let session_id = this.current_session.clone();
                cx.emit(AgentChatViewEvent::OpenFileInReview {
                    session_id,
                    path,
                    turn_id,
                });
            });
        });

        // 子代理卡片上的「查看推理过程」：卡片只知道子会话的协议 id 与标题，
        // 拉历史、开面板都由视图与宿主分别接手。
        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &OpenSubagentDetail, cx: &mut App| {
            let acp_session_id = action.acp_session_id.clone();
            let title = action.title.clone();
            let _ = view.update(cx, |this, cx| {
                this.open_subagent_detail(acp_session_id, title, cx);
            });
        });
    }

    fn resolve_pending_tool_action(
        &mut self,
        call_id: String,
        approved: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending_public_mcp_approvals.contains_key(&call_id) {
            self.resolve_pending_public_mcp_approval(&call_id, approved, cx);
            return true;
        }
        if !self.transcript.has_pending_tool_confirm(&call_id) {
            return false;
        }
        self.resolve_tool_call(call_id, approved, cx);
        true
    }

    /// 为这一次连接建好「agent 反问用户」的两条回程：权限确认与提问。
    fn start_acp_client_session(&mut self, cx: &mut Context<Self>) -> AcpClientProviders {
        self.reset_acp_client_session(cx);
        let (provider, receiver) = acp_permission_channel();
        let (elicitation_provider, elicitation_receiver) = acp_elicitation_channel();
        let (public_mcp_provider, public_mcp_receiver) = acp_public_mcp_approval_channel();
        self.acp_public_mcp_approval_provider = Some(public_mcp_provider);
        self._acp_permission_task = Some(Self::spawn_acp_permission_pump(receiver, cx));
        self._acp_elicitation_task =
            Some(Self::spawn_acp_elicitation_pump(elicitation_receiver, cx));
        self._acp_public_mcp_approval_task = Some(Self::spawn_public_mcp_approval_pump(
            public_mcp_receiver,
            cx,
        ));
        AcpClientProviders::new(provider, elicitation_provider)
    }

    fn spawn_acp_elicitation_pump(
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<AcpElicitationMessage>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| match message {
                    AcpElicitationMessage::Requested(envelope) => {
                        this.receive_acp_elicitation(envelope, cx)
                    }
                    AcpElicitationMessage::Expired { request_id } => {
                        this.expire_acp_elicitation(&request_id, cx)
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
    }

    fn receive_acp_elicitation(
        &mut self,
        envelope: AcpElicitationEnvelope,
        cx: &mut Context<Self>,
    ) {
        let request = envelope.request().clone();
        // 同一时刻只留一个问题：新的顶掉旧的（旧的按取消回给它的 agent）。
        self.cancel_pending_acp_elicitations(cx);
        let input = self.input.clone();
        input.update(cx, |input, cx| {
            input.set_pending_elicitation(Some(&request), cx);
        });
        self.pending_acp_elicitations
            .insert(request.request_id.clone(), envelope);
        self.request_scroll_to_bottom();
        self.auto_scroll.request_settle();
        cx.notify();
    }

    fn expire_acp_elicitation(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if self.pending_acp_elicitations.remove(request_id).is_none() {
            return;
        }
        // 超时是 agent 侧先放弃的：这里只把面板收起来，不必再回一次结果。
        self.clear_elicitation_panel(cx);
        cx.notify();
    }

    fn cancel_pending_acp_elicitations(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_acp_elicitations);
        if pending.is_empty() {
            return;
        }
        for (_request_id, envelope) in pending {
            envelope.resolve(AcpElicitationOutcome::Cancel);
        }
        self.clear_elicitation_panel(cx);
        cx.notify();
    }

    /// 把用户在 composer 面板里给出的答案送回 agent。
    fn resolve_pending_acp_elicitation(
        &mut self,
        outcome: AcpElicitationOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.pending_acp_elicitations.keys().next().cloned() else {
            return;
        };
        let Some(envelope) = self.pending_acp_elicitations.remove(&request_id) else {
            return;
        };
        // 送不回去说明通道已经关了（多半是超时）：结果丢弃即可，agent 那边不再等。
        let _ = envelope.resolve(outcome);
        self.clear_elicitation_panel(cx);
        cx.notify();
    }

    fn clear_elicitation_panel(&mut self, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_pending_elicitation(None, cx);
        });
    }

    fn spawn_public_mcp_approval_pump(
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<AcpPublicMcpApprovalMessage>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| match message {
                    AcpPublicMcpApprovalMessage::Requested(envelope) => {
                        this.receive_public_mcp_approval(envelope, cx)
                    }
                    AcpPublicMcpApprovalMessage::Expired { request_id } => {
                        this.expire_public_mcp_approval(&request_id, cx)
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
    }

    fn spawn_acp_permission_pump(
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<AcpPermissionMessage>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| match message {
                    AcpPermissionMessage::Requested(envelope) => {
                        this.receive_acp_permission(envelope, cx)
                    }
                    AcpPermissionMessage::Expired { request_id } => {
                        this.expire_acp_permission(&request_id, cx)
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
    }

    fn receive_acp_permission(&mut self, envelope: AcpPermissionEnvelope, cx: &mut Context<Self>) {
        let request = envelope.request().clone();
        if self
            .pending_acp_permissions
            .contains_key(&request.request_id)
        {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            return;
        }
        let requires_safety_confirmation = current_acp_tool_mode(cx)
            .unwrap_or(self.tool_execution_mode)
            == ToolExecutionMode::Manual;
        self.transcript
            .push_acp_permission(&request, requires_safety_confirmation);
        self.pending_acp_permissions
            .insert(request.request_id, envelope);
        self.request_scroll_to_bottom();
        self.auto_scroll.request_settle();
        cx.notify();
    }

    fn receive_public_mcp_approval(
        &mut self,
        envelope: AcpPublicMcpApprovalEnvelope,
        cx: &mut Context<Self>,
    ) {
        let request = envelope.request().clone();
        if self
            .pending_public_mcp_approvals
            .contains_key(&request.request_id)
        {
            envelope.resolve(AcpPublicMcpApprovalOutcome::Denied);
            return;
        }
        self.transcript.push_public_mcp_approval(&request);
        self.pending_public_mcp_approvals
            .insert(request.request_id.clone(), envelope);
        self.request_scroll_to_bottom();
        self.auto_scroll.request_settle();
        cx.notify();
    }

    fn resolve_pending_acp_permission(
        &mut self,
        request_id: String,
        option_id: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(envelope) = self.pending_acp_permissions.remove(&request_id) else {
            return false;
        };
        let Some(option) = envelope
            .request()
            .options
            .iter()
            .find(|option| option.option_id == option_id)
            .cloned()
        else {
            self.pending_acp_permissions.insert(request_id, envelope);
            return false;
        };
        let mut request = envelope.request().clone();
        if let Some(arguments) = self
            .transcript
            .tool_call_arguments(&request.tool_call_id)
            .cloned()
        {
            request.use_fallback_raw_input(arguments);
        }
        let grant = self
            .acp_public_mcp_approval_provider
            .clone()
            .and_then(|provider| acquire_acp_permission_grant(cx, &request, &option, provider));
        let delivered = envelope.resolve(AcpPermissionOutcome::Selected {
            option_id: option.option_id.clone(),
        });
        if delivered {
            if let Some(grant) = grant {
                grant.commit();
            }
            self.transcript.resolve_acp_permission(&request_id, &option);
        } else {
            self.transcript.cancel_acp_permission(&request_id);
        }
        cx.notify();
        true
    }

    fn resolve_pending_public_mcp_approval(
        &mut self,
        request_id: &str,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(envelope) = self.pending_public_mcp_approvals.remove(request_id) else {
            return;
        };
        let delivered = envelope.resolve(if approved {
            AcpPublicMcpApprovalOutcome::Approved
        } else {
            AcpPublicMcpApprovalOutcome::Denied
        });
        if delivered {
            self.transcript.resolve_tool_confirm(request_id, approved);
        } else {
            self.transcript.resolve_tool_confirm(request_id, false);
        }
        cx.notify();
    }

    fn expire_public_mcp_approval(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if let Some(envelope) = self.pending_public_mcp_approvals.remove(request_id) {
            envelope.resolve(AcpPublicMcpApprovalOutcome::Denied);
            self.transcript.resolve_tool_confirm(request_id, false);
            cx.notify();
        }
    }

    fn expire_acp_permission(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if let Some(envelope) = self.pending_acp_permissions.remove(request_id) {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            self.transcript.cancel_acp_permission(request_id);
            cx.notify();
        }
    }

    fn cancel_pending_acp_permissions(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_acp_permissions);
        for (request_id, envelope) in pending {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            self.transcript.cancel_acp_permission(&request_id);
        }
        cx.notify();
    }

    /// 收掉这一次连接留下的全部交互回程（权限 / 提问 / 二次审批）。
    fn reset_acp_client_session(&mut self, cx: &mut Context<Self>) {
        self.cancel_pending_acp_permissions(cx);
        self.cancel_pending_public_mcp_approvals(cx);
        self.cancel_pending_acp_elicitations(cx);
        // 连接已经收掉，回放窗口也没有意义了：留着它只会让人以为还有一批历史要落进来。
        self.acp_history_replay = None;
        self.acp_public_mcp_approval_provider = None;
        self._acp_permission_task = None;
        self._acp_elicitation_task = None;
        self._acp_public_mcp_approval_task = None;
    }

    fn cancel_pending_public_mcp_approvals(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_public_mcp_approvals);
        for (request_id, envelope) in pending {
            envelope.resolve(AcpPublicMcpApprovalOutcome::Denied);
            self.transcript.resolve_tool_confirm(&request_id, false);
        }
        cx.notify();
    }

    fn spawn_event_pump(
        mut rx: RuntimeEventReceiver,
        session_filter: Option<SessionId>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                // 先收事件，再把「丢过事件」这件事补掉。
                // 顺序不能反：本批里可能就带着某个会话的终态，先落地它，
                // 重同步就少一次误判。
                let batch = match rx.recv().await {
                    Ok(event) => {
                        collect_ready_runtime_events(&mut rx, event, session_filter.as_ref())
                    }
                    Err(RecvError::Lagged(skipped)) => RuntimeEventBatch {
                        events: Vec::new(),
                        skipped,
                    },
                    Err(RecvError::Closed) => break,
                };
                if !batch.events.is_empty()
                    && this
                        .update(cx, |this, cx| this.apply_runtime_events(batch.events, cx))
                        .is_err()
                {
                    break;
                }
                // 丢过事件就必须重同步：被挤掉的可能是某一轮的终态，
                // 迟到的补救没有任何其他触发点。见 `on_runtime_events_dropped`。
                if batch.skipped > 0
                    && this
                        .update(cx, |this, cx| {
                            this.on_runtime_events_dropped(batch.skipped, cx)
                        })
                        .is_err()
                {
                    break;
                }
            }
        })
    }

    fn apply_runtime_events(&mut self, events: Vec<RuntimeEvent>, cx: &mut Context<Self>) {
        for event in events {
            self.apply_runtime_event_with_deferred_budget(event, cx);
        }
        if self.backend == Backend::Acp {
            self.refresh_acp_model_options(cx);
        }
        self.transcript.flush_deferred_budget();
        for transcript in self.session_transcripts.values_mut() {
            transcript.flush_deferred_budget();
        }
    }

    /// 事件流被广播通道挤掉之后的补救。
    ///
    /// 为什么不能只记一条日志：丢的是**任意**事件，而清「正在响应」、推进排队
    /// 消息、落盘这一整串收尾动作，唯一的触发点就是终态事件
    /// （`TurnCompleted` / `TurnFailed` / `TurnCancelled`）。终态一旦被挤掉，
    /// 后面不会再有第二个事件来补齐——界面永久停在「正在响应」，排队的那条消息
    /// 永远发不出去，而且日志里看不出任何异常。所以丢过事件就必须重算一次。
    ///
    /// 判据取自**驱动侧**而不是视图侧：视图的 `running_sessions` 本来就是事件
    /// 推出来的，丢了事件它自己就是错的；本地看 runtime 会话的 `is_busy`，
    /// ACP 看连接自己的 `active_turn`，这两个都在产生事件的那一侧。
    fn on_runtime_events_dropped(&mut self, skipped: u64, cx: &mut Context<Self>) {
        tracing::warn!(
            skipped,
            backend = ?self.backend,
            running = self.running_sessions.len(),
            "runtime event stream lagged: dropped events cannot be replayed; \
             reconciling running state against the driver"
        );
        // 历史回放窗口开着时被挤掉，丢的就是回放本体：再 load 一次补全
        // （是否补、补几次由 `finish_acp_session_open` 判定）。
        if self.acp_history_replay.is_some() {
            self.acp_replay_lagged = true;
        }
        let stale: Vec<String> = self
            .running_sessions
            .iter()
            .filter(|session_uid| self.driver_reports_idle(session_uid))
            .cloned()
            .collect();
        if stale.is_empty() {
            return;
        }
        let backend = self.backend;
        let acp_phase = self.acp.as_ref().map(AcpConnection::phase);
        for session_uid in stale {
            tracing::warn!(
                %session_uid,
                ?backend,
                "terminating a turn whose terminal event was dropped"
            );
            if self
                .acp_turn_owners
                .iter()
                .any(|owner| owner.session_uid == session_uid)
            {
                self.cancel_pending_acp_permissions(cx);
                self.clear_acp_turn_owners_for_session(&session_uid);
            }
            if session_uid == self.current_session {
                self.auto_scroll.request_settle();
            }
            self.set_session_running(&session_uid, false, cx);
            // 终态该做的收尾一件都不能少：漏了落盘会丢历史，漏了推进排队会让
            // 用户排在下一条的消息永远发不出去——两个都是静默的。
            match backend {
                Backend::Local => {
                    self.persist_session(&session_uid, cx);
                    self.start_next_pending(&session_uid, cx);
                }
                Backend::Acp if acp_terminal_allows_queue_advance(acp_phase.as_ref()) => {
                    self.persist_acp_session(&session_uid, cx);
                    self.advance_acp_pending_after_terminal(&session_uid, cx);
                }
                Backend::Acp => {}
            }
        }
        self.trim_session_transcripts();
        cx.notify();
    }

    /// 驱动侧是否**确证**这个会话已经不在跑了。
    ///
    /// 只回答「确证空闲」：问不出来、或者轮次对不上，一律返回 `false`。
    /// 这里宁可漏收（界面上多转一会儿），也不能误清——误清会让真正还在跑的
    /// 那一轮凭空失去「正在响应」，用户会以为它结束了。
    fn driver_reports_idle(&self, session_uid: &str) -> bool {
        match self.backend {
            Backend::Local => {
                let session_id = SessionId::from_string(session_uid.to_string());
                // 会话不在 runtime 里就不下结论（可能是历史条目，也可能是别的
                // runtime 的会话）。在，就信它自己的 `is_busy`：runtime 是先登记
                // 活动轮次再发 `TurnStarted`、先清登记再发终态的，所以这个值在
                // 有事件可看的时候始终是准的。
                self.runtime
                    .session(&session_id)
                    .is_some_and(|session| !session.is_busy())
            }
            Backend::Acp => {
                let Some(acp) = self.acp.as_ref() else {
                    // 连接已经收掉：没有可问的对象，不猜。
                    return false;
                };
                // 按这个会话记在账上的轮次问连接：`try_prompt` 是先装 tracker 再发
                // `TurnStarted` 的，所以视图这边一看到轮次，连接那边就一定有对应的
                // tracker；查不到就是终态被丢了。
                let owned: Vec<&TurnId> = self
                    .acp_turn_owners
                    .iter()
                    .filter(|owner| owner.session_uid == session_uid)
                    .map(|owner| &owner.turn_id)
                    .collect();
                if owned.is_empty() {
                    // 账上没轮次、视图却还标着在跑：只认当前连接自己那条会话。
                    return acp.session_id().to_string() == session_uid;
                }
                owned.iter().all(|turn_id| !acp.turn_is_active(turn_id))
            }
        }
    }

    fn refresh_acp_model_options(&mut self, cx: &mut Context<Self>) {
        let Some(acp) = self.acp.as_ref() else {
            return;
        };
        let options = acp_model_options(&acp.state(), self.current_acp_id.as_ref());
        if options == self.model_options {
            return;
        }
        self.model_options = options.clone();
        self.input.update(cx, |input, cx| {
            input.set_menu_options(options, self.tool_options.clone(), cx);
        });
    }

    /// findbar 查询框的事件。
    ///
    /// `Enter` / `Shift+Enter` 走 `InputEvent::PressEnter`，不额外绑键——
    /// 输入组件已经替我们区分了「有/无修饰键」两种回车。
    fn on_findbar_input_event(&mut self, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                let query = self.findbar_input.read(cx).value().to_string();
                self.search.set_query(query, &self.transcript.messages);
                self.search_revision = self.transcript.revision();
                // 查询变化把游标复位到第一个命中：顺手把视图也拉过去。
                self.jump_to_current_search_hit();
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                self.step_search(!shift, cx);
            }
            _ => {}
        }
    }

    fn on_input_event(
        &mut self,
        input: &Entity<AgentInput>,
        event: &AgentInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.clone() {
            AgentInputEvent::Submit {
                text,
                mentions,
                images,
            } => {
                self.submit_with_window(text, mentions, images, window, cx);
            }
            AgentInputEvent::Stop => self.stop(cx),
            AgentInputEvent::SelectTarget { id } => {
                if !self.is_running {
                    self.select_target(&id, cx);
                }
            }
            AgentInputEvent::AddResourceToPool { id } => {
                if !self.is_running {
                    self.add_resource_to_pool(&id, cx);
                }
            }
            AgentInputEvent::RemoveResourceFromPool { id } => {
                if !self.is_running {
                    self.remove_resource_from_pool(&id, cx);
                }
            }
            AgentInputEvent::SelectResourceSource { id } => {
                if !self.is_running {
                    self.select_resource_source(&id, cx);
                }
            }
            AgentInputEvent::ToggleSkill { id } => {
                if !self.is_running {
                    self.toggle_skill(&id, cx);
                }
            }
            AgentInputEvent::ImportSkill { path } => {
                if !self.is_running {
                    self.import_skill(&path, cx);
                }
            }
            AgentInputEvent::PickScope { key: _ } => {}
            AgentInputEvent::SelectModel {
                id,
                provider_id,
                model,
            } => {
                if !self.is_running {
                    if self.backend == Backend::Acp {
                        self.select_acp_model(&id, &provider_id, &model, cx);
                    } else {
                        self.select_model(&id, &provider_id, &model, cx);
                    }
                }
            }
            AgentInputEvent::SelectExecutionMode { id } => self.select_execution_mode(&id, cx),
            AgentInputEvent::SelectWorkspace { path } => {
                if !self.is_running {
                    self.run_composer_action(window, cx, move |source, window, cx| {
                        (source.select_workspace)(std::path::Path::new(path.as_ref()), window, cx);
                    });
                }
            }
            AgentInputEvent::BrowseWorkspace => {
                if !self.is_running {
                    self.run_composer_action(window, cx, move |source, window, cx| {
                        (source.browse_workspace)(window, cx);
                    });
                }
            }
            AgentInputEvent::SelectBranch { name } => {
                if !self.is_running {
                    self.run_composer_action(window, cx, move |source, _window, cx| {
                        (source.select_branch)(&name, cx);
                    });
                }
            }
            AgentInputEvent::ToggleWorktree { enabled } => {
                if !self.is_running {
                    self.run_composer_action(window, cx, move |source, _window, cx| {
                        (source.toggle_worktree)(enabled, cx);
                    });
                }
            }
            AgentInputEvent::SubmitElicitation { content } => {
                self.resolve_pending_acp_elicitation(AcpElicitationOutcome::Accept(content), cx)
            }
            AgentInputEvent::DeclineElicitation => {
                self.resolve_pending_acp_elicitation(AcpElicitationOutcome::Decline, cx)
            }
            AgentInputEvent::CancelElicitation => {
                self.resolve_pending_acp_elicitation(AcpElicitationOutcome::Cancel, cx)
            }
            AgentInputEvent::SelectAgentBackend { id } => {
                if !self.is_running {
                    self.select_backend(id, cx);
                }
            }
            AgentInputEvent::RemoveQueued { index } => {
                let session_uid = self.current_session.clone();
                if self
                    .pending_submissions
                    .remove_at(&session_uid, index)
                    .is_some()
                {
                    self.sync_pending_preview(cx);
                    cx.notify();
                }
            }
            AgentInputEvent::EditQueued { index } => {
                let session_uid = self.current_session.clone();
                if let Some(submission) = self.pending_submissions.remove_at(&session_uid, index) {
                    self.sync_pending_preview(cx);
                    input.update(cx, |input, cx| {
                        input.restore_to_composer(&submission.text, submission.images, window, cx);
                    });
                    cx.notify();
                }
            }
        }
    }

    /// 把底栏动作转发给宿主注入的数据源。
    ///
    /// 先把 `Arc` 克隆出来再调用：宿主回调里会 `update` 本视图（重新取快照），
    /// 若直接借用 `self` 的字段就会自借用冲突。
    ///
    /// **必须延迟到本轮 `update` 收尾再执行**，不能原地调用。本函数处在
    /// `on_input_event` 里，而那是本视图订阅 `AgentInput` 的回调：GPUI 的
    /// `Context::subscribe_in` 会经 `invoke_subscriber_in` 先
    /// `subscriber.update(...)`，也就是说回调运行期间 `AgentChatView` 已被租借。
    /// 宿主动作末了会走 `shell.refresh_composer_context()`
    /// → `DefaultAgentChatPanel::refresh_composer_context` → `AgentChatView::update`，
    /// 同步调用就是 GPUI 的双重租借 panic：
    /// `cannot update ai_chat_view::agent_view::AgentChatView while it is already
    /// being updated`。而 macOS 的事件回调 `handle_view_event` 是 `extern "C"`，
    /// panic 无法 unwind，会升级成 `fatal runtime error: failed to initiate panic`
    /// 直接 abort 整个进程 —— 点一下底栏的分支 / Worktree / 工作区就是一次崩溃。
    ///
    /// `Window::defer` 把回调挂到 `Effect::Defer`，由最外层 `finish_update`
    /// 的 `flush_effects` 执行，那时所有租借都已释放，且仍在同一帧内。
    fn run_composer_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&ComposerContextSource, &mut Window, &mut gpui::App) + 'static,
    ) {
        let Some(source) = self.composer_context_source.clone() else {
            return;
        };
        window.defer(cx, move |window, cx| action(&source, window, cx));
    }

    /// 用户从输入框提交 —— 先过一道「worktree 待创建」的闸。
    ///
    /// 与 [`Self::submit`] 分开是因为闸门要 [`Window`]：worktree 创建失败时得把
    /// 文字与附件**还回**输入框并弹提示。而 `submit` 还被
    /// [`Self::send_external_message`]（侧栏 ask_ai，手上没有窗口）复用，不能
    /// 给它的签名加窗口。
    fn submit_with_window(
        &mut self,
        text: String,
        mentions: Vec<MentionItem>,
        images: Vec<crate::ImageAttachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_submission_for_worktree(&text, &mentions, &images, window, cx) {
            return;
        }
        self.submit(text, mentions, images, cx);
    }

    /// 这次提交是不是要先建 worktree；是则拦下并返回 `true`。
    ///
    /// 勾选 Worktree 只记意图，磁盘上还什么都没有（快照里的
    /// [`ComposerWorktreeState::pending`]）。到第一次发送才真正创建 ——
    /// 否则「只是勾一下」就会在磁盘上留一个 worktree、还把工作区根切走。
    ///
    /// 创建期间这条提交挂在 [`Self::gated_submissions`] 上等根落地：ACP 会话绑的是
    /// 工作区根，根没换过去就发等于跑在旧工作区上。成功由
    /// [`Self::set_workspace_root`] 接着发，失败走 [`Self::abort_gated_submission`]。
    fn defer_submission_for_worktree(
        &mut self,
        text: &str,
        mentions: &[MentionItem],
        images: &[crate::ImageAttachment],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(source) = self.composer_context_source.clone() else {
            return false;
        };
        // 只有第一条要开工：创建是实打实的 `git worktree add`，多调一次就多建一个
        // worktree。创建期间后来的提交只排队，等根切过去一起按序发。
        let preparing = !self.gated_submissions.is_empty();
        if !preparing && !(source.snapshot)(cx).worktree.pending {
            return false;
        }
        self.gated_submissions.push_back(PendingSubmission {
            text: text.to_string(),
            mentions: mentions.to_vec(),
            images: images.to_vec(),
        });
        if preparing {
            cx.notify();
            return true;
        }
        // 创建是后台 git 子进程（大仓库要数秒），等它跑完再发，别冻结输入框。
        let task = (source.prepare_worktree)(cx);
        cx.spawn_in(window, async move |this, cx| {
            // 成功不在这里做什么：换根是延后派发的，等 `set_workspace_root` 落地后
            // 由它接着发（在这儿发会用旧根）。
            if let Err(message) = task.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.abort_gated_submission(message, window, cx);
                });
            }
        })
        .detach();
        true
    }

    /// worktree 创建失败：中止这次发送，把内容还回输入框，并把原因交给宿主展示。
    ///
    /// 「中止」而不是「降级到主工作区继续发」：静默换个地方跑，用户会以为自己在
    /// 隔离的 worktree 里，实际改动落在主工作区上。勾选态**保留**，用户可以直接
    /// 再点一次发送重试，或取消勾选改用主工作区。
    fn abort_gated_submission(
        &mut self,
        message: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.gated_submissions.is_empty() {
            return;
        }
        // 排队期间攒下的几条一起还回去：输入框只有一个，按发送顺序用换行拼起来，
        // 附件合到一起。丢消息比这难看多了。
        let gated: Vec<PendingSubmission> = self.gated_submissions.drain(..).collect();
        let text = gated
            .iter()
            .map(|submission| submission.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let images = gated
            .into_iter()
            .flat_map(|submission| submission.images)
            .collect::<Vec<_>>();
        self.input.update(cx, |input, cx| {
            input.restore_to_composer(&text, images, window, cx);
        });
        if let Some(source) = self.composer_context_source.clone() {
            (source.report_worktree_failure)(&message, window, cx);
        }
        cx.notify();
    }

    /// 根已经切到新 worktree：把被拦下的提交按顺序发出去。
    ///
    /// 手上只有 [`Self::apply_workspace_root`] 一个调用点（它由
    /// [`Self::set_workspace_root`] 在「根落地」那一步调到）而不是 worktree 创建
    /// 完成的时刻 —— 创建完成时 `RootChanged` 还只是入队（`Effect::Emit` 延后派发），
    /// 视图这边的根、skills、ACP 连接都还是旧的。
    fn resume_gated_submission(&mut self, cx: &mut Context<Self>) {
        if self.gated_submissions.is_empty() {
            return;
        }
        // 先取干净再逐条发：第一条会正常启动，后面的在 `submit` 里看到本会话已在
        // 运行就自己排队 —— 复用既有队列，顺序不会乱。
        let gated: Vec<PendingSubmission> = self.gated_submissions.drain(..).collect();
        for submission in gated {
            self.submit(submission.text, submission.mentions, submission.images, cx);
        }
    }

    fn submit(
        &mut self,
        text: String,
        mentions: Vec<MentionItem>,
        images: Vec<crate::ImageAttachment>,
        cx: &mut Context<Self>,
    ) {
        // 新提交：跨轮的展开态与滚动跟随都要重置。
        self.expansion.clear();
        self.scroll.jump_to_tail();
        let session_uid = self.current_session.clone();
        // 文字与附件已从输入框消费（AgentInput::submit 里清空 / take），草稿同步
        // 作废——否则切走再切回，已发送的半句话和图片会「复活」回输入框。
        if let Some(session) = self
            .runtime
            .session(&SessionId::from_string(session_uid.clone()))
        {
            session.set_draft(None);
        }
        self.session_draft_attachments.remove(&session_uid);
        let submission = PendingSubmission {
            text,
            mentions,
            images,
        };
        // steer：ACP 会话正在跑一轮、队列为空时，新提交**并发发出去**而不是排队。
        // 轮次能跑几十分钟（实测 182 分钟），排队等不起；ACP 允许同会话并发
        // prompt（插话），agent 不支持时会以该轮 TurnFailed 浮出，旧轮不受影响。
        // 有排队时仍走 FIFO——插队会破坏已排消息的顺序。
        let steer_in_flight_turn = self.backend == Backend::Acp
            && self.running_sessions.contains(&session_uid)
            && self.acp_session_has_foreground_turn(&session_uid)
            && self.pending_submissions.len(&session_uid) == 0
            && !self.acp_connecting
            && self.acp_pending.is_none()
            && self.acp_session_transition_phase(&session_uid).is_none();
        if self.running_sessions.contains(&session_uid) && !steer_in_flight_turn {
            self.enqueue_submission(&session_uid, submission, cx);
            return;
        }
        if steer_in_flight_turn {
            self.push_user_to_session(
                &session_uid,
                &submission.text,
                submission.images.len(),
                &self.resources.clone(),
            );
            self.push_system_to_session(&session_uid, t!("AgentUi.acp_steer_sent").to_string());
            self.request_scroll_to_bottom();
            // 直接发；发不出去（连接问题等）就退回排队语义。
            if self.start_submission(&session_uid, &submission, cx) == SubmissionStart::RetryLater {
                self.enqueue_submission(&session_uid, submission, cx);
            }
            return;
        }

        // `NeedUserInput` / `TurnCancelled` 会暂停自动推进；用户再次显式提交时，
        // 仍先排到已有队列尾部，再从队首恢复，保证 FIFO 不被插队。
        if self.pending_submissions.len(&session_uid) > 0 {
            self.enqueue_submission(&session_uid, submission, cx);
            self.start_or_reconnect_current_pending(cx);
            return;
        }

        if self.backend == Backend::Acp
            && let Some(agent_id) = self
                .current_acp_id
                .clone()
                .filter(|id| self.can_retry_disconnected_acp(id))
        {
            self.enqueue_submission(&session_uid, submission, cx);
            self.select_acp_backend(agent_id, cx);
            return;
        }

        if self.start_submission(&session_uid, &submission, cx) == SubmissionStart::RetryLater {
            self.enqueue_submission(&session_uid, submission, cx);
        }
    }

    fn enqueue_submission(
        &mut self,
        session_uid: &str,
        submission: PendingSubmission,
        cx: &mut Context<Self>,
    ) {
        self.pending_submissions.enqueue(session_uid, submission);
        if session_uid == self.current_session {
            self.sync_pending_preview(cx);
        }
        cx.notify();
    }

    fn sync_pending_preview(&self, cx: &mut Context<Self>) {
        let previews = self
            .pending_submissions
            .items(&self.current_session)
            .into_iter()
            .map(|submission| {
                QueuedPromptPreview::new(submission.text.clone(), submission.images.len())
            })
            .collect::<Vec<_>>();
        let queue_blocked =
            !previews.is_empty() && !self.running_sessions.contains(&self.current_session);
        self.input.update(cx, |input, cx| {
            input.set_queued_submissions(previews, cx);
            input.set_pending_queue_blocked(queue_blocked, cx);
        });
    }

    fn start_next_pending(&mut self, session_uid: &str, cx: &mut Context<Self>) -> PendingAdvance {
        if self.running_sessions.contains(session_uid) {
            return PendingAdvance::Blocked;
        }

        loop {
            let Some(submission) = self.pending_submissions.front(session_uid).cloned() else {
                if session_uid == self.current_session {
                    self.sync_pending_preview(cx);
                }
                return PendingAdvance::Idle;
            };
            match self.start_submission(session_uid, &submission, cx) {
                SubmissionStart::Started => {
                    self.pending_submissions.pop_front(session_uid);
                    self.trim_session_transcripts();
                    if session_uid == self.current_session {
                        self.sync_pending_preview(cx);
                    }
                    return PendingAdvance::Started;
                }
                SubmissionStart::RetryLater => {
                    if session_uid == self.current_session {
                        self.sync_pending_preview(cx);
                    }
                    return PendingAdvance::Blocked;
                }
                SubmissionStart::Rejected => {
                    self.pending_submissions.pop_front(session_uid);
                    self.trim_session_transcripts();
                    if session_uid == self.current_session {
                        self.sync_pending_preview(cx);
                    }
                }
            }
        }
    }

    fn start_or_reconnect_current_pending(&mut self, cx: &mut Context<Self>) -> PendingAdvance {
        let session_uid = self.current_session.clone();
        if self.pending_submissions.len(&session_uid) == 0 {
            return PendingAdvance::Idle;
        }
        if let Some((operation, providers)) = self.prepare_current_pending_reconnect(cx) {
            self.spawn_acp_connect(operation, providers, cx);
            return PendingAdvance::Blocked;
        }
        self.start_next_pending(&session_uid, cx)
    }

    fn prepare_current_pending_reconnect(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(AcpConnectOperation, AcpClientProviders)> {
        let session_uid = self.current_session.clone();
        if self.pending_submissions.len(&session_uid) == 0 || self.backend != Backend::Acp {
            return None;
        }
        let agent_id = self
            .current_acp_id
            .clone()
            .filter(|id| self.can_retry_disconnected_acp(id))?;
        self.prepare_acp_connect(agent_id, cx)
    }

    fn advance_acp_pending_after_terminal(
        &mut self,
        owner_session_uid: &str,
        cx: &mut Context<Self>,
    ) {
        self.advance_acp_pending_after_origin(owner_session_uid, cx);
    }

    fn advance_acp_pending_after_origin(
        &mut self,
        origin_session_uid: &str,
        cx: &mut Context<Self>,
    ) {
        for session_uid in self.acp_pending_schedule_candidates(origin_session_uid) {
            if self.acp_session_has_foreground_turn(&session_uid) {
                break;
            }
            let advance = if session_uid == self.current_session {
                self.start_or_reconnect_current_pending(cx)
            } else {
                self.start_next_pending(&session_uid, cx)
            };
            if advance != PendingAdvance::Idle {
                break;
            }
        }
    }

    fn acp_pending_schedule_candidates(&self, origin_session_uid: &str) -> Vec<String> {
        let mut candidates = Vec::with_capacity(2);
        if !self.closed_sessions.contains(origin_session_uid) {
            candidates.push(origin_session_uid.to_string());
        }
        if origin_session_uid != self.current_session
            && !self.closed_sessions.contains(&self.current_session)
        {
            candidates.push(self.current_session.clone());
        }
        candidates
    }

    fn start_submission(
        &mut self,
        session_uid: &str,
        submission: &PendingSubmission,
        cx: &mut Context<Self>,
    ) -> SubmissionStart {
        match self.backend {
            Backend::Local => self.start_local_submission(session_uid, submission, cx),
            Backend::Acp => self.start_acp_submission(session_uid, submission, cx),
        }
    }

    fn start_acp_submission(
        &mut self,
        session_uid: &str,
        submission: &PendingSubmission,
        cx: &mut Context<Self>,
    ) -> SubmissionStart {
        self.sync_acp_tool_mode_from_provider(cx);
        let has_connection = self.acp.is_some();
        let session_transition_pending = self.acp_session_transition_phase(session_uid).is_some();
        if let Some(disposition) = submission_start_for_acp_availability(
            has_connection,
            self.acp_connecting,
            self.acp_pending.is_some(),
            session_transition_pending,
            self.current_acp_id.is_some(),
        ) {
            if disposition == SubmissionStart::Rejected {
                self.reject_submission_before_start(
                    session_uid,
                    t!("AgentUi.acp_not_connected").to_string(),
                    cx,
                );
            }
            return disposition;
        }
        let event_session_id = self
            .acp
            .as_ref()
            .expect("ACP availability allowed submission without a connection")
            .session_id();

        let input_images = match crate::input::prepare_input_images(&submission.images) {
            Ok(images) => images,
            Err(error) => {
                self.reject_submission_before_start(
                    session_uid,
                    t!("AgentUi.task_failed", error = error).to_string(),
                    cx,
                );
                return SubmissionStart::Rejected;
            }
        };
        let prompt = self
            .skills
            .selected_context()
            .wrap_user_prompt(&submission.text);
        let prompt = build_acp_prompt_blocks(prompt, &submission.mentions, &input_images);
        let turn_id = match self
            .acp
            .as_ref()
            .expect("ACP connection disappeared before prompt")
            .try_prompt(prompt)
        {
            Ok(turn_id) => turn_id,
            Err(error) => {
                if error == AcpPromptStartError::NotReady {
                    self.invalidate_unavailable_acp_connection(cx);
                }
                let disposition = submission_start_for_acp_error(error);
                if disposition == SubmissionStart::Rejected {
                    self.reject_submission_before_start(session_uid, error.to_string(), cx);
                }
                return disposition;
            }
        };

        self.push_user_to_session(
            session_uid,
            &submission.text,
            submission.images.len(),
            &self.resources.clone(),
        );
        if session_uid == self.current_session.as_str() {
            self.request_scroll_to_bottom();
        }
        self.set_session_running(session_uid, true, cx);
        self.acp_turn_owners.push(AcpTurnOwner {
            event_session_id,
            session_uid: session_uid.to_string(),
            turn_id,
            backgrounded: false,
            cancel_requested: false,
        });
        cx.notify();
        SubmissionStart::Started
    }

    fn start_local_submission(
        &mut self,
        session_uid: &str,
        submission: &PendingSubmission,
        cx: &mut Context<Self>,
    ) -> SubmissionStart {
        let session_id = SessionId::from_string(session_uid.to_string());
        let Some(session) = self.runtime.session(&session_id) else {
            self.reject_submission_before_start(
                session_uid,
                t!("AgentUi.run_failed", error = "session not found").to_string(),
                cx,
            );
            return SubmissionStart::Rejected;
        };

        let input_images = match crate::input::prepare_input_images(&submission.images) {
            Ok(images) => images,
            Err(error) => {
                self.reject_submission_before_start(
                    session_uid,
                    t!("AgentUi.task_failed", error = error).to_string(),
                    cx,
                );
                return SubmissionStart::Rejected;
            }
        };
        let mut resources = session.resources();
        if apply_mentioned_resources(
            &mut resources,
            &self.available_resources,
            &submission.mentions,
        ) {
            session.set_resources(resources.clone());
            if session_uid == self.current_session.as_str() {
                self.resources = resources.clone();
                self.sync_resource_targets(cx);
            }
        }
        self.push_user_to_session(
            session_uid,
            &submission.text,
            submission.images.len(),
            &resources,
        );
        self.upsert_live_summary(
            session_uid.to_string(),
            persistence_title_from_input(&submission.text),
            now_secs(),
        );
        self.reload_sessions(cx);
        if session_uid == self.current_session.as_str() {
            self.request_scroll_to_bottom();
        }
        let input = UserInput::new(submission.text.clone()).with_images(input_images);
        let operation_generation = self.next_local_operation_generation(session_uid);
        self.set_session_running(session_uid, true, cx);
        self.runtime
            .services()
            .set_agent_max_iterations(AppSettings::current(cx).ai_chat.max_iterations);

        let runtime = self.runtime.clone();
        let tool_mode = self.tool_execution_mode;
        // 「问答」模式下不向模型暴露工具，因此任务类型要跟着下拉一起下发。
        let task_kind = self.task_kind;
        let session_uid = session_uid.to_string();
        cx.spawn(async move |this, cx| {
            #[cfg(test)]
            let result = runtime
                .run_turn_blocking_with_tool_mode(&session_id, input, task_kind, tool_mode)
                .await;

            #[cfg(not(test))]
            let result = {
                let task = Tokio::spawn(cx, async move {
                    runtime
                        .run_turn_blocking_with_tool_mode(&session_id, input, task_kind, tool_mode)
                        .await
                });
                match task.await {
                    Ok(result) => result,
                    Err(err) => {
                        let _ = this.update(cx, |this, cx| {
                            this.finish_submission_without_event(
                                &session_uid,
                                operation_generation,
                                t!("AgentUi.task_failed", error = err).to_string(),
                                cx,
                            );
                        });
                        return;
                    }
                }
            };

            if let Err(err) = result {
                let _ = this.update(cx, |this, cx| {
                    this.finish_submission_without_event(
                        &session_uid,
                        operation_generation,
                        t!("AgentUi.run_failed", error = err).to_string(),
                        cx,
                    );
                });
            }
        })
        .detach();
        cx.notify();
        SubmissionStart::Started
    }

    fn reject_submission_before_start(
        &mut self,
        session_uid: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if self.closed_sessions.contains(session_uid) {
            return;
        }
        self.push_system_to_session(session_uid, message);
        if session_uid == self.current_session {
            self.request_scroll_to_bottom();
        }
        cx.notify();
    }

    fn finish_submission_without_event(
        &mut self,
        session_uid: &str,
        operation_generation: u64,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if self.closed_sessions.contains(session_uid)
            || !self.is_current_local_operation_generation(session_uid, operation_generation)
        {
            return;
        }
        self.push_system_to_session(session_uid, message);
        self.set_session_running(session_uid, false, cx);
        self.start_next_pending(session_uid, cx);
        if session_uid == self.current_session {
            self.request_scroll_to_bottom();
        }
        cx.notify();
    }

    fn push_user_to_session(
        &mut self,
        session_uid: &str,
        text: &str,
        image_count: usize,
        resources: &ResourceContext,
    ) {
        if self.closed_sessions.contains(session_uid) {
            return;
        }
        if session_uid == self.current_session {
            self.transcript.set_resource_context(resources);
            self.transcript.push_user(text, image_count);
            return;
        }
        {
            let transcript = self
                .session_transcripts
                .entry(session_uid.to_string())
                .or_insert_with(|| {
                    let mut transcript = AgentTranscript::new();
                    transcript.set_resource_context(resources);
                    transcript
                });
            transcript.set_resource_context(resources);
            transcript.push_user(text, image_count);
        }
        self.touch_session_transcript(session_uid);
        self.trim_session_transcripts();
    }

    fn request_acp_cancel_for_session(&mut self, session_uid: &str) -> bool {
        let has_connection = self.acp.is_some();
        let mut should_cancel = false;
        for owner in self.acp_turn_owners.iter_mut() {
            should_cancel |= owner.mark_cancel_requested(session_uid, has_connection);
        }
        if should_cancel {
            self.acp
                .as_ref()
                .expect("ACP owner marked for cancellation without a live connection")
                .cancel();
        }
        should_cancel
    }

    /// 当前 owner 的轮次 id（只在这个 owner 就属于 `session_uid` 时给）。
    fn acp_turn_owner_turn_id(&self, session_uid: &str) -> Option<TurnId> {
        self.acp_turn_owners
            .iter()
            .find(|owner| owner.session_uid == session_uid && !owner.backgrounded)
            .map(|owner| owner.turn_id.clone())
    }

    /// 这个会话是否有**前台**在飞轮次。
    fn acp_session_has_foreground_turn(&self, session_uid: &str) -> bool {
        self.acp_turn_owners
            .iter()
            .any(|owner| owner.session_uid == session_uid && !owner.backgrounded)
    }

    /// 收掉属于这个会话的全部 owner 记账（前台 + 后台）。
    fn clear_acp_turn_owners_for_session(&mut self, session_uid: &str) {
        self.acp_turn_owners
            .retain(|owner| owner.session_uid != session_uid);
    }

    /// 测试构建下不排取消兜底定时器，这个判归属的辅助函数只有生产路径在用。
    #[cfg_attr(test, allow(dead_code))]
    fn acp_turn_owner_matches(&self, session_uid: &str, turn_id: &TurnId) -> bool {
        self.acp_turn_owners.iter().any(|owner| {
            owner.session_uid == session_uid && &owner.turn_id == turn_id && !owner.backgrounded
        })
    }

    /// 本地把这一轮结掉：agent 没有回终态，界面不能一直转。
    ///
    /// 三件事必须一起做，少一件都还是「卡住」：
    /// - 清 owner —— 迟到的输出靠它被丢弃，不会算进下一轮；
    /// - 让连接层也放手 —— 否则它仍记着这一轮在跑，用户下一条消息会被拒成
    ///   「等这一轮结束」，而那一轮永远不结束；
    /// - 写一条说明 —— 用户点的是「停止」，得知道这一轮是按本地停止收的尾。
    fn settle_acp_turn_locally(&mut self, session_uid: &str, cx: &mut Context<Self>) {
        // 只动本会话的 owner:别的会话正在跑的轮次不归这次停止管。
        let owned_turns: Vec<TurnId> = self
            .acp_turn_owners
            .iter()
            .filter(|owner| owner.session_uid == session_uid)
            .map(|owner| owner.turn_id.clone())
            .collect();
        if !owned_turns.is_empty() {
            self.clear_acp_turn_owners_for_session(session_uid);
        }
        if let Some(acp) = self.acp.as_ref() {
            for turn_id in &owned_turns {
                acp.abandon_active_turn(turn_id);
            }
        }
        self.set_session_running(session_uid, false, cx);
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        self.push_system_to_session(
            session_uid,
            t!("AgentUi.acp_stop_settled_locally").to_string(),
        );
        cx.notify();
    }

    #[cfg(not(test))]
    fn spawn_acp_cancel_settle_after(
        &mut self,
        delay: std::time::Duration,
        session_uid: String,
        turn_id: TurnId,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                // 轮次还在跑才结账：能对上终态事件的话(owner 已被清)这里什么都不做。
                if !this.acp_turn_owner_matches(&session_uid, &turn_id) {
                    return;
                }
                this.settle_acp_turn_locally(&session_uid, cx);
            });
        })
        .detach();
    }

    /// 测试构建下不排定时器：到期后的行为由 [`Self::settle_acp_turn_locally`] 的
    /// 单测直接覆盖，不必真的等 8 秒。
    #[cfg(test)]
    fn spawn_acp_cancel_settle_after(
        &mut self,
        _delay: std::time::Duration,
        _session_uid: String,
        _turn_id: TurnId,
        _cx: &mut Context<Self>,
    ) {
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let session_uid = self.current_session.clone();
        if self.backend == Backend::Acp {
            let owns_current_turn = self.acp_session_has_foreground_turn(&session_uid);
            let cancel_outstanding = self
                .acp_turn_owners
                .iter()
                .any(|owner| owner.cancel_requested && !owner.backgrounded);
            let transition_phase = self.acp_session_transition_phase(&session_uid);
            let action = acp_stop_action(
                owns_current_turn,
                self.acp.is_some(),
                cancel_outstanding,
                self.acp_connecting,
                self.acp_pending.is_some(),
                transition_phase,
            );
            let stopped_session_uid = if action == AcpStopAction::ReturnToLocal {
                self.acp_connect_origin_session
                    .clone()
                    .unwrap_or_else(|| session_uid.clone())
            } else {
                session_uid.clone()
            };
            self.pending_submissions.clear_session(&stopped_session_uid);
            self.sync_pending_preview(cx);
            match action {
                AcpStopAction::CancelActivePrompt => {
                    let requested = self.request_acp_cancel_for_session(&session_uid);
                    // ACP cancellation is only a protocol notification. Keep the owner and
                    // running state until the matching prompt produces a real terminal event,
                    // otherwise late output can be attributed to a newer turn.
                    if requested && let Some(turn_id) = self.acp_turn_owner_turn_id(&session_uid) {
                        // 但等的上限必须有:agent 可以不回终态(provider 卡死时实测如此),
                        // 到时本地结账,界面不会永远停在「正在响应」。
                        self.spawn_acp_cancel_settle_after(
                            ACP_CANCEL_SETTLE_TIMEOUT,
                            session_uid.clone(),
                            turn_id,
                            cx,
                        );
                    }
                    cx.notify();
                    return;
                }
                AcpStopAction::ForceLocalStop => {
                    self.settle_acp_turn_locally(&session_uid, cx);
                    return;
                }
                AcpStopAction::ReturnToLocal => {
                    self.select_local_backend_for_session(&stopped_session_uid, cx);
                    return;
                }
                AcpStopAction::AbandonFailedTransition => {
                    self.invalidate_acp_operation();
                    self.clear_acp_turn_owners_for_session(&session_uid);
                    self.set_session_running(&session_uid, false, cx);
                    self.sync_pending_preview(cx);
                    self.sync_composer(cx);
                    cx.notify();
                    return;
                }
                AcpStopAction::ClearQueueOnly => {}
            }
            if owns_current_turn {
                self.clear_acp_turn_owners_for_session(&session_uid);
            }
            self.set_session_running(&session_uid, false, cx);
            cx.notify();
            return;
        }
        self.pending_submissions.clear_session(&session_uid);
        self.sync_pending_preview(cx);
        let session_id = SessionId::from_string(session_uid.clone());
        self.invalidate_local_operation_generation(&session_uid);
        let stopped_turn = self
            .runtime
            .session(&session_id)
            .and_then(|session| session.current_turn_id());
        if let Some(turn_id) = stopped_turn.as_ref() {
            self.ignored_local_turns.insert(turn_id.clone());
        }
        if let Err(err) = self.runtime.interrupt(&session_id) {
            if let Some(turn_id) = stopped_turn.as_ref() {
                self.ignored_local_turns.remove(turn_id);
            }
            self.push_system_to_session(
                &session_uid,
                t!("AgentUi.stop_failed", error = err).to_string(),
            );
        }
        self.set_session_running(&session_uid, false, cx);
        cx.notify();
    }

    fn approve_tool_call(
        &mut self,
        action: &ApproveToolCall,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_pending_tool_action(action.call_id.clone(), true, cx);
    }

    fn reject_tool_call(
        &mut self,
        action: &RejectToolCall,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_pending_tool_action(action.call_id.clone(), false, cx);
    }

    fn resolve_tool_call(&mut self, call_id: String, approved: bool, cx: &mut Context<Self>) {
        if self.backend != Backend::Local {
            return;
        }
        let runtime = self.runtime.clone();
        let session_id = self.session_id.clone();
        let session_uid = session_id.to_string();
        let Some(operation_generation) = self.current_local_operation_generation(&session_uid)
        else {
            return;
        };
        self.set_session_running(&session_uid, true, cx);
        let call_id = ToolCallId::from_string(call_id);
        cx.spawn(async move |this, cx| {
            #[cfg(test)]
            let result = if approved {
                runtime.approve_pending_tool(&session_id, &call_id).await
            } else {
                runtime.reject_pending_tool(&session_id, &call_id).await
            };

            #[cfg(not(test))]
            let result = {
                let task = Tokio::spawn(cx, async move {
                    if approved {
                        runtime.approve_pending_tool(&session_id, &call_id).await
                    } else {
                        runtime.reject_pending_tool(&session_id, &call_id).await
                    }
                });
                match task.await {
                    Ok(result) => result,
                    Err(err) => {
                        let _ = this.update(cx, |this, cx| {
                            this.finish_submission_without_event(
                                &session_uid,
                                operation_generation,
                                t!("AgentUi.approval_failed", error = err).to_string(),
                                cx,
                            );
                        });
                        return;
                    }
                }
            };

            if let Err(err) = result {
                let _ = this.update(cx, |this, cx| {
                    this.finish_submission_without_event(
                        &session_uid,
                        operation_generation,
                        t!("AgentUi.approval_failed", error = err).to_string(),
                        cx,
                    );
                });
            }
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    fn apply_runtime_event(&mut self, event: RuntimeEvent, cx: &mut Context<Self>) {
        self.apply_runtime_event_inner(event, false, cx);
    }

    fn apply_runtime_event_with_deferred_budget(
        &mut self,
        event: RuntimeEvent,
        cx: &mut Context<Self>,
    ) {
        self.apply_runtime_event_inner(event, true, cx);
    }

    fn apply_runtime_event_inner(
        &mut self,
        event: RuntimeEvent,
        defer_budget: bool,
        cx: &mut Context<Self>,
    ) {
        // 子代理详情会话：与当前对话无关的一段回放，走自己的通路。
        //
        // 必须在这里就分流，不能等到下面按会话归属：详情会话既不是 `current_session`，
        // 也不属于任何一轮，落进 `session_transcripts` 就会被当成「另一条真实会话」——
        // 于是进 LRU、被 `trim_session_transcripts` 挤掉，还可能被误当成一条要落盘的
        // 会话。它只是某张卡片的一次回放。
        if let Some(detail_uid) = subagent_detail_uid(&event) {
            self.apply_subagent_detail_event(&detail_uid, &event, defer_budget, cx);
            return;
        }
        let backend = self.backend;
        if backend == Backend::Local {
            let turn_id = runtime_event_turn_id(&event).clone();
            if self.ignored_local_turns.contains(&turn_id) {
                if matches!(
                    &event,
                    RuntimeEvent::TurnCompleted { .. }
                        | RuntimeEvent::TurnCancelled { .. }
                        | RuntimeEvent::TurnFailed { .. }
                ) {
                    self.ignored_local_turns.remove(&turn_id);
                }
                return;
            }
        }
        let session_uid = match backend {
            Backend::Local => event.session_id().to_string(),
            Backend::Acp => {
                // 按 turn id 在在飞表里找 owner；找不到再看是不是历史回放。
                // 多会话并发（后台轮次）时各轮的事件凭 turn id 各归各的会话。
                let event_turn = runtime_event_turn_id(&event);
                if let Some(owner) = self
                    .acp_turn_owners
                    .iter()
                    .find(|owner| &owner.turn_id == event_turn)
                {
                    if event.session_id() != &owner.event_session_id {
                        return;
                    }
                    owner.session_uid.clone()
                } else {
                    // 没有活动轮次：唯一合法的来源是 `session/load` 的历史回放。它没有轮次
                    // 可归，认不出回放就只能丢弃 —— 点开一条 ACP 历史会话会因此全屏空白。
                    let Some(replay) = self.acp_history_replay.as_ref() else {
                        return;
                    };
                    if !replay.accepts(&event) {
                        return;
                    }
                    replay.session_uid.clone()
                }
            }
        };
        let is_need_user_input = matches!(&event, RuntimeEvent::NeedUserInput { .. });
        let is_pending_tool_approval = matches!(
            &event,
            RuntimeEvent::NeedUserInput {
                pending_tool_call_id: Some(_),
                ..
            }
        );
        let is_cancelled = matches!(&event, RuntimeEvent::TurnCancelled { .. });
        let is_completed_or_failed = matches!(
            &event,
            RuntimeEvent::TurnCompleted { .. } | RuntimeEvent::TurnFailed { .. }
        );
        let is_real_terminal = is_cancelled || is_completed_or_failed;
        self.record_turn_timing(&event);
        match &event {
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                cx.emit(AgentChatViewEvent::TurnStarted {
                    session_id: session_uid.clone(),
                    turn_id: turn_id.to_string(),
                });
            }
            RuntimeEvent::TurnCompleted { turn_id, .. }
            | RuntimeEvent::TurnCancelled { turn_id, .. }
            | RuntimeEvent::TurnFailed { turn_id, .. } => {
                cx.emit(AgentChatViewEvent::TurnFinished {
                    session_id: session_uid.clone(),
                    turn_id: turn_id.to_string(),
                    success: matches!(&event, RuntimeEvent::TurnCompleted { .. }),
                });
            }
            _ => {}
        }
        let acp_terminal_phase = if backend == Backend::Acp && is_real_terminal {
            self.acp.as_ref().map(AcpConnection::phase)
        } else {
            None
        };
        if self.closed_sessions.contains(&session_uid) {
            if backend == Backend::Acp && is_real_terminal {
                self.cancel_pending_acp_permissions(cx);
                self.set_session_running(&session_uid, false, cx);
                self.clear_acp_turn_owners_for_session(&session_uid);
                self.trim_session_transcripts();
                if acp_connection_is_unavailable(acp_terminal_phase.as_ref()) {
                    self.invalidate_unavailable_acp_connection(cx);
                }
                if acp_terminal_allows_queue_advance(acp_terminal_phase.as_ref()) {
                    self.advance_acp_pending_after_terminal(&session_uid, cx);
                }
                cx.notify();
            }
            return;
        }
        let is_current_session = session_uid == self.current_session
            && !self
                .acp_turn_owners
                .iter()
                .any(|owner| owner.turn_id == *runtime_event_turn_id(&event) && owner.backgrounded);
        let clears_running = is_real_terminal
            || (backend == Backend::Local && is_need_user_input && !is_pending_tool_approval);
        let advances_queue = match backend {
            Backend::Local => is_completed_or_failed,
            Backend::Acp => {
                is_real_terminal && acp_terminal_allows_queue_advance(acp_terminal_phase.as_ref())
            }
        };
        let acp_error = match &event {
            RuntimeEvent::TurnFailed { reason, .. } if backend == Backend::Acp => {
                Some(self.acp_turn_error(reason))
            }
            _ => None,
        };
        let resources = self.resources.clone();
        let applied = if is_current_session {
            self.transcript.set_budget_deferred(defer_budget);
            if let Some(error) = acp_error.as_ref() {
                self.transcript.apply_acp_failure(&event, error)
            } else {
                self.transcript.apply(&event)
            }
        } else {
            let applied = {
                let transcript = self
                    .session_transcripts
                    .entry(session_uid.clone())
                    .or_insert_with(|| {
                        let mut transcript = AgentTranscript::new();
                        transcript.set_resource_context(&resources);
                        transcript
                    });
                transcript.set_budget_deferred(defer_budget);
                if let Some(error) = acp_error.as_ref() {
                    transcript.apply_acp_failure(&event, error)
                } else {
                    transcript.apply(&event)
                }
            };
            self.touch_session_transcript(&session_uid);
            self.trim_session_transcripts();
            applied
        };
        // 终态收尾放在转录去重**之前**。
        //
        // `applied` 说的是「这条事件的内容是重复的」，不是「这一轮不用收尾」。
        // 曾经的位置在 `if !applied { return; }` 之后，于是一条被判重的终态会把
        // 清 running / 清 owner 一起带走——界面就永久停在「正在响应」，而且因为
        // 转录里确实有内容，看起来完全不像卡住。收尾只依赖 `clears_running`。
        if clears_running {
            if backend == Backend::Acp && is_real_terminal {
                self.cancel_pending_acp_permissions(cx);
            }
            if is_current_session {
                self.auto_scroll.request_settle();
            }
            let turn_finished = runtime_event_turn_id(&event).clone();
            if backend == Backend::Acp && is_real_terminal {
                // 后台轮次与当前轮共享同一个内置 uid：后台轮次终态不能把新会话的
                // running 一起清掉——只有这条 uid 名下不再有任何在飞轮次时才清。
                let session_still_running = self.acp_turn_owners.iter().any(|owner| {
                    owner.session_uid == session_uid && owner.turn_id != turn_finished
                });
                if !session_still_running {
                    self.set_session_running(&session_uid, false, cx);
                }
                self.acp_turn_owners
                    .retain(|owner| owner.turn_id != turn_finished);
                self.trim_session_transcripts();
            } else {
                self.set_session_running(&session_uid, false, cx);
            }
        }
        if !applied {
            return;
        }
        if is_current_session {
            self.sync_composer(cx);
            // 跟随当前会话的流式输出 / 新卡片自动滚到底。
            self.request_scroll_to_bottom();
        }
        if backend == Backend::Acp
            && is_real_terminal
            && acp_connection_is_unavailable(acp_terminal_phase.as_ref())
        {
            self.invalidate_unavailable_acp_connection(cx);
        }
        // 本地轮次暂停等待用户输入时也要保存已产生的历史；ACP 会话由外部 agent 管理。
        if backend == Backend::Local && (clears_running || is_need_user_input) {
            self.persist_session(&session_uid, cx);
            self.reload_sessions(cx);
        }
        // ACP 会话的历史仍然归外部 agent，但侧栏得留着这一行，而且每轮结束
        // 都算一次「有过动静」，不然这段对话会永远沉在列表底部。
        if backend == Backend::Acp && is_real_terminal {
            self.persist_acp_session(&session_uid, cx);
        }
        if advances_queue {
            match backend {
                Backend::Local => {
                    self.start_next_pending(&session_uid, cx);
                }
                Backend::Acp => self.advance_acp_pending_after_terminal(&session_uid, cx),
            }
        }
        cx.notify();
    }

    /// 把一条子代理详情事件落进它自己的转录。
    ///
    /// 详情转录是**独立**的一份 [`AgentTranscript`]：它不参与当前会话判定、不碰
    /// running 状态、不落盘、也不进会话列表。唯一的外向动作是通知面板重绘。
    fn apply_subagent_detail_event(
        &mut self,
        detail_uid: &str,
        event: &RuntimeEvent,
        defer_budget: bool,
        cx: &mut Context<Self>,
    ) {
        // 加载成功就别再挂着上一次的失败文案。
        self.subagent_detail_errors.remove(detail_uid);
        let resources = self.resources.clone();
        let changed = {
            let transcript = self
                .subagent_details
                .entry(detail_uid.to_string())
                .or_insert_with(|| {
                    let mut transcript = AgentTranscript::new();
                    transcript.set_resource_context(&resources);
                    transcript
                });
            transcript.set_budget_deferred(defer_budget);
            let changed = transcript.apply(event);
            transcript.flush_deferred_budget();
            changed
        };
        if !changed {
            return;
        }
        cx.emit(AgentChatViewEvent::SubagentDetailUpdated {
            detail_session_id: detail_uid.to_string(),
        });
        cx.notify();
    }

    /// 用户点了子代理卡片上的「查看推理过程」。
    ///
    /// 交互是**懒加载**的：每 `session/load` 一次，agent 就把整段子代理历史重放一遍。
    /// 子代理动辄上千条 part，为每张卡片都在结束时自动拉一遍，代价和噪声都不划算；
    /// 用户点了才拉，才符合「我想看看这只子代理干了什么」的意图。
    pub(crate) fn open_subagent_detail(
        &mut self,
        acp_session_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AgentChatViewEvent::SubagentDetailRequested {
            acp_session_id: acp_session_id.clone(),
            title,
        });
        self.ensure_subagent_detail_loaded(acp_session_id, cx);
        cx.notify();
    }

    /// 拉取（且只拉一次）某条子会话的完整推理。
    fn ensure_subagent_detail_loaded(&mut self, acp_session_id: String, cx: &mut Context<Self>) {
        if self.backend != Backend::Acp {
            return;
        }
        // 插进去失败说明已经在拉、或已经拉过。状态会一直留着：转录本来就缓存着，
        // 再点一次只是切回那份缓存。
        if !self.subagent_loads.insert(acp_session_id.clone()) {
            return;
        }
        let Some(loader) = self.acp.as_ref().map(AcpConnection::detail_loader) else {
            // 连接已经不在（断线 / 刚切走）：把标记撤回，等重连后再点还有机会。
            self.subagent_loads.remove(&acp_session_id);
            return;
        };
        // 已经登记过的子会话不需要再 load：转录还在缓存里，重复 load 只会重放一遍。
        if self
            .acp
            .as_ref()
            .is_some_and(|acp| acp.is_detail_session_registered(&acp_session_id))
        {
            return;
        }
        let detail_uid = detail_session_id_for(&acp_session_id);
        let cwd = self.acp_detail_cwd();
        cx.spawn(async move |this, cx| {
            let result = loader
                .load(
                    agent_client_protocol::schema::v1::SessionId::new(acp_session_id),
                    cwd,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_subagent_detail_load(&detail_uid, result, cx);
            });
        })
        .detach();
    }

    fn finish_subagent_detail_load(
        &mut self,
        detail_uid: &str,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.subagent_detail_errors.remove(detail_uid);
            }
            Err(error) => {
                // 失败要让用户看见：卡片点得动、点完却一片空白，是最难排查的那种静默失败。
                let message =
                    t!("AgentUi.subagent_detail_failed", error = error.to_string()).to_string();
                tracing::warn!(%error, detail = %detail_uid, "failed to load subagent detail");
                self.subagent_detail_errors
                    .insert(detail_uid.to_string(), message);
            }
        }
        cx.emit(AgentChatViewEvent::SubagentDetailUpdated {
            detail_session_id: detail_uid.to_string(),
        });
        cx.notify();
    }

    /// 详情会话的转录消息；面板渲染用。
    pub(crate) fn subagent_detail_messages(
        &self,
        detail_session_id: &str,
    ) -> Option<&[ChatMessageUI]> {
        self.subagent_details
            .get(detail_session_id)
            .map(|transcript| transcript.messages.as_slice())
    }

    /// 详情会话加载失败的文案；`None` 表示没失败（或还没结果）。
    pub(crate) fn subagent_detail_error(&self, detail_session_id: &str) -> Option<&str> {
        self.subagent_detail_errors
            .get(detail_session_id)
            .map(String::as_str)
    }

    /// 详情会话的状态令牌：`(转录修订号, 是否有失败文案)`。
    ///
    /// 详情面板靠它判断「要不要重新取一份快照」。直接把消息克隆过去虽然简单，但面板
    /// 每次重绘都会克隆一遍整段转录（工具卡片里躺着几十 KB 的 JSON），光是鼠标划过
    /// 就够呛。
    pub(crate) fn subagent_detail_token(&self, detail_session_id: &str) -> (u64, bool) {
        (
            self.subagent_details
                .get(detail_session_id)
                .map(AgentTranscript::revision)
                .unwrap_or(0),
            self.subagent_detail_errors.contains_key(detail_session_id),
        )
    }

    /// `session/load` 详情会话要用的工作目录。
    ///
    /// agent 按 cwd 定位会话（OpenCode 的 `session.get` 带 `directory` 参数），拿一个
    /// 不相干的目录去 load 会直接查不到。所以优先用**当前 ACP 会话自己的** cwd；
    /// 拿不到（当前会话不在已拉取的列表里）才退回工作区根目录——那是新建会话时用的
    /// 目录，也是绝大多数情况下的正确答案。
    fn acp_detail_cwd(&self) -> std::path::PathBuf {
        self.acp
            .as_ref()
            .map(AcpConnection::protocol_session_id)
            .and_then(|protocol_id| {
                self.acp_sessions
                    .iter()
                    .find(|session| session.id == protocol_id)
                    .map(|session| session.cwd.clone())
            })
            .unwrap_or_else(|| self.workspace_root.clone())
    }

    fn invalidate_unavailable_acp_connection(&mut self, cx: &mut Context<Self>) -> bool {
        let phase = self.acp.as_ref().map(AcpConnection::phase);
        if !acp_connection_is_unavailable(phase.as_ref()) {
            return false;
        }
        self.reset_acp_client_session(cx);
        self.acp = None;
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
        true
    }

    /// 连接不可用时先丢弃旧连接，再按有界退避尝试自动重连。
    ///
    /// 测试构建下健康检查不启动，因此这里可能未被调用；决策本身由纯函数覆盖。
    #[cfg_attr(test, allow(dead_code))]
    fn handle_acp_unavailable(&mut self, cx: &mut Context<Self>) {
        if self.invalidate_unavailable_acp_connection(cx) {
            self.schedule_acp_auto_reconnect(cx);
        }
    }

    /// 当前连接是否已进入不可恢复的终态。
    #[cfg_attr(test, allow(dead_code))]
    fn acp_connection_unavailable(&self) -> bool {
        match self.acp.as_ref() {
            Some(connection) => acp_connection_is_unavailable(Some(&connection.phase())),
            None => false,
        }
    }

    /// 自动重连的目标 agent；用户主动切走（换后端 / 关闭会话）时为 `None`。
    #[cfg_attr(test, allow(dead_code))]
    fn acp_reconnect_target(&self) -> Option<SharedString> {
        if self.backend != Backend::Acp || self.closed_sessions.contains(&self.current_session) {
            return None;
        }
        self.current_acp_id.clone()
    }

    /// 取消待触发的自动重连，但不改尝试计数（新连接在飞时使用）。
    fn invalidate_acp_reconnect_schedule(&mut self) {
        self.acp_reconnect.scheduled = false;
        self.acp_reconnect.generation = self.acp_reconnect.generation.wrapping_add(1);
    }

    /// 用户主动离开 ACP：清空重连预算并停止健康检查。
    fn cancel_acp_auto_reconnect(&mut self) {
        self.invalidate_acp_reconnect_schedule();
        self.acp_reconnect.attempts = 0;
        self._acp_health_task = None;
    }

    /// 按尝试次数决定自动重连、放弃还是不动。
    #[cfg_attr(test, allow(dead_code))]
    fn schedule_acp_auto_reconnect(&mut self, cx: &mut Context<Self>) {
        let busy = self.acp_connecting
            || self.acp_pending.is_some()
            || self.acp_session_transition.is_some()
            || !self.acp_turn_owners.is_empty();
        let decision = acp_reconnect_decision(
            self.backend == Backend::Acp,
            true,
            busy,
            self.acp_reconnect_target().is_some(),
            self.acp_reconnect.attempts,
        );
        match decision {
            AcpReconnectDecision::Idle => {}
            AcpReconnectDecision::GiveUp => {
                self.acp_reconnect.scheduled = false;
                let agent_id = self
                    .current_acp_id
                    .clone()
                    .unwrap_or_else(|| SharedString::from("acp"));
                let agent_name = self.acp_agent_name(&agent_id);
                let error = AcpError::new(
                    AcpErrorKind::ConnectionClosed,
                    agent_id.to_string(),
                    agent_name.to_string(),
                    t!("AgentUi.acp_reconnect_exhausted").to_string(),
                )
                .with_recovery(AcpRecoveryAction::Retry);
                self.transcript.set_acp_error(&error);
                self.sync_composer(cx);
                cx.notify();
            }
            AcpReconnectDecision::Reconnect => {
                if self.acp_reconnect.scheduled {
                    return;
                }
                self.acp_reconnect.scheduled = true;
                let attempt = self.acp_reconnect.attempts;
                self.acp_reconnect.attempts = self.acp_reconnect.attempts.saturating_add(1);
                let generation = self.acp_reconnect.generation.wrapping_add(1);
                self.acp_reconnect.generation = generation;
                self.transcript.set_acp_status(
                    t!(
                        "AgentUi.acp_reconnecting",
                        attempt = attempt + 1,
                        max = ACP_RECONNECT_MAX_ATTEMPTS
                    )
                    .to_string(),
                );
                self.sync_composer(cx);
                cx.notify();
                self.spawn_acp_reconnect_after(acp_reconnect_delay(attempt), generation, cx);
            }
        }
    }

    /// 延迟后重试连接；代次不匹配说明已被更新的操作取代。
    #[cfg(not(test))]
    fn spawn_acp_reconnect_after(
        &mut self,
        delay: std::time::Duration,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                if this.acp_reconnect.generation != generation {
                    return;
                }
                this.acp_reconnect.scheduled = false;
                if this.acp_connection_unavailable() {
                    this.invalidate_unavailable_acp_connection(cx);
                }
                if this.acp.is_some() || this.acp_connecting || this.acp_pending.is_some() {
                    return;
                }
                let Some(agent_id) = this.acp_reconnect_target() else {
                    return;
                };
                // 不重置尝试计数：本次重试本身就是预算的一次消耗。
                if let Some((operation, permission_provider)) =
                    this.prepare_acp_connect(agent_id, cx)
                {
                    this.spawn_acp_connect(operation, permission_provider, cx);
                }
            });
        })
        .detach();
    }

    /// 测试构建下不拉子进程：自动重连只走纯决策，不产生真实连接。
    #[cfg(test)]
    fn spawn_acp_reconnect_after(
        &mut self,
        _delay: std::time::Duration,
        _generation: u64,
        _cx: &mut Context<Self>,
    ) {
    }

    /// 空闲连接的健康检查。agent 进程空闲退出不会产生轮次事件，只能靠轮询发现。
    #[cfg(not(test))]
    fn spawn_acp_health(&mut self, agent_id: SharedString, cx: &mut Context<Self>) {
        self._acp_health_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(ACP_HEALTH_INTERVAL).await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.backend != Backend::Acp
                            || this.current_acp_id.as_ref() != Some(&agent_id)
                        {
                            return false;
                        }
                        if this.acp_connection_unavailable() {
                            this.handle_acp_unavailable(cx);
                        }
                        // 连接已丢弃且没有在飞动作时收工；重连成功会再起一个新任务。
                        this.acp.is_some()
                            || this.acp_connecting
                            || this.acp_pending.is_some()
                            || this.acp_session_transition.is_some()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn acp_turn_error(&self, reason: &str) -> AcpError {
        let agent_id = self
            .current_acp_id
            .clone()
            .unwrap_or_else(|| SharedString::from("acp"));
        let agent_name = self.acp_agent_name(&agent_id);
        if reason.starts_with(t!("AgentUi.acp_empty_response_summary").as_ref()) {
            return AcpError::empty_response(agent_id.to_string(), agent_name.to_string());
        }
        AcpError::new(
            AcpErrorKind::PromptFailed,
            agent_id.to_string(),
            agent_name.to_string(),
            t!("AgentUi.acp_request_failed").to_string(),
        )
        .with_detail(reason)
        .with_recovery(AcpRecoveryAction::Retry)
    }

    /// 记录轮次起止时间，以及真实收尾结果。
    ///
    /// 只认真实事件时刻；折叠头里的「用时 Ns」不足以凭猜显示。收尾结果同理：
    /// 只有终态事件能判定失败/取消，绝不从系统提示文本推断。
    fn record_turn_timing(&mut self, event: &RuntimeEvent) {
        let now = unix_now_secs();
        match event {
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                self.turn_timings.note_started(turn_id.as_str(), now);
            }
            RuntimeEvent::TurnCompleted { turn_id, .. } => {
                self.turn_timings.note_finished(turn_id.as_str(), now);
                self.turn_timings
                    .note_outcome(turn_id.as_str(), TurnOutcome::Completed);
            }
            RuntimeEvent::TurnCancelled { turn_id, .. } => {
                self.turn_timings.note_finished(turn_id.as_str(), now);
                self.turn_timings
                    .note_outcome(turn_id.as_str(), TurnOutcome::Cancelled);
            }
            RuntimeEvent::TurnFailed { turn_id, .. } => {
                self.turn_timings.note_finished(turn_id.as_str(), now);
                self.turn_timings
                    .note_outcome(turn_id.as_str(), TurnOutcome::Failed);
            }
            _ => {}
        }
    }

    /// 某条消息属于哪一轮。
    ///
    /// 工具卡片的「在 Review 中打开」手里只有路径和自己的消息 id —— `ChatCard::render`
    /// 拿到的 `CardMessage` 里没有轮次视图。这里用消息 id 反查它落在哪一轮，
    /// 好过给 `render_one` / `render_card` 这条共用渲染路径再塞一个只为这一处
    /// 服务的参数（那要牵动七处调用点）。
    ///
    /// 消息自身的 `turn_id` 就是权威值（见 `ChatMessageUIGeneric::turn_id` 的注释：
    /// 不得用数组下标或渲染顺序推断）；历史恢复出来的消息没有它，返回 `None`。
    fn turn_id_for_message(&self, message_id: &str) -> Option<String> {
        self.transcript
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .and_then(|message| message.turn_id.clone())
    }

    /// 处理轮次视图抛出的交互请求。
    fn apply_message_list_action(&mut self, action: MessageListAction, cx: &mut Context<Self>) {
        match action {
            MessageListAction::SetProcessExpanded { key, expanded } => {
                self.expansion.set(key, expanded);
            }
            MessageListAction::ScrollToLatest => {
                self.request_scroll_to_bottom();
            }
            MessageListAction::RestoreTurn { turn_id } => {
                cx.emit(AgentChatViewEvent::RestoreTurn {
                    session_id: self.current_session.clone(),
                    turn_id,
                });
            }
            MessageListAction::OpenFileInReview { path, turn_id } => {
                cx.emit(AgentChatViewEvent::OpenFileInReview {
                    session_id: self.current_session.clone(),
                    path,
                    turn_id,
                });
            }
        }
        cx.notify();
    }

    fn request_scroll_to_bottom(&mut self) {
        self.scroll.jump_to_tail();
        self.auto_scroll.request();
        self.scroll_handle.scroll_to_bottom();
    }

    fn request_scroll_to_bottom_until_layout_settles(&mut self) {
        self.scroll.jump_to_tail();
        self.auto_scroll.request_settle();
        self.scroll_handle.scroll_to_bottom();
    }

    pub fn on_sidebar_shown(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar_mode {
            return;
        }
        self.request_scroll_to_bottom_until_layout_settles();
        cx.notify();
    }

    fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        let session_uid = self.current_session.clone();
        self.set_session_running(&session_uid, running, cx);
    }

    fn next_local_operation_generation(&mut self, session_uid: &str) -> u64 {
        let generation = self
            .local_operation_generations
            .entry(session_uid.to_string())
            .or_default();
        *generation = generation.wrapping_add(1);
        if *generation == 0 {
            *generation = 1;
        }
        *generation
    }

    fn invalidate_local_operation_generation(&mut self, session_uid: &str) {
        self.next_local_operation_generation(session_uid);
    }

    fn current_local_operation_generation(&self, session_uid: &str) -> Option<u64> {
        self.local_operation_generations.get(session_uid).copied()
    }

    fn is_current_local_operation_generation(&self, session_uid: &str, generation: u64) -> bool {
        self.current_local_operation_generation(session_uid) == Some(generation)
    }

    fn next_acp_operation(&mut self) -> AcpOperationToken {
        self.acp_session_transition = None;
        self.trim_session_transcripts();
        self.acp_operation_generation = self.acp_operation_generation.wrapping_add(1);
        if self.acp_operation_generation == 0 {
            self.acp_operation_generation = 1;
        }
        AcpOperationToken(self.acp_operation_generation)
    }

    fn invalidate_acp_operation(&mut self) {
        self.next_acp_operation();
    }

    fn is_current_acp_operation(&self, operation: AcpOperationToken) -> bool {
        self.acp_operation_generation == operation.0
    }

    fn is_current_acp_connection_operation(
        &self,
        operation: AcpOperationToken,
        agent_id: &SharedString,
        origin_session_uid: &str,
    ) -> bool {
        self.is_current_acp_operation(operation)
            && self.acp_connecting_id.as_ref() == Some(agent_id)
            && self.acp_connect_origin_session.as_deref() == Some(origin_session_uid)
    }

    fn is_current_acp_session_operation(
        &self,
        operation: AcpOperationToken,
        agent_id: &SharedString,
        session_uid: &str,
    ) -> bool {
        self.is_current_acp_operation(operation)
            && self.backend == Backend::Acp
            && self.current_acp_id.as_ref() == Some(agent_id)
            && self.current_session == session_uid
            && !self.closed_sessions.contains(session_uid)
    }

    fn begin_acp_session_transition(
        &mut self,
        agent_id: SharedString,
        session_uid: String,
    ) -> AcpOperationToken {
        self.begin_acp_session_transition_with_phase(
            agent_id,
            session_uid,
            AcpSessionTransitionPhase::Creating,
        )
    }

    /// 带阶段参数的版本。`Listing` 走这条:它同属「连接被 take 走了」的窗口。
    fn begin_acp_session_transition_with_phase(
        &mut self,
        agent_id: SharedString,
        session_uid: String,
        phase: AcpSessionTransitionPhase,
    ) -> AcpOperationToken {
        let operation = self.next_acp_operation();
        self.acp_session_transition = Some(AcpSessionTransition {
            operation,
            agent_id,
            session_uid,
            phase,
        });
        operation
    }

    fn is_current_acp_session_transition(
        &self,
        operation: AcpOperationToken,
        agent_id: &SharedString,
        session_uid: &str,
    ) -> bool {
        self.acp_session_transition
            .as_ref()
            .is_some_and(|transition| {
                transition.operation == operation
                    && &transition.agent_id == agent_id
                    && transition.session_uid == session_uid
            })
            && self.is_current_acp_session_operation(operation, agent_id, session_uid)
    }

    fn acp_session_transition_phase(&self, session_uid: &str) -> Option<AcpSessionTransitionPhase> {
        let transition = self.acp_session_transition.as_ref()?;
        self.is_current_acp_session_transition(
            transition.operation,
            &transition.agent_id,
            session_uid,
        )
        .then_some(transition.phase)
    }

    fn mark_acp_session_transition_failed(
        &mut self,
        operation: AcpOperationToken,
        agent_id: &SharedString,
        session_uid: &str,
    ) -> bool {
        if !self.is_current_acp_session_transition(operation, agent_id, session_uid) {
            return false;
        }
        let Some(transition) = self.acp_session_transition.as_mut() else {
            return false;
        };
        transition.phase = AcpSessionTransitionPhase::Failed;
        true
    }

    fn clear_acp_session_transition(&mut self, operation: AcpOperationToken) {
        if self
            .acp_session_transition
            .as_ref()
            .is_some_and(|transition| transition.operation == operation)
        {
            self.acp_session_transition = None;
            self.trim_session_transcripts();
        }
    }

    fn set_session_running(&mut self, session_uid: &str, running: bool, cx: &mut Context<Self>) {
        if running {
            self.running_sessions.insert(session_uid.to_string());
        } else {
            self.running_sessions.remove(session_uid);
            self.trim_session_transcripts();
        }
        if session_uid == self.current_session {
            self.is_running = running;
            self.input
                .update(cx, |input, cx| input.set_running(running, cx));
            self.sync_pending_preview(cx);
        }
        self.reload_sessions(cx);
    }

    fn push_system_to_session(&mut self, session_uid: &str, message: String) {
        if self.closed_sessions.contains(session_uid) {
            return;
        }
        if session_uid == self.current_session {
            self.transcript.push_system(message);
        } else {
            {
                self.session_transcripts
                    .entry(session_uid.to_string())
                    .or_default()
                    .push_system(message);
            }
            self.touch_session_transcript(session_uid);
            self.trim_session_transcripts();
        }
    }

    fn transcript_for_open_session_mut(
        &mut self,
        session_uid: &str,
    ) -> Option<&mut AgentTranscript> {
        if self.closed_sessions.contains(session_uid) {
            return None;
        }
        if session_uid == self.current_session {
            return Some(&mut self.transcript);
        }
        if !self.session_transcripts.contains_key(session_uid) {
            let mut transcript = AgentTranscript::new();
            transcript.set_resource_context(&self.resources);
            self.session_transcripts
                .insert(session_uid.to_string(), transcript);
        }
        self.touch_session_transcript(session_uid);
        self.trim_session_transcripts_to_preserving(
            MAX_CACHED_SESSION_TRANSCRIPTS,
            Some(session_uid),
        );
        self.session_transcripts.get_mut(session_uid)
    }

    /// 重建并把展示上下文推给输入框。
    fn sync_composer(&mut self, cx: &mut Context<Self>) {
        self.refresh_execution_mode_options();
        let model = self
            .acp
            .as_ref()
            .and_then(|acp| acp_model_option(&acp.state(), self.current_acp_id.as_ref()))
            .or_else(|| self.selected_model.clone());
        let mut ctx = build_composer_context(
            &self.resources,
            self.execution_selection(),
            model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
            self.local_context_tokens(),
        );
        apply_composer_snapshot(&mut ctx, self.composer_snapshot(cx));
        self.input.update(cx, |inp, cx| {
            inp.set_slash_commands(self.composer_slash_commands(), cx);
            inp.set_context(ctx, cx);
        });
    }

    /// 向宿主取一次上下文栏快照；未注入数据源时给默认值（底栏只有权限级别）。
    ///
    /// 宿主快照闭包拿的是 `&mut App`：它可能要读设置、走 git，允许内部缓存，
    /// 但绝不能反过来更新本视图（会在渲染路径里形成 `Entity::update` 重入）。
    fn composer_snapshot(&self, cx: &mut Context<Self>) -> ComposerContextSnapshot {
        match self.composer_context_source.as_ref() {
            Some(source) => (source.snapshot)(cx),
            None => ComposerContextSnapshot::default(),
        }
    }

    /// 当前本地会话的上下文占用(模型报告过计量才有;ACP 会话由 agent 上报,
    /// 走 [`AcpUsage`] 那条路,不经过这里)。
    fn local_context_tokens(&self) -> Option<u64> {
        if self.backend != Backend::Local {
            return None;
        }
        self.runtime
            .session(&SessionId::from_string(self.current_session.clone()))
            .and_then(|session| session.context_tokens())
    }

    pub fn set_workspace_root(&mut self, root: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.workspace_root == root {
            return;
        }
        self.apply_workspace_root(root.clone(), cx);
        AppSettings::update_and_save(cx, |settings| {
            settings.ai_chat.last_workspace_root = Some(root);
        });
    }

    /// 换根的后半程，也是被拦提交真正落地的地方：技能重载、ACP 连接作废、
    /// 底栏重算，最后把「等这次换根」的提交接着发出去。
    ///
    /// 与 [`Self::set_workspace_root`] 分开，是为了让它能在单测里被直接驱动 ——
    /// 后者会写用户真实的 `settings.json`，测试不能碰。**不要**为了省一次调用
    /// 把它合回去：合回去就等于这条路径没有测试覆盖（`RootChanged` 那条链上的
    /// 「提交接手」会静默失效，而测试照样全绿）。
    ///
    /// 用「根落地」而不是「worktree 创建完成」当触发点：创建完成时 `RootChanged`
    /// 才刚入队（`Effect::Emit` 延后派发），技能与 ACP 连接都还是旧根的。
    fn apply_workspace_root(&mut self, root: std::path::PathBuf, cx: &mut Context<Self>) {
        self.workspace_root = root.clone();
        // 项目级 skills 跟随工作区；选择集合只保留仍然存在的路径。
        self.skills.reload_for_workspace(&root);
        self.sync_session_skills();
        // 换工作区会丢弃旧连接；连接前的模型选择也不再适用于新 agent 会话。
        self.pending_acp_model = None;
        self.cancel_acp_auto_reconnect();
        self.acp = None;
        self.acp_pending = None;
        self.acp_connecting = false;
        self.clear_acp_sessions();
        self.sync_composer(cx);
        cx.notify();
        // 副作用：若用户在等待那一两秒里手动切了工作区，这条提交会落在新根上 ——
        // 概率极低，且用户本来就切到了那里，比把内容憋在 `gated_submissions` 里强。
        self.resume_gated_submission(cx);
    }

    pub fn restore_persisted_acp(&mut self, cx: &mut Context<Self>) {
        let Some(agent_id) = AppSettings::current(cx)
            .ai_chat
            .last_acp_agent_id
            .map(SharedString::from)
        else {
            return;
        };
        if self
            .acp_agents
            .iter()
            .any(|entry| entry.id == agent_id && entry.selectable())
        {
            self.select_acp_backend(agent_id, cx);
        }
    }

    pub(crate) fn restore_persisted_acp_model(&mut self, cx: &mut Context<Self>) {
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        self.apply_initial_acp_model(&agent_id, cx);
    }

    /// 连接建立后决定首个模型：优先连接前的选择，其次上次为该 agent 保存的模型。
    fn apply_initial_acp_model(&mut self, agent_id: &SharedString, cx: &mut Context<Self>) {
        let desired = self.pending_acp_model.take().or_else(|| {
            AppSettings::current(cx)
                .ai_chat
                .acp_models
                .get(agent_id.as_ref())
                .cloned()
        });
        let Some(model) = desired else {
            return;
        };
        let Some(option) = self
            .model_options
            .iter()
            .find(|option| option.model.as_ref() == model)
            .cloned()
        else {
            return;
        };
        self.select_acp_model(
            option.id.as_ref(),
            option.provider_id.as_ref(),
            option.model.as_ref(),
            cx,
        );
    }

    /// 连接建立前，用探测到的模型填充选择器。
    ///
    /// 仅在「当前就是该 ACP agent 且尚未连接」时生效；真正连接后由
    /// [`acp_model_options`] 覆盖成带配置项 id 的权威列表。
    fn apply_probe_model_options(&mut self, agent_id: &SharedString, cx: &mut Context<Self>) {
        if self.acp.is_some() {
            return;
        }
        let Some(probe) = self.acp_probes.get(agent_id) else {
            return;
        };
        if probe.models.is_empty() {
            return;
        }
        let options = acp_model_options_from_probe(agent_id, &probe.models);
        self.selected_model = self
            .pending_acp_model
            .as_deref()
            .and_then(|model| {
                options
                    .iter()
                    .find(|option| option.model.as_ref() == model)
                    .cloned()
            })
            .or_else(|| {
                self.selected_model
                    .clone()
                    .filter(|selected| options.iter().any(|option| option.id == selected.id))
            })
            .or_else(|| options.first().cloned());
        self.model_options = options.clone();
        self.input.update(cx, |input, cx| {
            input.set_menu_options(options, self.tool_options.clone(), cx);
        });
        self.sync_composer(cx);
    }

    /// 当前会话可补全的 `/` 命令。
    ///
    /// 命令只来自 Acp agent 的 `available_commands`：内置 agent 没有这套协议，返回空向量，
    /// 输入框里打 `/` 就不弹菜单（不造假的命令表）。
    fn composer_slash_commands(&self) -> Vec<SlashCommandItem> {
        if self.backend != Backend::Acp {
            return Vec::new();
        }
        self.acp
            .as_ref()
            .map(|acp| {
                let state = acp.state();
                slash_commands_from_acp_state(&state)
            })
            .unwrap_or_default()
    }

    fn refresh_acp_agents(&mut self, cx: &mut Context<Self>) {
        match build_acp_agent_entries(cx) {
            Ok(agents) => self.refresh_acp_agents_from(agents, cx),
            Err(error) => {
                tracing::warn!(%error, "Failed to refresh ACP agent configs");
            }
        }
    }

    fn refresh_acp_agents_from(&mut self, agents: Vec<AcpAgentEntry>, cx: &mut Context<Self>) {
        self.acp_agents = agents;
        #[cfg(not(test))]
        self.spawn_acp_probes(cx);
        self.sync_composer(cx);
        cx.notify();
    }

    /// 设置页改了 ACP agent 配置（列表/启用状态/当前选中项）后的对齐动作。
    fn on_acp_agent_config_changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_acp_agents(cx);
        self.sync_active_acp_agent_from_settings(cx);
    }

    /// 把当前后端对齐到设置页选中的 agent。
    ///
    /// 正在跑一轮或正在连接时不动连接：切后端会掐掉当前回合，等这一轮结束或面板
    /// 重新挂载时再对齐。
    fn sync_active_acp_agent_from_settings(&mut self, cx: &mut Context<Self>) {
        if self.is_running || self.acp_connecting {
            return;
        }
        let Some(target) = AppSettings::current(cx)
            .ai_chat
            .last_acp_agent_id
            .map(SharedString::from)
        else {
            return;
        };
        if self.backend == Backend::Acp && self.current_acp_id.as_ref() == Some(&target) {
            return;
        }
        if !self
            .acp_agents
            .iter()
            .any(|entry| entry.id == target && entry.selectable())
        {
            return;
        }
        self.select_acp_backend(target, cx);
    }

    /// 为尚未探测的 ACP agent 各起一次后台探测。
    ///
    /// 探测会真的拉起子进程，因此：
    /// - 只处理**已启用**的 agent（停用的不启动任何进程）；
    /// - 先查落盘缓存：启动指纹一致就直接复用上次结论；
    /// - 剩下没缓存的才真正后台探测，结果写回缓存供下次复用。
    #[cfg(not(test))]
    fn spawn_acp_probes(&mut self, cx: &mut Context<Self>) {
        let mut targets: Vec<(SharedString, AcpAgentConfig, String)> = Vec::new();
        {
            let cache = acp_probe_cache(cx);
            for entry in self.acp_agents.iter().filter(|entry| entry.enabled) {
                let Some(config) = entry.config.clone() else {
                    continue;
                };
                if self.acp_probes.contains_key(&entry.id)
                    || self.acp_probe_inflight.contains(&entry.id)
                {
                    continue;
                }
                let fingerprint = probe_fingerprint(&config);
                if let Some(record) = cache.get(entry.id.as_ref(), &fingerprint) {
                    self.acp_probes.insert(entry.id.clone(), record.probe);
                    continue;
                }
                targets.push((entry.id.clone(), config, fingerprint));
            }
        }
        if targets.is_empty() {
            return;
        }
        let handle = Tokio::handle(cx);
        for (id, _, _) in &targets {
            self.acp_probe_inflight.insert(id.clone());
        }
        // 串行探测，且每个探测在 GPUI 后台线程上用 `Handle::block_on` 跑：
        // 一次只拉起一个 CLI，`EnterGuard` 也不会落到 GPUI 主线程上交错。
        let probe_task = cx.background_spawn(async move {
            let mut results = Vec::with_capacity(targets.len());
            for (id, config, fingerprint) in targets {
                let probe = crate::acp::probe_agent_blocking(&config, handle.clone());
                results.push((id, probe, fingerprint));
            }
            results
        });
        cx.spawn(async move |this, cx| {
            for (id, probe, fingerprint) in probe_task.await {
                if !probe.identified() {
                    tracing::warn!(
                        agent = %id,
                        error = probe.error.as_deref().unwrap_or("unknown"),
                        "ACP agent probe failed"
                    );
                }
                let alive = this
                    .update(cx, |this, cx| {
                        this.acp_probe_inflight.remove(&id);
                        acp_probe_cache(cx).store(
                            id.as_ref(),
                            AcpProbeRecord::new(fingerprint.clone(), probe.clone()),
                        );
                        this.acp_probes.insert(id.clone(), probe);
                        if this.backend == Backend::Acp && this.current_acp_id.as_ref() == Some(&id)
                        {
                            this.apply_probe_model_options(&id, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// 当前 agent 列表对应的探测状态文案，供切换器直接展示。
    fn agent_probe_statuses(&self) -> HashMap<SharedString, SharedString> {
        self.acp_agents
            .iter()
            .filter_map(|entry| {
                let status = self.acp_probe_status(&entry.id)?;
                Some((entry.id.clone(), status))
            })
            .collect()
    }

    fn acp_probe_status(&self, id: &SharedString) -> Option<SharedString> {
        if self.acp_probe_inflight.contains(id) {
            return Some(SharedString::from(t!("AgentUi.agent_probing").to_string()));
        }
        Some(probe_status_label(self.acp_probes.get(id)?))
    }

    fn agent_switcher_options(&self) -> Vec<ComposerAgentOption> {
        composer_agent_options_with_status(
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            &self.agent_probe_statuses(),
        )
    }

    /// 在目标下拉中选中某个资源:设为当前目标并同步给会话与输入框。
    fn select_target(&mut self, id: &str, cx: &mut Context<Self>) {
        let rid = ResourceId::new(id.to_string());
        if self.resources.get(&rid).is_none() {
            return;
        }
        self.resources.current = Some(rid);
        self.sync_session_resources();
        self.sync_resource_targets(cx);
        cx.notify();
    }

    fn add_resource_to_pool(&mut self, id: &str, cx: &mut Context<Self>) {
        if add_resource_to_pool(&mut self.resources, &self.available_resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn remove_resource_from_pool(&mut self, id: &str, cx: &mut Context<Self>) {
        if remove_resource_from_pool(&mut self.resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn toggle_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.skills.toggle(id) {
            self.sync_session_skills();
            self.sync_composer(cx);
            cx.notify();
        }
    }

    fn import_skill(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        match self.skills.import_skill(path) {
            Ok(()) => {
                self.sync_session_skills();
                self.sync_composer(cx);
            }
            Err(error) => {
                self.transcript
                    .push_system(t!("AgentUi.import_skill_failed", error = error).to_string());
            }
        }
        cx.notify();
    }

    fn sync_session_skills(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_skills(self.skills.selected_context());
        }
    }

    fn select_resource_source(&mut self, id: &str, cx: &mut Context<Self>) {
        if apply_resource_source(&mut self.resources, &self.available_resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn select_model(&mut self, id: &str, provider_id: &str, model: &str, cx: &mut Context<Self>) {
        // 切换模型会替换整个 Runtime；任一后台会话仍在运行时必须保留旧 Runtime。
        if !self.running_sessions.is_empty() || !self.pending_submissions.is_empty() {
            return;
        }
        let Some(opt) = self.model_options.iter().find(|o| {
            o.id.as_ref() == id
                && o.provider_id.as_ref() == provider_id
                && o.model.as_ref() == model
        }) else {
            return;
        };
        let opt = opt.clone();
        let previous_session = self.session_id.clone();
        let mut binding = RuntimeBinding {
            runtime: self.runtime.clone(),
            session_id: self.session_id.clone(),
            selected_model: self.selected_model.clone(),
            runtime_factory: self.runtime_factory.clone(),
        };
        match binding.switch_model(&opt, &self.resources) {
            Ok(true) => {
                // session id 与切换前一致 ⇒ 历史已随会话搬进新 Runtime：这仍是
                // 同一个会话，转录、侧栏摘要、会话缓存都不能重置，只换了模型。
                let carried_session = binding.session_id == previous_session;
                // Runtime 与新会话已成功构造；提交切换前先保存旧会话。
                self.persist_current(cx);
                self.runtime = binding.runtime;
                self.session_id = binding.session_id;
                if self.system_instruction_from_settings {
                    self.system_instruction = AppSettings::current(cx)
                        .ai_chat
                        .effective_custom_system_prompt();
                }
                self.apply_system_instruction_to_current_session();
                self.sync_session_skills();
                self.sync_session_resources();
                self.selected_model = binding.selected_model;
                self.current_session = self.session_id.to_string();
                self.pending_submissions = PendingSubmissions::default();
                self.acp_turn_owners.clear();
                if !carried_session {
                    // 没有可延续的会话（当前会话已不在 Runtime 中），只能按新会话
                    // 处理：清转录与缓存，并在侧栏用「provider / model」占位。
                    self.clear_cached_session_transcripts();
                    self.live_sessions.clear();
                    self.ignored_local_turns.clear();
                    self.closed_sessions.clear();
                    self.upsert_live_summary(
                        self.current_session.clone(),
                        format!("{} / {}", opt.provider_label, opt.model),
                        now_secs(),
                    );
                    self.transcript.clear();
                    self.transcript.set_resource_context(&self.resources);
                }
                self._event_task = Self::spawn_event_pump(self.runtime.subscribe(), None, cx);
                self.reload_sessions(cx);
                self.sync_pending_preview(cx);
            }
            Ok(false)
                if self
                    .selected_model
                    .as_ref()
                    .is_some_and(|current| current.id == opt.id) =>
            {
                self.selected_model = Some(opt);
            }
            Ok(false) => return,
            Err(error) => {
                self.transcript.push_system(
                    t!("AgentUi.model_switch_failed", error = error.to_string()).to_string(),
                );
                self.request_scroll_to_bottom();
                self.sync_composer(cx);
                cx.notify();
                return;
            }
        }
        self.sync_composer(cx);
        cx.notify();
    }

    fn select_acp_model(
        &mut self,
        id: &str,
        provider_id: &str,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(option) = self.model_options.iter().find(|option| {
            option.id.as_ref() == id
                && option.provider_id.as_ref() == provider_id
                && option.model.as_ref() == model
        }) else {
            return;
        };
        let option = option.clone();
        // 尚未连接：先记住选择，等 `activate_acp` 拿到真实配置项后再下发。
        let Some(acp) = self.acp.take() else {
            self.pending_acp_model = Some(option.model.to_string());
            self.selected_model = Some(option);
            self.sync_composer(cx);
            cx.notify();
            return;
        };
        let Some(config) = acp.state().current_model_config().cloned() else {
            self.acp = Some(acp);
            return;
        };
        let value = agent_client_protocol::schema::v1::SessionConfigValueId::new(model);
        let config_id = config.id.clone();
        let provider_id = provider_id.to_string();
        let model = model.to_string();
        cx.spawn(async move |this, cx| {
            let result = acp.set_model(config_id, value).await;
            let _ = this.update(cx, |this, cx| {
                this.acp = Some(acp);
                match result {
                    Ok(()) => {
                        this.selected_model = Some(option);
                        AppSettings::update_and_save(cx, |settings| {
                            settings
                                .ai_chat
                                .acp_models
                                .insert(provider_id.to_string(), model.to_string());
                        });
                        this.sync_composer(cx);
                    }
                    Err(error) => this
                        .transcript
                        .push_system(format!("ACP model switch failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// composer 的执行模式选择：任务类型 + 工具策略。
    fn execution_selection(&self) -> ExecutionSelection {
        ExecutionSelection::new(self.task_kind, self.tool_execution_mode)
    }

    /// 按当前后端刷新执行模式下拉项。
    ///
    /// ACP 已连接且 agent 声明了会话模式时用 agent 的模式（那是它真正支持的执行档位），
    /// 其余情况用本地的任务类型/工具策略列表。
    fn refresh_execution_mode_options(&mut self) {
        self.tool_options = self
            .acp
            .as_ref()
            .map(|acp| acp_execution_mode_options(&acp.state()))
            .filter(|options| !options.is_empty())
            .unwrap_or_else(default_execution_mode_options);
    }

    fn select_execution_mode(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_running {
            return;
        }
        // ACP 后端下拉里是 agent 自己声明的会话模式，选中后走 `session/set_mode`。
        if let Some(mode_id) = id.strip_prefix(ACP_MODE_OPTION_PREFIX) {
            self.select_acp_mode(mode_id.to_string(), cx);
            return;
        }
        // 新式 agent（opencode 等）把模式放在 configOptions，选中后走 `session/setConfigOption`。
        if let Some(value) = id.strip_prefix(ACP_CONFIG_MODE_OPTION_PREFIX) {
            self.select_acp_config_mode(value.to_string(), cx);
            return;
        }
        if self.tool_options.iter().all(|o| o.id.as_ref() != id) {
            return;
        }
        // 「问答」只改任务类型（它本身不下发工具），其余 id 只改工具策略、任务类型回到常规。
        let selection = match task_kind_from_id(id) {
            TaskKind::Agent => ExecutionSelection::tool(tool_execution_mode_from_id(id)),
            task => ExecutionSelection::new(task, self.tool_execution_mode),
        };
        if self.backend == Backend::Acp
            && let Err(error) = set_current_acp_tool_mode(cx, selection.tool)
        {
            let message = t!(
                "AgentChat.acp_tool_mode_update_failed",
                error = error.to_string()
            )
            .to_string();
            tracing::warn!(%error, "Failed to update ACP Public MCP permission mode");
            self.transcript.push_system(message);
            self.request_scroll_to_bottom();
            cx.notify();
            return;
        }
        AppSettings::update_and_save(cx, |settings| {
            settings.ai_chat.tool_execution_mode = settings_execution_mode(selection);
        });
        self.tool_execution_mode = selection.tool;
        self.task_kind = selection.task;
        self.sync_composer(cx);
        cx.notify();
    }

    /// 切换 ACP agent 自己声明的会话模式。
    fn select_acp_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        let Some(acp) = self.acp.take() else {
            return;
        };
        let mode = agent_client_protocol::schema::v1::SessionModeId::new(mode_id);
        cx.spawn(async move |this, cx| {
            let result = acp.set_mode(mode).await;
            let _ = this.update(cx, |this, cx| {
                this.acp = Some(acp);
                if let Err(error) = result {
                    this.transcript
                        .push_system(format!("ACP mode switch failed: {error}"));
                }
                this.sync_composer(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 切换新式 agent（`SessionConfigOption{category=mode}`）的会话模式。
    fn select_acp_config_mode(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(acp) = self.acp.take() else {
            return;
        };
        let Some(config_id) = acp
            .state()
            .current_mode_config()
            .map(|option| option.id.clone())
        else {
            self.acp = Some(acp);
            return;
        };
        let value = agent_client_protocol::schema::v1::SessionConfigValueId::new(value);
        cx.spawn(async move |this, cx| {
            let result = acp.set_config_option(config_id, value).await;
            let _ = this.update(cx, |this, cx| {
                this.acp = Some(acp);
                if let Err(error) = result {
                    this.transcript
                        .push_system(format!("ACP mode switch failed: {error}"));
                }
                this.sync_composer(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        self.history_popover_open = false;
        // ACP 后端:会话由外部 agent 管理,这里仅做视觉重置(清空转录)。
        if self.backend == Backend::Acp {
            // 正在跑的轮次不取消：转入后台继续跑，输出继续落进这个会话的转录缓存。
            // 用户点「新建对话」意图是开新话题，不是杀掉正在生成的回答。
            for owner in self.acp_turn_owners.iter_mut() {
                if owner.session_uid == self.current_session {
                    owner.backgrounded = true;
                }
            }
            let session_uid = self.current_session.clone();
            let Some(agent_id) = self.current_acp_id.clone() else {
                self.transcript.clear();
                self.transcript
                    .push_system(t!("AgentUi.acp_not_connected").to_string());
                cx.notify();
                return;
            };
            let transition_phase = self.acp_session_transition_phase(&session_uid);
            if transition_phase == Some(AcpSessionTransitionPhase::Creating) {
                return;
            }
            let retrying_failed_transition =
                transition_phase == Some(AcpSessionTransitionPhase::Failed);
            let Some(mut acp) = self.acp.take() else {
                self.transcript.clear();
                self.transcript
                    .push_system(t!("AgentUi.acp_not_connected").to_string());
                cx.notify();
                return;
            };
            if !retrying_failed_transition {
                self.pending_submissions.clear_session(&session_uid);
            }
            // 屏上转录交给缓存：后台轮次的后续输出靠它接住，不能随 visual reset 丢掉。
            let mut stashed = AgentTranscript::new();
            stashed.set_resource_context(&self.resources);
            let transcript = std::mem::replace(&mut self.transcript, stashed);
            self.cache_session_transcript(session_uid.clone(), transcript);
            self.set_session_running(&session_uid, false, cx);
            self.input
                .update(cx, |input, cx| input.set_running(false, cx));
            self.trim_session_transcripts();
            self.sync_pending_preview(cx);
            let operation =
                self.begin_acp_session_transition(agent_id.clone(), session_uid.clone());
            self.transcript.clear();
            self.transcript
                .push_system(t!("AgentUi.creating_acp_session").to_string());
            self.input
                .update(cx, |input, cx| input.set_running(true, cx));
            self.sync_pending_preview(cx);
            self.request_scroll_to_bottom();
            cx.notify();
            let workspace_root = self.workspace_root.clone();
            cx.spawn(async move |this, cx| {
                let result = acp.create_session(workspace_root).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.is_current_acp_session_transition(operation, &agent_id, &session_uid) {
                        return;
                    }
                    this.input
                        .update(cx, |input, cx| input.set_running(false, cx));
                    this.acp = Some(acp);
                    this.transcript.clear();
                    let protocol_session_id =
                        this.acp.as_ref().map(|acp| acp.protocol_session_id());
                    match result {
                        Ok(_) => {
                            this.clear_acp_session_transition(operation);
                            // 新建出来的会话也是这个内置会话「当前指向」的那条，
                            // 下次重连要回到它，而不是再开一条空的。
                            if let Some(protocol_session_id) = protocol_session_id.as_deref() {
                                this.remember_acp_protocol_session(
                                    &session_uid,
                                    &agent_id,
                                    protocol_session_id,
                                    cx,
                                );
                            }
                            // 刚建好的会话要立刻出现在会话列表里：以前只有手动点刷新
                            // 或重连才会拉列表，用户会以为压根没建成。
                            this.reload_acp_sessions(cx);
                            // 后台轮次还在飞时不推进队列：AlreadyRunning 会自己排队，
                            // 终态后 `advance_acp_pending_after_terminal` 会补发。
                            this.start_next_pending(&session_uid, cx);
                        }
                        Err(err) => {
                            this.mark_acp_session_transition_failed(
                                operation,
                                &agent_id,
                                &session_uid,
                            );
                            this.transcript.push_system(
                                t!("AgentUi.create_acp_session_failed", error = err).to_string(),
                            );
                        }
                    }
                    this.request_scroll_to_bottom();
                    this.sync_pending_preview(cx);
                    this.sync_composer(cx);
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        // 已经站在**当前工作区的一张白纸**上：「新建会话」不该再 produce 第二张
        // 白纸。（本地后端才有的判断，理由见 `current_session_is_blank`；
        // 白纸本身归属别的工作区时要照常新建——侧栏不按工作区过滤，用户完全
        // 可能站在别的工作区的白纸上。）
        if self.current_session_is_blank(cx)
            && self.session_belongs_to_current_workspace(&self.current_session)
        {
            self.show_archived = false;
            self.reload_sessions(cx);
            cx.notify();
            return;
        }
        // 上一张被留在身后的白纸还在的话，回到它——比再造一张更贴近用户预期
        // （他上一次「新建」得到的那张纸还在，没必要给两张）。
        let reusable_draft = self
            .draft_session
            .clone()
            .filter(|uid| self.reusable_blank_session(uid));
        if let Some(draft) = reusable_draft {
            self.draft_session = None;
            self.show_archived = false;
            self.switch_session(&draft, cx);
            return;
        }
        // 指针在本工作区用不上就清掉——**除了**别的工作区的白纸：那张纸在它
        // 自己的工作区里仍然有效，用户切回去时应当还能复用（每次复用前都会
        // 经 `reusable_blank_session` 重新校验，指针留着是安全的）。
        self.draft_session = self
            .draft_session
            .take()
            .filter(|uid| !self.session_belongs_to_current_workspace(uid));
        // 新建前先保存当前会话,避免内容丢失。
        // 草稿同理：先捕获再落盘，半句话跟着旧会话走。
        self.capture_session_draft(cx);
        self.persist_current(cx);
        self.show_archived = false;
        self.start_fresh_session(cx);
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 切换驱动后端:`None` = One_Agent(自研);`Some(id)` = 对应 ACP agent。
    fn select_backend(&mut self, agent_id: Option<SharedString>, cx: &mut Context<Self>) {
        if !self.can_select_backend(agent_id.as_ref()) {
            return;
        }
        match agent_id {
            None => self.select_local_backend(cx),
            Some(id) => self.select_acp_backend(id, cx),
        }
    }

    fn can_select_backend(&self, agent_id: Option<&SharedString>) -> bool {
        // 后端切换会替换事件订阅；先让所有本地后台任务自然结束。
        let retrying_disconnected_acp =
            agent_id.is_some_and(|id| self.can_retry_disconnected_acp(id));
        self.running_sessions.is_empty()
            && (self.pending_submissions.is_empty() || retrying_disconnected_acp)
    }

    fn can_retry_disconnected_acp(&self, requested_id: &SharedString) -> bool {
        self.backend == Backend::Acp
            && self.current_acp_id.as_ref() == Some(requested_id)
            && self.acp.is_none()
            && !self.acp_connecting
            && self.acp_pending.is_none()
            && self.acp_turn_owners.is_empty()
            && self.acp_session_transition.is_none()
            && !self.closed_sessions.contains(&self.current_session)
    }

    /// 新建一个空会话并设为当前(仅运行时层面,不触碰持久化 / 列表)。
    fn start_fresh_session(&mut self, cx: &mut Context<Self>) {
        if self.is_running && should_stop_task_before_session_switch(self.backend) {
            self.stop(cx);
        }
        self.stash_current_transcript();
        let previous_session = self.current_session.clone();
        let session = self.runtime.create_session(self.resources.clone());
        self.session_id = session.id().clone();
        // 归属定格：新会话记住创建时的工作区，之后不随外壳切换而漂移。
        self.session_roots.insert(
            self.session_id.to_string(),
            self.workspace_root.to_string_lossy().into_owned(),
        );
        if self.system_instruction_from_settings {
            // 新会话跟随设置的最新值（含清空）；外部显式指令保持不变。
            self.system_instruction = AppSettings::current(cx)
                .ai_chat
                .effective_custom_system_prompt();
        }
        self.apply_system_instruction_to_current_session();
        self.sync_session_skills();
        self.current_session = self.session_id.to_string();
        // 新建会话也是一次「主动访问」，旧会话留在后退栈里。
        self.session_navigation
            .visit(Some(previous_session), self.current_session.as_str());
        self.session_switcher
            .record_access(self.current_session.as_str());
        self.closed_sessions.remove(&self.current_session);
        self.trim_session_transcripts();
        self.transcript = AgentTranscript::new();
        self.transcript.set_resource_context(&self.resources);
        self.is_running = false;
        // 新会话从空输入框开始（渲染时应用；文字与附件都换空的——
        // 上一会话挂在输入框里的图片不许跟过来）。
        self.pending_input_draft = Some(ComposerDraft::default());
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self.sync_pending_preview(cx);
        self.upsert_live_summary(
            self.current_session.clone(),
            current_agent_task_title(),
            now_secs(),
        );
    }

    /// 从存储重载当前视图(活跃 / 已归档)的会话列表。
    fn reload_sessions(&mut self, cx: &mut Context<Self>) {
        let persisted = if self.show_archived {
            persistence::list_archived_summaries(cx)
        } else {
            persistence::list_summaries(cx)
        };
        self.sessions = merge_live_session_summaries(
            persisted,
            &self.live_sessions,
            &self.current_session,
            &self.running_sessions,
            self.show_archived,
        );
    }

    pub(crate) fn showing_archived_sessions(&self) -> bool {
        self.show_archived
    }

    /// 切换「活跃 / 已归档」视图。
    pub(crate) fn toggle_archived(&mut self, cx: &mut Context<Self>) {
        self.show_archived = !self.show_archived;
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 归档(软删除)一个会话;归档当前会话时自动新建空会话顶上。
    pub(crate) fn apply_archive(&mut self, uid: &str, cx: &mut Context<Self>) {
        if !persistence::set_archived(cx, uid, true) {
            return;
        }
        if self.current_session == uid {
            self.start_fresh_session(cx);
        }
        self.discard_live_session(uid);
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 从归档恢复一个会话(回到活跃列表)。
    fn apply_unarchive(&mut self, uid: &str, cx: &mut Context<Self>) {
        if persistence::set_archived(cx, uid, false) {
            self.reload_sessions(cx);
            cx.notify();
        }
    }

    /// 把当前会话快照写入持久化存储,并刷新其侧边栏摘要(空会话不落库)。
    fn persist_current(&mut self, cx: &mut Context<Self>) {
        let uid = self.current_session.clone();
        self.persist_session(&uid, cx);
    }

    fn persist_session(&mut self, uid: &str, cx: &mut Context<Self>) {
        if self.closed_sessions.contains(uid) {
            return;
        }
        let session_id = SessionId::from_string(uid.to_string());
        let Some(session) = self.runtime.session(&session_id) else {
            return;
        };
        // 归属定格：优先创建/载入时记录的工作区；缺失（旧数据在本次升级后
        // 首次落盘）才用当前工作区补。
        let workspace_root = self
            .session_roots
            .get(uid)
            .cloned()
            .unwrap_or_else(|| self.workspace_root.to_string_lossy().into_owned());
        if let Some((title, updated_at)) =
            persistence::save_session_with_workspace(cx, &session, Some(&workspace_root))
        {
            self.upsert_live_summary(uid.to_string(), title, updated_at);
        }
    }

    fn upsert_live_summary(&mut self, uid: String, title: String, updated_at: i64) {
        self.live_sessions.retain(|summary| summary.id != uid);
        let workspace_root = self.session_roots.get(&uid).cloned();
        self.live_sessions.insert(
            0,
            SessionSummary::new(uid, title, updated_at).with_workspace_root(workspace_root),
        );
    }

    fn touch_session_transcript(&mut self, uid: &str) {
        if !self.session_transcripts.contains_key(uid) {
            return;
        }
        self.session_transcript_order.retain(|item| item != uid);
        self.session_transcript_order.push_back(uid.to_string());
    }

    fn cache_session_transcript(&mut self, uid: String, transcript: AgentTranscript) {
        self.session_transcripts.insert(uid.clone(), transcript);
        self.touch_session_transcript(&uid);
        self.trim_session_transcripts();
    }

    fn remove_cached_session_transcript(&mut self, uid: &str) -> Option<AgentTranscript> {
        self.session_transcript_order.retain(|item| item != uid);
        self.session_transcripts.remove(uid)
    }

    fn clear_cached_session_transcripts(&mut self) {
        self.session_transcripts.clear();
        self.session_transcript_order.clear();
    }

    fn session_transcript_is_protected(&self, uid: &str) -> bool {
        uid == self.current_session
            || self.running_sessions.contains(uid)
            || self.pending_submissions.len(uid) > 0
            || self
                .acp_turn_owners
                .iter()
                .any(|owner| owner.session_uid == uid)
            || self
                .acp_session_transition
                .as_ref()
                .is_some_and(|transition| transition.session_uid == uid)
    }

    fn trim_session_transcripts(&mut self) {
        self.trim_session_transcripts_to(MAX_CACHED_SESSION_TRANSCRIPTS);
    }

    fn trim_session_transcripts_to(&mut self, max_entries: usize) {
        self.trim_session_transcripts_to_preserving(max_entries, None);
    }

    fn trim_session_transcripts_to_preserving(
        &mut self,
        max_entries: usize,
        preserve_uid: Option<&str>,
    ) {
        while self.session_transcripts.len() > max_entries {
            let Some(index) = self.session_transcript_order.iter().position(|uid| {
                preserve_uid != Some(uid.as_str())
                    && self.session_transcripts.contains_key(uid)
                    && !self.session_transcript_is_protected(uid)
            }) else {
                break;
            };
            let uid = self
                .session_transcript_order
                .remove(index)
                .expect("session transcript LRU index must remain valid");
            self.session_transcripts.remove(&uid);
        }
    }

    /// 把屏幕上的本地转录收进会话缓存。返回**是否真的收下了一段非空转录**。
    ///
    /// 返回值给调用方判断要不要提示用户「本地这段不会带到 agent 那边」：空转录没什么可失的，
    /// 提示只会变成噪音。
    ///
    /// 空转录**不入缓存**：缓存里留一个空壳会遮蔽 Runtime 快照重建
    /// （见 [`Self::restore_local_transcript`]），让「切回本地」看起来像历史丢了。
    fn stash_current_transcript(&mut self) -> bool {
        if self.backend != Backend::Local {
            return false;
        }
        let mut replacement = AgentTranscript::new();
        replacement.set_resource_context(&self.resources);
        let transcript = std::mem::replace(&mut self.transcript, replacement);
        if transcript.is_empty() {
            return false;
        }
        self.cache_session_transcript(self.current_session.clone(), transcript);
        true
    }

    /// 把输入框里尚未发送的文字记到当前会话上（会话草稿）。
    ///
    /// 调用时机：**离开当前会话之前**（切换 / 新建），且必须在 `persist_current`
    /// 之前——落盘走 `session.snapshot()`，草稿要先写进 Runtime 会话才会被带上。
    /// 草稿是每个会话独立的：切走再切回，输入框显示的是**那个会话自己**的
    /// 未发送内容，而不是全局共享一块输入。
    ///
    /// 附件是**只读捕获**（`ImageAttachment` 内是 `Arc`，克隆廉价）：不把图片从
    /// 输入框里拿走，留给 `apply_pending_input_draft` 在切换成立后整体替换——
    /// 这样切换中途失败（目标快照加载不到）时输入框原样未动，不会出现
    /// 「图没了、字还在」的中间态。
    fn capture_session_draft(&mut self, cx: &App) {
        // 上一次切换排队的草稿还没落进输入框：此刻输入框里显示的是**再上一个**
        // 会话的内容，当成当前会话的草稿捕获就串台了。当前会话的正确草稿
        // 仍在 Runtime 会话上（staging 只读不写），跳过即可。两次切换之间
        // 必然没有用户输入（中间隔着一帧渲染），不会漏掉真实的编辑。
        if self.pending_input_draft.is_some() {
            return;
        }
        if self.closed_sessions.contains(&self.current_session) {
            return;
        }
        let text = self.input.read(cx).composer_text(cx);
        let images = self.input.read(cx).composer_attachments().to_vec();
        if let Some(session) = self
            .runtime
            .session(&SessionId::from_string(self.current_session.clone()))
        {
            session.set_draft((!text.trim().is_empty() || !images.is_empty()).then(|| text));
        }
        if images.is_empty() {
            self.session_draft_attachments.remove(&self.current_session);
        } else {
            self.session_draft_attachments
                .insert(self.current_session.clone(), images);
        }
    }

    /// 目标会话的草稿送入输入框（经 [`Self::pending_input_draft`] 在渲染时应用）。
    ///
    /// 没有 `window` 时只排队不写——渲染帧必然到来，见字段注释。
    fn stage_session_draft(&mut self, uid: &str) {
        let text = self
            .runtime
            .session(&SessionId::from_string(uid.to_string()))
            .and_then(|session| session.draft())
            .unwrap_or_default();
        let images = self
            .session_draft_attachments
            .get(uid)
            .cloned()
            .unwrap_or_default();
        self.pending_input_draft = Some(ComposerDraft { text, images });
    }

    /// 渲染开头应用排队的草稿。只有切换会话会排队，平时是 no-op。
    ///
    /// 文字与附件**一起**换：附件是整体替换——目标会话没有附件草稿时，
    /// 上一会话挂在输入框里的图片必须被清掉，否则会跟着新会话一起发出去。
    fn apply_pending_input_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.pending_input_draft.take() {
            self.input.update(cx, |input, cx| {
                input.set_composer_text(&draft.text, window, cx);
                input.set_composer_attachments(draft.images, cx);
            });
        }
    }

    /// `uid` 的 **Runtime 历史**是否还只有系统提示——即这个会话到底有没有被用过。
    ///
    /// 这是空白判定的**事实源**（不看屏幕转录）：转录可能只是留在缓存里的副本，
    /// 或者在流式过程中短暂落后于历史。Runtime 不在这个会话时返回 `false`
    /// （「不敢当白纸」比「误当白纸」安全得多）。
    fn runtime_history_is_blank(&self, uid: &str) -> bool {
        let session_id = SessionId::from_string(uid.to_string());
        self.runtime.session(&session_id).is_some_and(|session| {
            session
                .snapshot()
                .history
                .iter()
                .all(HistoryItem::is_system_note)
        })
    }

    /// 当前会话是否还是一张白纸：**本地后端**、屏幕上没聊过、Runtime 历史也没聊过、
    /// 没在跑、没有排队提交、输入框也空着（**文字与附件都空**）。
    ///
    /// 只看本地后端：ACP 的转录空白并不等于 agent 那边没有上下文——`session/resume`
    /// 按设计不回放历史，转录正是空的，可 agent 手里握着整段对话（见
    /// [`AcpSessionContinuity::ReusedWithoutHistory`]）。把「转录空」当「会话空」，
    /// 会让用户点「新建会话」时被静默拦下，那才是真 bug。
    ///
    /// 输入框非空（文字**或**挂了图片）也算「不空」：用户已经开始准备内容了，
    /// 他要的是一个干净输入框；这些半成品会跟着旧会话留在它的草稿里
    /// （见 [`Self::stage_session_draft`]），不会被吞。
    fn current_session_is_blank(&self, cx: &App) -> bool {
        self.backend == Backend::Local
            && !self.transcript.has_conversation()
            && self.runtime_history_is_blank(&self.current_session)
            && !self.is_running
            && self
                .pending_submissions
                .items(self.current_session.as_str())
                .is_empty()
            && self.input.read(cx).composer_text(cx).trim().is_empty()
            && self.input.read(cx).composer_attachments().is_empty()
    }

    /// 离开当前会话时，若它还是白纸就记下来当「可复用的草稿会话」。
    ///
    /// 不记的话，用户每点一次「+」都在侧栏留一个空条目：
    /// 空白会话增殖是「新建会话」最容易被抱怨的地方。
    fn remember_blank_session_on_leave(&mut self, cx: &App) {
        if !self.current_session_is_blank(cx) {
            return;
        }
        let uid = self.current_session.clone();
        if !self.closed_sessions.contains(&uid) {
            self.draft_session = Some(uid);
        }
    }

    /// `uid` 是否是一张仍在运行时里、还没被聊过的白纸。
    ///
    /// 判据取 Runtime 的**历史**而不是屏幕转录：转录可能是切换时留在缓存里的副本，
    /// 历史才是「这个会话到底说过话没有」的事实源。
    ///
    /// 只复用**当前工作区**的白纸：工作区 B 里点「新建」不该把用户切到工作区 A
    /// 留下的白纸上——那张纸的快照归属、落盘目录都是 A 的。这条约束按工作区生效，
    /// 工作区不同的白纸一律不复用。
    fn reusable_blank_session(&self, uid: &str) -> bool {
        if uid == self.current_session
            || self.closed_sessions.contains(uid)
            || self.running_sessions.contains(uid)
            || !self.pending_submissions.items(uid).is_empty()
            || !self.session_belongs_to_current_workspace(uid)
        {
            return false;
        }
        self.runtime_history_is_blank(uid)
    }

    /// 会话是否归属于当前工作区。
    ///
    /// 没有记录归属（旧数据 / 测试里直接 restore 的会话）视为归属当前工作区 ——
    /// 与落盘时的兜底同一约定（见 [`Self::persist_session`]）。注意这只是**判定**
    /// 用的默认值：点开一条无归属旧会话不会给它定格归属，别拿这里的 `true`
    /// 去反推「它就属于当前工作区」并回写。
    fn session_belongs_to_current_workspace(&self, uid: &str) -> bool {
        let Some(root) = self.session_roots.get(uid) else {
            return true;
        };
        *root == self.workspace_root.to_string_lossy().into_owned()
    }

    /// 切回本地后端时把该会话的本地转录放回屏幕。
    ///
    /// 顺序：内存缓存（离开本地时 stash 的）→ Runtime 快照重建 → 清空。
    /// 不能无条件清空：屏幕上此刻是 ACP agent 的内容，清掉就等于把本地那段对话抹了，
    /// 这正是「会话记不住」的观感来源。ACP 自己的转录归 agent 所有（靠 load/resume 取回），
    /// 不进这个缓存——缓存里永远只有本地转录，切回来才不会串台。
    ///
    /// 缓存命中也要看**是否非空**：可能有人往这个会话的缓存里塞过一个空壳转录
    /// （例如连接失败时给非当前会话建过一块），空壳命中会让下面的 Runtime 重建被跳过，
    /// 屏幕上就空了——而 Runtime 里历史还在。
    fn restore_local_transcript(&mut self, session_uid: &str) {
        if let Some(transcript) = self.remove_cached_session_transcript(session_uid)
            && !transcript.is_empty()
        {
            self.transcript = transcript;
            self.transcript.set_resource_context(&self.resources);
            return;
        }
        self.transcript.clear();
        let session_id = SessionId::from_string(session_uid.to_string());
        let Some(session) = self.runtime.session(&session_id) else {
            self.transcript.set_resource_context(&self.resources);
            return;
        };
        let snapshot = session.snapshot();
        self.transcript
            .load_history(&snapshot.history, snapshot.plan.as_ref());
        self.transcript.set_resource_context(&self.resources);
    }

    fn discard_live_session(&mut self, uid: &str) {
        self.closed_sessions.insert(uid.to_string());
        self.request_acp_cancel_for_session(uid);
        self.invalidate_local_operation_generation(uid);
        let session_id = SessionId::from_string(uid.to_string());
        let active_turn = self
            .runtime
            .session(&session_id)
            .and_then(|session| session.current_turn_id());
        if let Some(turn_id) = active_turn.as_ref() {
            self.ignored_local_turns.insert(turn_id.clone());
        }
        let was_running = self.running_sessions.remove(uid);
        if (was_running || active_turn.is_some())
            && let Err(error) = self.runtime.interrupt(&session_id)
        {
            tracing::warn!(session_id = %uid, %error, "Failed to interrupt discarded agent session");
            if let Some(turn_id) = active_turn.as_ref() {
                self.ignored_local_turns.remove(turn_id);
            }
        }
        self.runtime.close_session(&session_id);
        self.pending_submissions.remove_session(uid);
        self.remove_cached_session_transcript(uid);
        // 会话没了，它的附件草稿（可能握着几 MB 的图片）也要一并放掉。
        self.session_draft_attachments.remove(uid);
        // 这张纸没了，别再拿它当「可复用的空白草稿」。
        if self.draft_session.as_deref() == Some(uid) {
            self.draft_session = None;
        }
        self.live_sessions.retain(|summary| summary.id != uid);
        // 会话没了：导航栈里也不能再有它，否则后退会落到一个不存在的会话。
        self.session_navigation.remove(uid);
        // 切换器的 recency / 列表同样要清。
        self.session_switcher.remove(uid);
    }

    /// 切换到另一个(已持久化的)会话:保存当前 → 加载快照恢复 → 重建转录。
    fn switch_session(&mut self, uid: &str, cx: &mut Context<Self>) {
        self.switch_session_with_origin(uid, SessionSwitchOrigin::Visit, cx);
    }

    /// 后退/前进导航：切换到导航栈给出的目标。
    ///
    /// 栈的移动（go_back/go_forward）发生在调用**之前**，这里只负责切换本身，
    /// 不再向 back 栈压入当前会话——否则后退会变成「来回横跳永不收敛」。
    fn navigate_session(&mut self, uid: &str, cx: &mut Context<Self>, origin: SessionSwitchOrigin) {
        debug_assert!(
            origin != SessionSwitchOrigin::Visit,
            "导航路径不该走 Visit 语义"
        );
        self.switch_session_with_origin(uid, origin, cx);
    }

    /// 会话后退；没有可退的目标时是 no-op（不是错误）。
    fn navigate_session_back(&mut self, cx: &mut Context<Self>) {
        let current = self.current_session.clone();
        if self
            .session_navigation
            .back_target()
            .is_some_and(|target| target != current)
            && let Some(target) = self.session_navigation.go_back(&current)
        {
            self.navigate_session(&target, cx, SessionSwitchOrigin::Back);
        }
    }

    /// 会话前进；没有可进的目标时是 no-op。
    fn navigate_session_forward(&mut self, cx: &mut Context<Self>) {
        let current = self.current_session.clone();
        if self
            .session_navigation
            .forward_target()
            .is_some_and(|target| target != current)
            && let Some(target) = self.session_navigation.go_forward(&current)
        {
            self.navigate_session(&target, cx, SessionSwitchOrigin::Forward);
        }
    }

    /// 切换会话的统一实现。`origin` 决定导航栈怎么记账（见 [`SessionSwitchOrigin`]）。
    fn switch_session_with_origin(
        &mut self,
        uid: &str,
        origin: SessionSwitchOrigin,
        cx: &mut Context<Self>,
    ) {
        // 侧边栏视图:从历史 Popover 选择后随即收起。
        self.history_popover_open = false;
        if uid == self.current_session {
            self.sync_pending_preview(cx);
            cx.notify();
            return;
        }
        let previous_session = self.current_session.clone();
        // 把「离开的是一张白纸」记下来（判定要在 `transcript` 被换掉之前做）。
        self.remember_blank_session_on_leave(cx);
        // ACP 的在飞轮次不取消：转入后台，输出继续回流到这个会话的转录缓存。
        if self.backend == Backend::Acp {
            for owner in self.acp_turn_owners.iter_mut() {
                if owner.session_uid == previous_session {
                    owner.backgrounded = true;
                }
            }
        }
        if self.is_running && should_stop_task_before_session_switch(self.backend) {
            self.stop(cx);
        }
        // 草稿必须先于落盘捕获：persist_current 走 session.snapshot()，
        // 晚一步这次切换产生的草稿就不会进快照。
        self.capture_session_draft(cx);
        self.persist_current(cx);
        self.stash_current_transcript();

        let target_id = SessionId::from_string(uid.to_string());
        let target = if let Some(session) = self.runtime.session(&target_id) {
            session
        } else {
            let Some(snapshot) = persistence::load_snapshot(cx, uid) else {
                let current_session = self.current_session.clone();
                if let Some(transcript) = self.remove_cached_session_transcript(&current_session) {
                    self.transcript = transcript;
                }
                self.reload_sessions(cx);
                cx.notify();
                return;
            };
            // 载入即定格：快照里**记过**归属就照它定格；没有归属的旧会话保持
            // 「无归属」——点开看一眼不是归属变更（侧栏「未分组」就是它的语义），
            // 真正的定格留到它下次落盘（见 [`Self::persist_session`] 的兜底）。
            if let Some(root) = snapshot.workspace_root.clone() {
                self.session_roots.insert(uid.to_string(), root);
            }
            let restored = self.runtime.restore_session(snapshot);
            restored.set_resources(self.resources.clone());
            restored
        };
        self.closed_sessions.remove(uid);
        // 这段对话是不是归外部 agent 管：运行时会话带着地址（从快照恢复的同样带）。
        let external_agent = target.acp_ref();
        self.session_id = target.id().clone();
        // 快照自带指令的会话优先快照值（行为可复现）；没有指令的旧会话回落到
        // 当前全局设置（可能为空），并继续保持来源跟随设置。
        self.system_instruction = target.system_instruction().or_else(|| {
            AppSettings::current(cx)
                .ai_chat
                .effective_custom_system_prompt()
        });
        self.system_instruction_from_settings = target.system_instruction().is_none();
        self.current_session = self.session_id.to_string();
        // 只有「用户主动访问」才进导航历史；后退/前进本身不动栈（栈在调用前已移动）。
        if origin == SessionSwitchOrigin::Visit {
            self.session_navigation
                .visit(Some(previous_session), self.current_session.as_str());
            // 用户已经点了别的会话：切换器不再悬着（提交守卫之外的主动关闭）。
            self.session_switcher.dismiss();
        }
        // recency：无论来源（点击 / 后退 / 前进 / 切换器），都是一次真实访问。
        self.session_switcher.record_access(uid);
        // 切换已成立：输入框换显示目标会话自己的草稿（渲染时应用）。
        self.stage_session_draft(uid);
        self.apply_system_instruction_to_current_session();
        if let Some(transcript) = self.remove_cached_session_transcript(uid)
            && !transcript.is_empty()
        {
            self.transcript = transcript;
        } else {
            let snapshot = target.snapshot();
            self.transcript
                .load_history(&snapshot.history, snapshot.plan.as_ref());
            self.transcript.set_resource_context(&self.resources);
        }
        self.trim_session_transcripts();
        self.is_running = self.running_sessions.contains(uid);
        self.input
            .update(cx, |input, cx| input.set_running(self.is_running, cx));
        self.sync_pending_preview(cx);
        self.reload_sessions(cx);
        self.sync_composer(cx);
        self.request_scroll_to_bottom();
        if self.backend == Backend::Acp && !self.is_running {
            self.start_or_reconnect_current_pending(cx);
        }
        // 回到一条外部 agent 会话：把 agent 那边接回来（本地只有地址，历史在它手上）。
        if let Some(reference) = external_agent {
            self.reopen_acp_session(uid, reference, cx);
        }
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    /// 打开重命名对话框。
    fn start_rename(
        &mut self,
        uid: String,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(&current_name)
                .placeholder(t!("AgentUi.session_name").to_string())
        });
        let view = cx.entity();
        let input = input_state.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            let uid = uid.clone();
            dialog
                .title(t!("AgentUi.rename_session").to_string())
                .w(px(360.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("AgentUi.save").to_string())
                        .cancel_text(t!("AgentUi.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let new_name = input_for_ok.read(cx).value().trim().to_string();
                    if !new_name.is_empty() {
                        view_for_ok.update(cx, |this, cx| this.apply_rename(&uid, new_name, cx));
                    }
                    true
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("AgentUi.enter_new_session_name").to_string()),
                        )
                        .child(Input::new(&input).w_full()),
                )
        });
    }

    /// 提交重命名:更新存储与侧边栏摘要。
    fn apply_rename(&mut self, uid: &str, new_name: String, cx: &mut Context<Self>) {
        if persistence::rename_session(cx, uid, &new_name) {
            if let Some(summary) = self.sessions.iter_mut().find(|s| s.id == uid) {
                summary.name = new_name.clone().into();
                summary.updated_at = now_secs();
            }
            if let Some(summary) = self
                .live_sessions
                .iter_mut()
                .find(|summary| summary.id == uid)
            {
                summary.name = new_name.into();
                summary.updated_at = now_secs();
            }
            cx.notify();
        }
    }

    /// 打开删除确认对话框。
    fn confirm_delete(
        &mut self,
        uid: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_for_ok = view.clone();
            let uid = uid.clone();
            dialog
                .title(t!("AgentUi.delete_session").to_string())
                .w(px(360.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("AgentUi.delete").to_string())
                        .cancel_text(t!("AgentUi.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    view_for_ok.update(cx, |this, cx| this.apply_delete(&uid, cx));
                    true
                })
                .child(
                    div()
                        .text_sm()
                        .child(t!("AgentUi.delete_session_confirm", name = name).to_string()),
                )
        });
    }

    /// 提交删除:从存储与列表移除;若删的是当前会话,自动新建一个空会话。
    fn apply_delete(&mut self, uid: &str, cx: &mut Context<Self>) {
        persistence::delete_session(cx, uid);
        if self.current_session == uid {
            self.start_fresh_session(cx);
        }
        self.discard_live_session(uid);
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 从外部发送消息(兼容 sidebar 的 ask_ai 功能)。
    pub fn send_external_message(&mut self, message: String, cx: &mut Context<Self>) {
        if message.trim().is_empty() {
            return;
        }
        self.submit(message, Vec::new(), Vec::new(), cx);
    }

    /// 设置系统提示词（用于自定义 AI 行为）。
    pub fn set_system_instruction(&mut self, instruction: Option<String>, cx: &mut Context<Self>) {
        self.system_instruction = instruction.clone();
        self.system_instruction_from_settings = false;
        self.apply_system_instruction_to_current_session();
        cx.notify();
    }

    fn apply_system_instruction_to_current_session(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_system_instruction(self.system_instruction.clone());
        }
    }

    fn sync_session_resources(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_resources(self.resources.clone());
        }
    }

    fn sync_resource_targets(&mut self, cx: &mut Context<Self>) {
        self.transcript.set_resource_context(&self.resources);
        let target_options: Vec<ComposerTarget> = self
            .resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        let ctx = build_composer_context(
            &self.resources,
            self.execution_selection(),
            self.selected_model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
            self.local_context_tokens(),
        );
        self.input.update(cx, |input, cx| {
            input.set_target_options(target_options, cx);
            input.set_context(ctx, cx);
        });
    }

    /// 更新可操作资源上下文与 `@` 提及项。
    pub fn set_resource_context(
        &mut self,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        cx: &mut Context<Self>,
    ) {
        let available_resources = resources.resources.clone();
        self.set_resource_context_with_catalog(resources, mentions, available_resources, cx);
    }

    pub fn set_resource_catalog(
        &mut self,
        mentions: Vec<MentionItem>,
        available_resources: Vec<ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        self.available_resources = available_resources;
        let resource_metadata_changed =
            refresh_pool_resource_metadata(&mut self.resources, &self.available_resources);
        self.input
            .update(cx, |input, cx| input.set_mentions(mentions, cx));
        if resource_metadata_changed {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
        } else {
            self.sync_composer(cx);
        }
        cx.notify();
    }

    pub fn set_resource_context_with_catalog(
        &mut self,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        self.available_resources = available_resources;
        self.resources = resources.clone();
        self.transcript.set_resource_context(&self.resources);
        self.sync_session_resources();
        let target_options: Vec<ComposerTarget> = self
            .resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        let ctx = build_composer_context(
            &self.resources,
            self.execution_selection(),
            self.selected_model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
            self.local_context_tokens(),
        );
        self.input.update(cx, |input, cx| {
            input.set_mentions(mentions, cx);
            input.set_target_options(target_options, cx);
            input.set_context(ctx, cx);
        });
        if self.sidebar_mode {
            self.request_scroll_to_bottom_until_layout_settles();
        } else {
            self.request_scroll_to_bottom();
        }
        cx.notify();
    }

    /// 注册代码块操作。
    pub fn register_code_block_action(&mut self, action: CodeBlockAction, _cx: &mut Context<Self>) {
        self.code_block_actions.register(action);
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        self.input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    /// 渲染单个会话行:活跃视图可点击切换 + 重命名/归档/删除;归档视图为恢复/永久删除。
    fn render_session_row(
        &self,
        session: &SessionSummary,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let uid = session.id.clone();
        let name = session.name.to_string();
        let archived_view = self.show_archived;
        let selected = !archived_view && self.current_session == session.id;
        let running = !archived_view && self.running_sessions.contains(&session.id);
        let group = SharedString::from(format!("agent-session-row-{uid}"));
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let row_style = themed_session_row_style(&theme);
        let running_color = running_session_indicator_color(selected, row_style);
        let running_indicator_id = format!("agent-session-running-spinner-{uid}");
        let running_animation_id = running_session_animation_id(&uid);

        // 标题区:活跃视图可点击切换;归档视图只读。
        let label = session_sidebar::session_row_with_style(session, selected, row_style, cx).when(
            running,
            move |label| {
                let debug_selector = running_indicator_id.clone();
                label.child(
                    h_flex()
                        .id(SharedString::from(running_indicator_id))
                        .debug_selector(move || debug_selector.clone())
                        .items_center()
                        .gap_0p5()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(running_color)
                        .child(
                            Spinner::new()
                                .small()
                                .color(running_color)
                                .animation_id(running_animation_id),
                        )
                        .child(t!("AgentUi.running").to_string()),
                )
            },
        );
        let label_area = if archived_view {
            div().flex_1().min_w_0().child(label).into_any_element()
        } else {
            let switch_uid = uid.clone();
            div()
                .id(SharedString::from(format!("agent-session-{uid}")))
                .flex_1()
                .min_w_0()
                .on_click(cx.listener(move |this, _, _, cx| this.switch_session(&switch_uid, cx)))
                .child(label)
                .into_any_element()
        };

        let mut actions = h_flex()
            .flex_shrink_0()
            .gap_0p5()
            .invisible()
            .group_hover(group.clone(), |this| this.visible());

        let delete_uid = uid.clone();
        let delete_name = name.clone();
        let delete_btn = Button::new(SharedString::from(format!("agent-delete-{uid}")))
            .icon(IconName::Delete)
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_delete(delete_uid.clone(), delete_name.clone(), window, cx);
            }));

        if archived_view {
            let unarchive_uid = uid.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("agent-unarchive-{uid}")))
                        .icon(IconName::WindowRestore)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_unarchive(&unarchive_uid, cx);
                        })),
                )
                .child(delete_btn);
        } else {
            let rename_uid = uid.clone();
            let rename_name = name.clone();
            let archive_uid = uid.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("agent-rename-{uid}")))
                        .icon(IconName::Edit)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_rename(rename_uid.clone(), rename_name.clone(), window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("agent-archive-{uid}")))
                        .icon(IconName::Inbox)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_archive(&archive_uid, cx);
                        })),
                )
                .child(delete_btn);
        }

        h_flex()
            .w_full()
            .items_center()
            .gap_0p5()
            .group(group)
            .child(label_area)
            .child(actions)
            .into_any_element()
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.sidebar_collapsed {
            return v_flex()
                .w(one_ui::theme_geometry().layout.compact_rail)
                .h_full()
                .flex_shrink_0()
                .border_r_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .items_center()
                .py_2()
                .gap_2()
                .child(
                    IconButton::new("agent-expand", IconName::PanelLeftOpen)
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("AgentUi.open_sidebar").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    IconButton::new("agent-new-collapsed", IconName::Plus)
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("AgentUi.new_task").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                )
                .into_any_element();
        }

        let body = self.render_sidebar_body(cx);

        v_flex()
            .w(one_ui::theme_geometry().layout.context_sidebar_default)
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(self.render_sidebar_header(cx))
            .child(body)
            .into_any_element()
    }

    /// 侧栏主体：内置 agent 会话 ∪ ACP 历史会话。
    ///
    /// 方案 §7.1 要的「统一列表」：ACP 会话不再把整个侧栏换成一句「externally managed」，
    /// 而是和内置会话并排，只是额外标出来源。ACP 能力不支持时才退回那句说明。
    fn render_sidebar_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let sessions = self.sessions.clone();
        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(sessions.len() + 4);
        for session in &sessions {
            rows.push(self.render_session_row(session, cx));
        }

        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        if self.acp_session_list_visible() {
            rows.push(acp_session_section_header(
                &theme,
                SharedString::from("agent-acp-sessions-refresh"),
                cx.listener(|this, _, _, cx| this.reload_acp_sessions(cx)),
            ));
            if let Some(model) = self.acp_session_list_model() {
                let current = self.acp_session_id_snapshot();
                for summary in &model.sessions {
                    let id = summary.id.clone();
                    rows.push(acp_session_row(
                        &theme,
                        summary,
                        current.as_deref() == Some(summary.id.as_str()),
                        SharedString::from(format!("agent-acp-session-{}", summary.id)),
                        cx.listener(move |this, _, _, cx| this.open_acp_session(&id, cx)),
                    ));
                }
                if let Some(placeholder) = acp_session_placeholder(
                    &theme,
                    cx.theme().danger,
                    model.loading,
                    model.error.as_deref(),
                    !model.sessions.is_empty(),
                ) {
                    rows.push(placeholder);
                }
            }
        } else if self.backend == Backend::Acp {
            rows.push(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.acp_external_managed").to_string())
                    .into_any_element(),
            );
        }

        v_flex()
            .id("agent-session-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_1()
            .children(rows)
            .into_any_element()
    }

    fn render_sidebar_header(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let title = agent_history_title(self.show_archived);
        PanelHeader::new("agent-sidebar-header")
            .variant(PanelHeaderVariant::Sidebar)
            .background(cx.theme().muted)
            .border_color(cx.theme().border)
            .leading(
                IconButton::new("agent-collapse", IconName::PanelLeftClose)
                    .role(IconButtonRole::Compact)
                    .tooltip(t!("AgentUi.close_sidebar").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .title(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .trailing(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        IconButton::new("agent-toggle-archived", IconName::Inbox)
                            .role(IconButtonRole::Compact)
                            .selected(self.show_archived)
                            .tooltip(t!("AgentUi.archived").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_archived(cx))),
                    )
                    .child(
                        IconButton::new("agent-new", IconName::Plus)
                            .role(IconButtonRole::Compact)
                            .tooltip(t!("AgentUi.new_conversation").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    ),
            )
            .into_any_element()
    }

    /// 侧边栏视图(窄面板)头部:标题 + 新建对话 + 历史记录(Popover)。
    fn render_sidebar_mode_header(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let border = theme.border;
        let muted = theme.muted;
        let history_open = self.history_popover_open;
        // 仅在打开时构建列表,避免每帧渲染全部会话行。
        let history_list = history_open.then(|| self.render_history_list(cx));

        PanelHeader::new("agent-sidebar-mode-header")
            .variant(PanelHeaderVariant::Sidebar)
            .border_color(border)
            .background(muted)
            .title(self.render_agent_switcher(cx))
            .trailing(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        IconButton::new("agent-sidebar-new", IconName::Plus)
                            .role(IconButtonRole::Compact)
                            .custom(agent_header_icon_variant(&theme, cx))
                            .tooltip(t!("AgentUi.new_task").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    )
                    .child(
                        Popover::new("agent-sidebar-history")
                            .anchor(Anchor::TopRight)
                            .p_0()
                            .open(history_open)
                            .on_open_change(cx.listener(|this, open: &bool, _window, cx| {
                                this.history_popover_open = *open;
                                if *open {
                                    this.reload_sessions(cx);
                                }
                                cx.notify();
                            }))
                            .trigger(
                                IconButton::new("agent-sidebar-history-btn", IconName::BookOpen)
                                    .role(IconButtonRole::Compact)
                                    .custom(agent_header_icon_variant(&theme, cx))
                                    .tooltip(t!("AgentUi.history_tasks").to_string()),
                            )
                            .when_some(history_list, |popover, list| popover.child(list)),
                    )
                    .when(self.show_sidebar_frame_controls, |this| {
                        this.child(self.render_sidebar_frame_options(cx))
                    })
                    .child(
                        IconButton::new("agent-sidebar-close", IconName::Close)
                            .role(IconButtonRole::Compact)
                            .custom(agent_header_icon_variant(&theme, cx))
                            .tooltip(t!("AgentUi.close_panel").to_string())
                            .on_click(cx.listener(|_this, _, _, cx| {
                                cx.emit(AgentChatViewEvent::Close);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub fn set_sidebar_header_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.show_sidebar_header == visible {
            return;
        }
        self.show_sidebar_header = visible;
        cx.notify();
    }

    pub fn set_sidebar_frame_controls(
        &mut self,
        visible: bool,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        if self.show_sidebar_frame_controls == visible && self.sidebar_frame_placement == placement
        {
            return;
        }
        self.show_sidebar_frame_controls = visible;
        self.sidebar_frame_placement = placement;
        cx.notify();
    }

    /// 当前可见会话列表（已合并实时运行中的会话）。
    pub fn session_summaries(&self) -> Vec<SessionSummary> {
        self.sessions.clone()
    }

    /// 当前会话标识。
    pub fn current_session_id(&self) -> &str {
        &self.current_session
    }

    /// 当前会话是否已经产生对话内容。
    ///
    /// 用于决定工作区选择器是否允许切换当前会话的工作区：
    /// 空会话可以随用户切工作区，已有消息后工作区被锁定。
    pub fn current_session_has_messages(&self) -> bool {
        !self.transcript.messages.is_empty()
    }

    /// 宿主同步「某会话里哪些轮次可以回滚」。
    ///
    /// 由工作区浏览器在快照锚定成功 / 回滚截断后推送**全量**列表。视图不自行推断：
    /// 快照可能锚定失败，猜错就会渲染出一个点了没反应的按钮。
    pub fn set_restorable_turns(
        &mut self,
        session_id: String,
        turn_ids: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        // 调用方是事件驱动的（轮次结束 / 回滚完成），不是逐帧渲染，直接重绘即可。
        if turn_ids.is_empty() {
            self.restorable_turns.remove(&session_id);
        } else {
            self.restorable_turns.insert(session_id, turn_ids);
        }
        cx.notify();
    }
    /// 内建侧栏是否已被工作台外壳接管。
    pub fn sidebar_suppressed(&self) -> bool {
        self.sidebar_suppressed
    }

    /// 工作台外壳接管左侧会话栏时调用；隐藏内建侧栏后由外壳渲染会话列表。
    pub fn set_sidebar_suppressed(&mut self, suppressed: bool, cx: &mut Context<Self>) {
        if self.sidebar_suppressed == suppressed {
            return;
        }
        self.sidebar_suppressed = suppressed;
        cx.notify();
    }

    /// 切换到指定会话；未知 id 由 [`Self::switch_session`] 自行兜底。
    pub fn select_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.switch_session(id, cx);
    }

    /// 新建会话。
    pub fn create_session(&mut self, cx: &mut Context<Self>) {
        self.new_session(cx);
    }

    /// 历史记录 Popover 内容:小标题 + 活跃/归档切换 + 会话行列表(复用行渲染)。
    fn render_history_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let border = theme.border;
        // ACP 模式:会话由外部 agent 管理,不展示本地列表。
        if self.backend == Backend::Acp {
            return v_flex()
                .w(sp(300.0))
                .p_3()
                .bg(theme.background)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(t!("AgentUi.acp_external_managed").to_string()),
                )
                .into_any_element();
        }
        let title = agent_history_title(self.show_archived);
        let sessions = self.sessions.clone();
        let rows: Vec<gpui::AnyElement> = sessions
            .iter()
            .map(|session| self.render_session_row(session, cx))
            .collect();
        let show_archived = self.show_archived;

        v_flex()
            .w(sp(300.0))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                PanelHeader::new("agent-history-header")
                    .variant(PanelHeaderVariant::Sidebar)
                    .horizontal_padding(one_ui::theme_geometry().spacing.space_2)
                    .background(theme.background)
                    .border_color(border)
                    .title(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .trailing(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                IconButton::new("agent-history-archived", IconName::Inbox)
                                    .role(IconButtonRole::Compact)
                                    .custom(agent_header_icon_variant(&theme, cx))
                                    .selected(show_archived)
                                    .tooltip(t!("AgentUi.archived").to_string())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_archived(cx)),
                                    ),
                            )
                            .child(
                                IconButton::new("agent-history-new", IconName::Plus)
                                    .role(IconButtonRole::Compact)
                                    .custom(agent_header_icon_variant(&theme, cx))
                                    .tooltip(t!("AgentUi.new_conversation").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                            ),
                    ),
            )
            .child(if rows.is_empty() {
                div()
                    .px_3()
                    .py_4()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(if show_archived {
                        t!("AgentUi.no_archived_sessions").to_string()
                    } else {
                        t!("AgentUi.no_history_sessions").to_string()
                    })
                    .into_any_element()
            } else {
                v_flex()
                    .id("agent-history-list")
                    .max_h(sp(360.0))
                    .overflow_y_scroll()
                    .p_1()
                    .gap_0p5()
                    .children(rows)
                    .into_any_element()
            })
            .into_any_element()
    }

    /// 注入工作台外壳的侧栏开关；注入后工具条在 agent 切换器两侧渲染
    /// 导航栏开关（leading）与右侧标签组开关（trailing）。
    pub fn set_workbench_toggles(
        &mut self,
        toggles: WorkbenchSidebarToggles,
        cx: &mut Context<Self>,
    ) {
        self.workbench_toggles = Some(toggles);
        cx.notify();
    }

    /// 注入输入框下方上下文栏的数据源（工作区 / 分支 / Worktree）。
    ///
    /// 注入后底栏出现对应入口；`settle` 之前底栏只有「权限级别」一项。
    pub fn set_composer_context_source(
        &mut self,
        source: ComposerContextSource,
        cx: &mut Context<Self>,
    ) {
        self.composer_context_source = Some(source);
        self.sync_composer(cx);
        cx.notify();
    }

    /// 重新向宿主取一次上下文栏快照（分支/worktree 变化后由宿主调用）。
    pub fn refresh_composer_context(&mut self, cx: &mut Context<Self>) {
        self.sync_composer(cx);
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        // 会话内搜索入口。findbar 是浮层，打开状态本身已经很明显，
        // 这里只用 outline 做一次状态复核（图标按钮没有 selected 语义）。
        let mut search_button = IconButton::new("agent-chat-find", IconName::Search)
            .role(IconButtonRole::Compact)
            .custom(agent_header_icon_variant(&theme, cx))
            .tooltip(t!("AgentUi.find_in_transcript").to_string())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_findbar(window, cx)));
        if self.findbar_open {
            search_button = search_button.outline();
        }
        // `IconButton` 不接受 `debug_selector`，包一层拿测试锚点。
        let search_entry = div()
            .debug_selector(|| "agent-chat-find".to_string())
            .flex_shrink_0()
            .child(search_button);

        // 工作台模式：导航栏开关在 agent 切换器左侧，右侧标签组开关与搜索
        // 同在 trailing；非工作台模式保持原样。
        let (leading, trailing) = match &self.workbench_toggles {
            Some(toggles) => {
                let nav_collapsed = (toggles.nav_collapsed)(cx);
                let toggle_nav = toggles.toggle_nav.clone();
                let toggle_right = toggles.toggle_right.clone();
                let nav_toggle = IconButton::new(
                    "workbench-toolbar-nav-toggle",
                    if nav_collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    },
                )
                .role(IconButtonRole::Compact)
                .tooltip(
                    if nav_collapsed {
                        t!("Workbench.expand_nav")
                    } else {
                        t!("Workbench.collapse_nav")
                    }
                    .to_string(),
                )
                .on_click(move |_, window, cx| (toggle_nav)(window, cx));
                let right_open = (toggles.right_open)(cx);
                let right_toggle = IconButton::new(
                    "workbench-toolbar-right-toggle",
                    if right_open {
                        IconName::PanelRightClose
                    } else {
                        IconName::PanelRightOpen
                    },
                )
                .role(IconButtonRole::Compact)
                .tooltip(
                    if right_open {
                        t!("Workbench.collapse_right_sidebar")
                    } else {
                        t!("Workbench.expand_right_sidebar")
                    }
                    .to_string(),
                )
                .on_click(move |_, window, cx| (toggle_right)(window, cx));
                (
                    Some(nav_toggle.into_any_element()),
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(search_entry)
                        .child(right_toggle)
                        .into_any_element(),
                )
            }
            None => (None, search_entry.into_any_element()),
        };

        let mut header = PanelHeader::new("agent-chat-toolbar")
            .variant(PanelHeaderVariant::Toolbar)
            .horizontal_padding(one_ui::theme_geometry().spacing.space_4)
            .background(theme.background)
            .border_color(theme.border)
            .title(self.render_agent_switcher(cx))
            .trailing(trailing);
        if let Some(leading) = leading {
            header = header.leading(leading);
        }
        header.into_any_element()
    }

    fn render_sidebar_frame_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let placement = self.sidebar_frame_placement;
        IconButton::new("agent-sidebar-frame-options", IconName::Ellipsis)
            .role(IconButtonRole::Compact)
            .custom(agent_header_icon_variant(
                &resolve_agent_chat_theme(self.theme.as_ref(), cx),
                cx,
            ))
            .tooltip(t!("AgentUi.panel_options").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                build_sidebar_frame_options_menu(menu, view.clone(), placement, window, cx)
            })
    }

    fn render_agent_switcher(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = cx.entity();
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let label = current_agent_label(
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
        );
        let trigger = Button::new("agent-header-switcher-btn")
            .small()
            .icon(current_agent_icon(self.backend))
            .label(compact_agent_label(label.as_ref(), 24))
            .outline()
            .dropdown_caret(true)
            .disabled(self.is_running)
            .bg(theme.panel)
            .border_color(theme.border)
            .text_color(theme.foreground);

        Popover::new("agent-header-switcher")
            .anchor(Anchor::TopLeft)
            .p_0()
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    if *open {
                        view.update(cx, |this, cx| this.refresh_acp_agents(cx));
                    }
                }
            })
            .trigger(trigger)
            .content({
                let theme = theme.clone();
                let view_for_content = view.clone();
                move |_state, _window, cx| {
                    let options = view_for_content.read(cx).agent_switcher_options();
                    render_agent_switcher_content(view.clone(), options.clone(), &theme, cx)
                }
            })
            .into_any_element()
    }
}

fn acp_model_option(
    state: &AcpSessionState,
    agent_id: Option<&SharedString>,
) -> Option<ComposerModelOption> {
    let agent_id = agent_id?;
    let option = state.current_model_config()?;
    let agent_client_protocol::schema::v1::SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };
    let value = &select.current_value;
    let selected = select_config_value(&select.options, value)?;
    Some(
        ComposerModelOption::new(
            format!("acp:{}:{}:{}", agent_id, option.id, value),
            agent_id.clone(),
            agent_id.clone(),
            value.to_string(),
        )
        .with_model_only()
        .with_hint(selected),
    )
}

/// 由探测结果构造连接前可用的模型候选。
///
/// 探测拿不到配置项 id，这里用一个稳定的占位段；真正连接后
/// [`acp_model_options`] 会用 agent 给出的配置项 id 覆盖这批选项。
fn acp_model_options_from_probe(
    agent_id: &SharedString,
    models: &[AcpModelInfo],
) -> Vec<ComposerModelOption> {
    models
        .iter()
        .map(|model| {
            ComposerModelOption::new(
                format!("acp:{}:probe:{}", agent_id, model.id),
                agent_id.clone(),
                agent_id.clone(),
                model.id.clone(),
            )
            .with_model_only()
            .with_hint(model.label.clone())
        })
        .collect()
}

fn acp_model_options(
    state: &AcpSessionState,
    agent_id: Option<&SharedString>,
) -> Vec<ComposerModelOption> {
    let Some(agent_id) = agent_id else {
        return Vec::new();
    };
    let Some(config) = state.current_model_config() else {
        return Vec::new();
    };
    state
        .model_options()
        .into_iter()
        .map(|(value, label)| {
            ComposerModelOption::new(
                format!("acp:{}:{}:{}", agent_id, config.id, value),
                agent_id.clone(),
                agent_id.clone(),
                value,
            )
            .with_model_only()
            .with_hint(label)
        })
        .collect()
}

fn select_config_value(
    options: &agent_client_protocol::schema::v1::SessionConfigSelectOptions,
    value: &agent_client_protocol::schema::v1::SessionConfigValueId,
) -> Option<String> {
    use agent_client_protocol::schema::v1::SessionConfigSelectOptions;
    let values = match options {
        SessionConfigSelectOptions::Ungrouped(values) => values,
        SessionConfigSelectOptions::Grouped(groups) => {
            return groups
                .iter()
                .flat_map(|group| group.options.iter())
                .find(|option| option.value == *value)
                .map(|option| option.name.clone());
        }
        _ => return None,
    };
    values
        .iter()
        .find(|option| option.value == *value)
        .map(|option| option.name.clone())
}

/// 把一次探测结论压成切换器副标题。
///
/// 已识别时优先「名称 版本 · N 个模型」；只有鉴权要求时退化为「需要登录」。
/// 失败时只保留错误首行并截断，避免撑破菜单行。
fn probe_status_label(probe: &AcpAgentProbe) -> SharedString {
    if let Some(error) = probe.error.as_deref() {
        return SharedString::from(
            t!(
                "AgentUi.agent_probe_failed",
                error = truncate_probe_error(error)
            )
            .to_string(),
        );
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(name) = probe.name.as_deref().filter(|name| !name.trim().is_empty()) {
        parts.push(
            match probe
                .version
                .as_deref()
                .filter(|version| !version.trim().is_empty())
            {
                Some(version) => format!("{name} {version}"),
                None => name.to_string(),
            },
        );
    }
    if !probe.models.is_empty() {
        parts.push(t!("AgentUi.agent_model_count", count = probe.models.len()).to_string());
    } else if !probe.auth_methods.is_empty() {
        parts.push(t!("AgentUi.agent_login_required").to_string());
    }
    if parts.is_empty() {
        return SharedString::from("ACP Agent");
    }
    SharedString::from(parts.join(" · "))
}

/// 探测错误取首行并按字符上限截断。
fn truncate_probe_error(error: &str) -> String {
    const MAX_CHARS: usize = 80;
    let first_line = error.lines().next().unwrap_or(error).trim();
    if first_line.chars().count() <= MAX_CHARS {
        return first_line.to_string();
    }
    first_line.chars().take(MAX_CHARS).collect::<String>() + "…"
}

impl EventEmitter<AgentChatViewEvent> for AgentChatView {}

/// 当前 Unix 秒；时钟异常时退化为 0（时长会被判为未知而不是负值）。
fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

impl Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 会话切换排队的草稿在第一帧应用（见 `pending_input_draft` 字段注释）。
        self.apply_pending_input_draft(window, cx);
        if self.auto_scroll.take_pending_for_render() {
            self.scroll_handle.scroll_to_bottom();
        }
        // 正文变化（含流式输出）后按当前查询重算命中。
        // 这里刻意用无通知版本：渲染期间 notify 会把「重算 → 通知 → 再渲染」转起来。
        self.refresh_search_if_stale();
        let chat_theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        // 「执行中…」只在**没有**进行中轮次时才挂：轮次页脚已经在显示「进行中」，
        // 两者紧挨着出现就是同一句话说两遍（用户看到的「三个执行状态」）。
        let has_live_turn = self
            .transcript
            .messages
            .iter()
            .any(crate::turn::is_live_message);
        let running_activity =
            (self.is_running && !has_live_turn).then(|| render_running_activity(&chat_theme));

        // 滚动几何观测。「内容变长」不算用户移动阅读位置，否则流式输出会把
        // 跟随态自己关掉（见 `TranscriptScrollState::observe`）。
        let show_scroll_to_latest = self
            .scroll
            .observe(
                self.scroll_handle.offset().y,
                self.scroll_handle.max_offset().y,
            )
            .unwrap_or(false)
            && !self.sidebar_mode;

        let list_layout = if self.sidebar_mode {
            MessageListLayout::EdgeToEdge
        } else {
            MessageListLayout::Centered
        };
        let action_listener = cx.listener(|this, action: &MessageListAction, _window, cx| {
            this.apply_message_list_action(action.clone(), cx);
        });
        let on_action: MessageListActionHandler =
            std::rc::Rc::new(move |action, window, cx| action_listener(&action, window, cx));

        let findbar = self.render_findbar(&chat_theme, cx);
        let messages = render_message_list(
            &self.transcript.messages,
            &self.scroll_handle,
            MessageListContext::new(list_layout)
                .with_activity(running_activity)
                .with_code_actions(Some(&self.code_block_actions))
                .with_theme(Some(&chat_theme))
                .with_expansion(Some(&self.expansion))
                .with_timings(Some(&self.turn_timings))
                .with_action_handler(Some(on_action))
                .with_turn_chrome(!self.sidebar_mode)
                .with_search(Some(&self.search))
                .with_findbar(findbar)
                .with_restorable_turns(self.restorable_turns.get(&self.current_session))
                .with_scroll_to_latest(show_scroll_to_latest),
            window,
            cx,
        );
        let decision_dock = self.render_decision_dock(&chat_theme, cx);
        let input_area = div()
            .id("agent-input-area")
            .debug_selector(|| "agent-input-area".to_string())
            .w_full()
            .min_w_0()
            .when(self.sidebar_mode, |this| {
                this.min_h_0().flex_shrink_1().overflow_y_scroll()
            })
            .when(!self.sidebar_mode, |this| {
                this.flex_shrink_0().overflow_hidden()
            })
            .border_t_1()
            .border_color(chat_theme.border)
            .bg(chat_theme.background)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .when(self.sidebar_mode, |this| this.min_h_0().overflow_hidden())
                    // 决策栏贴输入区顶部、通栏铺开（与内联卡片同一决策源）。
                    .when_some(decision_dock, |this, dock| this.child(dock))
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .when(self.sidebar_mode, |this| this.min_h_0().overflow_hidden())
                            .when(!self.sidebar_mode, |this| this.p_3())
                            .child(self.input.clone()),
                    ),
            );
        let auth_actions = self.render_acp_auth_actions(cx);

        if self.sidebar_mode {
            // 侧边栏视图:紧凑头部(新建对话 / 历史记录) + 消息 + 输入。
            let header = self
                .show_sidebar_header
                .then(|| self.render_sidebar_mode_header(cx));
            div()
                .debug_selector(|| "agent-sidebar-root".to_string())
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .text_color(chat_theme.foreground)
                .bg(chat_theme.background)
                .key_context(AI_CHAT_SEARCH_CONTEXT)
                .on_action(cx.listener(Self::approve_tool_call))
                .on_action(cx.listener(Self::reject_tool_call))
                .on_action(cx.listener(|this, _: &ToggleTranscriptFind, window, cx| {
                    this.open_findbar(window, cx);
                }))
                .on_action(cx.listener(|this, _: &FindNextInTranscript, _, cx| {
                    this.step_search(true, cx);
                }))
                .on_action(cx.listener(|this, _: &FindPreviousInTranscript, _, cx| {
                    this.step_search(false, cx);
                }))
                // 会话导航：cmd-[ / cmd-]（可自定义）与鼠标后退/前进键。
                .on_action(cx.listener(|this, _: &NavigateSessionBack, _, cx| {
                    this.navigate_session_back(cx);
                }))
                .on_action(cx.listener(|this, _: &NavigateSessionForward, _, cx| {
                    this.navigate_session_forward(cx);
                }))
                .on_action(cx.listener(|this, _: &ToggleSessionSwitcher, window, cx| {
                    this.toggle_session_switcher(window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Back),
                    cx.listener(|this, _, _, cx| {
                        this.navigate_session_back(cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Forward),
                    cx.listener(|this, _, _, cx| {
                        this.navigate_session_forward(cx);
                    }),
                )
                // 最近会话切换器 overlay。
                .when_some(
                    self.render_session_switcher(&chat_theme, cx),
                    |root, overlay| root.child(overlay),
                )
                .child(
                    v_flex()
                        .debug_selector(|| "agent-sidebar-stack".to_string())
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .when_some(header, |this, header| this.child(header))
                        .child(messages)
                        .when_some(auth_actions, |this, actions| this.child(actions))
                        .child(input_area),
                )
        } else {
            // 普通全宽视图:常驻左侧会话栏 + 主区(标题 / 消息 / 输入)。
            // 工作台外壳接管会话栏时整块隐藏（`sidebar_suppressed`）。
            let sidebar = (!self.sidebar_suppressed).then(|| self.render_sidebar(cx));
            let toolbar = self.render_toolbar(cx);
            let dropped = self.transcript.dropped_messages();
            let truncation_hint = (dropped > 0).then(|| {
                div()
                    .w_full()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(t!("AgentUi.history_truncated", count = dropped).to_string())
                    .into_any_element()
            });
            div()
                .size_full()
                .text_color(chat_theme.foreground)
                .bg(chat_theme.background)
                .key_context(AI_CHAT_SEARCH_CONTEXT)
                .on_action(cx.listener(Self::approve_tool_call))
                .on_action(cx.listener(Self::reject_tool_call))
                .on_action(cx.listener(|this, _: &ToggleTranscriptFind, window, cx| {
                    this.open_findbar(window, cx);
                }))
                .on_action(cx.listener(|this, _: &FindNextInTranscript, _, cx| {
                    this.step_search(true, cx);
                }))
                .on_action(cx.listener(|this, _: &FindPreviousInTranscript, _, cx| {
                    this.step_search(false, cx);
                }))
                // 会话导航：cmd-[ / cmd-]（可自定义）与鼠标后退/前进键。
                .on_action(cx.listener(|this, _: &NavigateSessionBack, _, cx| {
                    this.navigate_session_back(cx);
                }))
                .on_action(cx.listener(|this, _: &NavigateSessionForward, _, cx| {
                    this.navigate_session_forward(cx);
                }))
                .on_action(cx.listener(|this, _: &ToggleSessionSwitcher, window, cx| {
                    this.toggle_session_switcher(window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Back),
                    cx.listener(|this, _, _, cx| {
                        this.navigate_session_back(cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Forward),
                    cx.listener(|this, _, _, cx| {
                        this.navigate_session_forward(cx);
                    }),
                )
                // 最近会话切换器 overlay。
                .when_some(
                    self.render_session_switcher(&chat_theme, cx),
                    |root, overlay| root.child(overlay),
                )
                .child(
                    h_flex()
                        .size_full()
                        .when_some(sidebar, |this, sidebar| this.child(sidebar))
                        .child(
                            div().flex_1().h_full().min_w_0().child(
                                v_flex()
                                    .size_full()
                                    .child(toolbar)
                                    .when_some(truncation_hint, |this, hint| this.child(hint))
                                    .child(messages)
                                    .when_some(auth_actions, |this, actions| this.child(actions))
                                    .child(input_area),
                            ),
                        ),
                )
        }
    }
}

/// 把宿主快照里的底栏数据覆盖进已构建好的上下文。
///
/// 单独成函数而不是给 [`build_composer_context`] 加参数：那个函数在测试里
/// 有十几处直接调用，多一个「几乎总是默认值」的参数只会污染所有调用点。
fn apply_composer_snapshot(context: &mut AgentComposerContext, snapshot: ComposerContextSnapshot) {
    context.workspace = snapshot.workspace;
    context.workspace_options = snapshot.workspace_options;
    context.branch_options = snapshot.branches;
    context.worktree = snapshot.worktree;
}

fn build_composer_context(
    resources: &ResourceContext,
    selection: ExecutionSelection,
    model: Option<&ComposerModelOption>,
    plan: Option<&PlanCardData>,
    subagents: &[SubAgentCardData],
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
    acp_state: Option<AcpSessionState>,
    available_resources: &[ResourceRef],
    skill_summary: ComposerSkillSummary,
    skill_items: Vec<ComposerSkillItem>,
    local_context_tokens: Option<u64>,
) -> AgentComposerContext {
    let mut context = build_context(resources, selection, model);
    context.resource_source_options = resource_source_options(resources, available_resources);
    context.resource_pool_items = resource_pool_items(resources, available_resources);
    context.skill_summary = skill_summary;
    context.skill_items = skill_items;
    context.plan_items = composer_plan_items(plan);
    context.subagent_items = composer_subagent_items(subagents);
    context.agent_options =
        composer_agent_options(backend, acp_agents, current_acp_id, acp_connecting);
    if backend == Backend::Acp {
        apply_acp_state_to_context(&mut context, acp_state.as_ref());
        // ACP 的「执行模式」是 agent 自己声明的会话模式（如 codex 的 read-only /
        // agent / agent-full-access），比本地那套工具策略更贴近实际语义。
        if let Some(label) = acp_state.as_ref().and_then(acp_mode_label) {
            context.execution_mode_label = SharedString::from(label);
        }
        // 圆环 gauge 与 `acp-usage` scope 同源:scope 报精确读数,圆环给
        // 一眼可见的占用比例。agent 没报窗口(`size == 0`)时给 `None`,
        // 圆环退化成只显示 token 数。
        if let Some(usage) = acp_state.as_ref().and_then(AcpSessionState::usage) {
            let window = (usage.size > 0).then_some(usage.size);
            context.context_usage = Some(
                ContextUsage::new(usage.used, window)
                    .with_cost(usage.cost.as_ref().map(format_acp_cost)),
            );
        }
    } else if let Some(tokens) = local_context_tokens {
        // 本地后端:模型报了计量才展示;窗口大小按模型名尽力解析,查不到就
        // 只显示 token 数,不猜百分比。
        let window = model.and_then(|option| model_context_window(&option.model));
        context.scopes.push(ComposerScope::new(
            "local-usage",
            t!("AgentUi.usage").to_string(),
            format_local_usage(tokens, window),
        ));
        context.context_usage = Some(ContextUsage::new(tokens, window));
    }
    context
}

fn apply_acp_state_to_context(
    context: &mut AgentComposerContext,
    acp_state: Option<&AcpSessionState>,
) {
    context.target = Some(ComposerTarget::new(
        "acp-session",
        acp_state
            .and_then(AcpSessionState::title)
            .map(str::to_string)
            .unwrap_or_else(|| t!("AgentUi.acp_session").to_string()),
        "AI",
        "ACP",
        "Agent Client Protocol",
    ));
    context.scopes = acp_state.map(acp_scopes).unwrap_or_default();
    context.capabilities = acp_state.map(acp_capabilities).unwrap_or_else(|| {
        vec![
            SharedString::from("ACP"),
            SharedString::from(t!("AgentUi.connecting").to_string()),
        ]
    });
}

fn acp_scopes(state: &AcpSessionState) -> Vec<ComposerScope> {
    let mut scopes = Vec::new();
    if let Some(mode) = acp_mode_label(state) {
        scopes.push(ComposerScope::new(
            "acp-mode",
            t!("AgentUi.mode").to_string(),
            mode,
        ));
    }
    if let Some(updated_at) = state.updated_at() {
        scopes.push(ComposerScope::new(
            "acp-updated",
            t!("AgentUi.updated").to_string(),
            updated_at,
        ));
    }
    if let Some(usage) = state.usage() {
        scopes.push(ComposerScope::new(
            "acp-usage",
            t!("AgentUi.usage").to_string(),
            format_acp_usage(usage),
        ));
    }
    scopes
}

/// 用量文案：`used/size tokens`，agent 报了费用就在后面追加。
///
/// 货币代码原样透传（协议给什么写什么，不自己拼符号）；小数位先按 4 位截掉尾随零，
/// 免得到处是 `0.0120`。
fn format_acp_usage(usage: &AcpUsage) -> String {
    let mut text = format!("{}/{} tokens", usage.used, usage.size);
    if let Some(cost) = usage.cost.as_ref() {
        text.push_str(&format!(" · {}", format_acp_cost(cost)));
    }
    text
}

/// 费用文案:`0.0124 USD`。货币代码原样透传(协议给什么写什么,不自己拼
/// 符号);小数位先按 4 位截掉尾随零,免得到处是 `0.0120`。
fn format_acp_cost(cost: &agent_client_protocol::schema::v1::Cost) -> String {
    let amount = format!("{:.4}", cost.amount);
    let amount = amount.trim_end_matches('0').trim_end_matches('.');
    format!("{} {}", amount, cost.currency)
}

/// 把 Acp agent 推送的 `available_commands` 转成 composer 的 `/` 补全项。
///
/// 命令名原样使用（补全插入的就是它）；`input` 只取非空提示，空的当成没有参数说明。
fn slash_commands_from_acp_state(state: &AcpSessionState) -> Vec<SlashCommandItem> {
    state
        .available_commands()
        .iter()
        .map(|command| {
            let item = SlashCommandItem::new(command.name.clone(), command.description.clone());
            match command.input.as_ref() {
                Some(AvailableCommandInput::Unstructured(input)) => {
                    item.with_input_hint(input.hint.clone())
                }
                // 协议枚举标了 non_exhaustive：未来新增的输入形态先当无提示处理。
                _ => item,
            }
        })
        .collect()
}

fn acp_capabilities(state: &AcpSessionState) -> Vec<SharedString> {
    let mut labels = vec![SharedString::from("ACP")];
    labels.extend(acp_agent_capability_labels(state));
    if !state.available_commands().is_empty() {
        labels.push(SharedString::from(format!(
            "{}:{}",
            t!("AgentUi.commands"),
            state.available_commands().len()
        )));
    }
    if !state.config_options().is_empty() {
        labels.push(SharedString::from(format!(
            "{}:{}",
            t!("AgentUi.configuration"),
            state.config_options().len()
        )));
    }
    labels
}

fn acp_agent_capability_labels(state: &AcpSessionState) -> Vec<SharedString> {
    let caps = state.agent_capabilities();
    let session = &caps.session_capabilities;
    let mut labels = Vec::new();
    if caps.load_session {
        labels.push(SharedString::from(t!("AgentUi.load_session").to_string()));
    }
    if session.list.is_some() {
        labels.push(SharedString::from(t!("AgentUi.list_sessions").to_string()));
    }
    if session.resume.is_some() {
        labels.push(SharedString::from(t!("AgentUi.resume").to_string()));
    }
    if session.close.is_some() {
        labels.push(SharedString::from(t!("AgentUi.close_session").to_string()));
    }
    if session.delete.is_some() {
        labels.push(SharedString::from(t!("AgentUi.delete").to_string()));
    }
    labels
}

fn acp_mode_label(state: &AcpSessionState) -> Option<String> {
    state.current_mode_label()
}

/// 由资源上下文构建输入框展示用上下文。
fn build_context(
    resources: &ResourceContext,
    selection: ExecutionSelection,
    model: Option<&ComposerModelOption>,
) -> AgentComposerContext {
    let current = resources.current();
    let target = current.map(target_from_resource);
    let scopes = current
        .map(|r| {
            r.scopes
                .iter()
                .map(|scope| ComposerScope::new(&scope.key, &scope.label, &scope.value))
                .collect()
        })
        .unwrap_or_default();
    let capabilities = current
        .map(|r| {
            vec![
                SharedString::from(t!("AgentUi.target").to_string()),
                SharedString::from(r.kind.as_str().to_string()),
            ]
        })
        .unwrap_or_default();
    AgentComposerContext {
        target,
        resource_pool: resource_pool_summary(resources),
        resource_type_filters: resource_type_filters(resources),
        resource_source_options: Vec::new(),
        resource_pool_items: Vec::new(),
        skill_summary: Default::default(),
        skill_items: Vec::new(),
        scopes,
        capabilities,
        plan_items: Vec::new(),
        subagent_items: Vec::new(),
        agent_options: Vec::new(),
        model: model.map(ComposerModelOption::to_composer_model),
        execution_mode_label: SharedString::from(selection.label()),
        workspace: Default::default(),
        workspace_options: Vec::new(),
        branch_options: Vec::new(),
        worktree: Default::default(),
        context_usage: None,
    }
}

fn composer_plan_items(plan: Option<&PlanCardData>) -> Vec<ComposerPlanItem> {
    plan.map(|plan| {
        plan.steps
            .iter()
            .map(|step| {
                ComposerPlanItem::new(step.title.clone(), step.status.clone()).with_details(
                    step.description.clone(),
                    step.risk.clone(),
                    step.tool.clone().map(SharedString::from),
                )
            })
            .collect()
    })
    .unwrap_or_default()
}

fn composer_subagent_items(subagents: &[SubAgentCardData]) -> Vec<ComposerSubAgentItem> {
    subagents
        .iter()
        .map(|subagent| {
            ComposerSubAgentItem::new(
                subagent.subagent_id.clone(),
                subagent.name.clone(),
                subagent.task.clone(),
                subagent_status_for_composer(subagent),
            )
            .with_summary(subagent.summary.clone())
        })
        .collect()
}

fn subagent_status_for_composer(subagent: &SubAgentCardData) -> &'static str {
    if subagent.running {
        "running"
    } else if subagent.success == Some(false) {
        "failed"
    } else {
        "completed"
    }
}

fn current_agent_icon(backend: Backend) -> Icon {
    if backend == Backend::Acp {
        Icon::new(IconName::Bot)
    } else {
        Icon::new(IconName::AI).color()
    }
}

fn compact_agent_label(label: &str, max_chars: usize) -> SharedString {
    if label.chars().count() <= max_chars {
        return SharedString::from(label.to_string());
    }
    let mut s: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    s.push_str("...");
    SharedString::from(s)
}

fn render_agent_switcher_content(
    view: Entity<AgentChatView>,
    agents: Vec<ComposerAgentOption>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let mut col = v_flex()
        .p_1()
        .gap(sp(2.0))
        .min_w(sp(300.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(header_switcher_group_label("Agent", theme));
    if agents.is_empty() {
        return col
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("AgentUi.no_agents").to_string()),
            )
            .into_any_element();
    }

    for agent in agents {
        col = col.child(header_agent_option_row(
            view.clone(),
            agent,
            muted,
            theme,
            cx,
        ));
    }
    col.into_any_element()
}

fn header_switcher_group_label(label: &'static str, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label)
        .into_any_element()
}

fn header_agent_option_row(
    view: Entity<AgentChatView>,
    agent: ComposerAgentOption,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.foreground;
    let icon_fg = if agent.selected { theme.accent } else { muted };
    let target = agent.id.clone();
    let disabled = agent_option_disabled(&agent);

    h_flex()
        .id(SharedString::from(format!(
            "agent-header-option-{}",
            agent.element_id()
        )))
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .when(agent.selected, |this| this.bg(selected_bg))
        .when(agent.selected, |this| this.text_color(selected_fg))
        .when(disabled, |this| this.opacity(0.5))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _window, cx| {
                    let target = target.clone();
                    view.update(cx, |this, cx| {
                        if !this.is_running {
                            this.select_backend(target, cx);
                        }
                    });
                })
        })
        .child(
            Icon::new(current_agent_icon_for_option(&agent))
                .small()
                .text_color(icon_fg),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().truncate().child(agent.label))
                .child(div().text_xs().text_color(muted).child(agent.subtitle)),
        )
        .when(agent.selected, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(icon_fg))
        })
        .into_any_element()
}

fn current_agent_icon_for_option(agent: &ComposerAgentOption) -> Icon {
    if agent.id.is_some() {
        Icon::new(IconName::Bot)
    } else {
        Icon::new(IconName::AI).color()
    }
}

fn resource_pool_summary(resources: &ResourceContext) -> ComposerResourcePoolSummary {
    let current = resources.current();
    ComposerResourcePoolSummary::new(
        current.map(|resource| SharedString::from(resource.id.as_str().to_string())),
        current
            .map(|resource| resource.label.clone())
            .unwrap_or_else(|| t!("AgentUi.no_default_target").to_string()),
        resources.resources.len(),
    )
}

fn resource_type_filters(resources: &ResourceContext) -> Vec<ComposerResourceTypeFilter> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for resource in &resources.resources {
        *counts
            .entry(resource.kind.as_str().to_string())
            .or_default() += 1;
    }

    let mut filters = vec![ComposerResourceTypeFilter::new(
        "all",
        t!("AgentUi.all").to_string(),
        resources.resources.len(),
        true,
    )];
    filters.extend(counts.into_iter().map(|(kind, count)| {
        ComposerResourceTypeFilter::new(kind.clone(), kind.to_uppercase(), count, false)
    }));
    filters
}

fn resource_source_options(
    pool: &ResourceContext,
    catalog: &[ResourceRef],
) -> Vec<ComposerResourceSourceOption> {
    let pool_ids = resource_id_set(&pool.resources);
    let catalog_ids = resource_id_set(catalog);
    let current_selected = pool.resources.len() == 1
        && pool
            .current
            .as_ref()
            .is_some_and(|current| Some(current) == pool.resources.first().map(|r| &r.id));
    let all_selected = !current_selected && !catalog_ids.is_empty() && pool_ids == catalog_ids;
    let ssh_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Ssh));
    let db_ids = source_ids(catalog, is_database_kind);
    let redis_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Redis));
    let terminal_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Terminal));
    let source_selected = |ids: &std::collections::HashSet<ResourceId>| {
        !current_selected && !all_selected && !ids.is_empty() && pool_ids == *ids
    };
    let type_selected = source_selected(&ssh_ids)
        || source_selected(&db_ids)
        || source_selected(&redis_ids)
        || source_selected(&terminal_ids);
    let manual_selected = !current_selected && !all_selected && !type_selected;

    vec![
        ComposerResourceSourceOption::new(
            "current",
            t!("AgentUi.current").to_string(),
            current_count(pool),
            current_selected,
        ),
        ComposerResourceSourceOption::new(
            "pool",
            t!("AgentUi.resource_pool").to_string(),
            pool.resources.len(),
            false,
        ),
        ComposerResourceSourceOption::new(
            "all",
            t!("AgentUi.all").to_string(),
            catalog.len(),
            all_selected,
        ),
        ComposerResourceSourceOption::new("ssh", "SSH", ssh_ids.len(), source_selected(&ssh_ids)),
        ComposerResourceSourceOption::new("db", "DB", db_ids.len(), source_selected(&db_ids)),
        ComposerResourceSourceOption::new(
            "redis",
            "Redis",
            redis_ids.len(),
            source_selected(&redis_ids),
        ),
        ComposerResourceSourceOption::new(
            "terminal",
            "Terminal",
            terminal_ids.len(),
            source_selected(&terminal_ids),
        ),
        ComposerResourceSourceOption::new(
            "manual",
            t!("AgentUi.manual").to_string(),
            pool.resources.len(),
            manual_selected,
        ),
        ComposerResourceSourceOption::new(
            "workspace",
            t!("AgentUi.workspace").to_string(),
            0,
            false,
        )
        .disabled(t!("AgentUi.no_workspace_source").to_string()),
        ComposerResourceSourceOption::new("tag", t!("AgentUi.tag").to_string(), 0, false)
            .disabled(t!("AgentUi.no_tag_source").to_string()),
    ]
}

fn resource_id_set(resources: &[ResourceRef]) -> std::collections::HashSet<ResourceId> {
    resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect()
}

fn source_ids(
    catalog: &[ResourceRef],
    predicate: fn(&ResourceKind) -> bool,
) -> std::collections::HashSet<ResourceId> {
    catalog
        .iter()
        .filter(|resource| predicate(&resource.kind))
        .map(|resource| resource.id.clone())
        .collect()
}

fn is_database_kind(kind: &ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite | ResourceKind::Mongo
    )
}

fn current_count(pool: &ResourceContext) -> usize {
    usize::from(pool.current().is_some())
}

fn resource_pool_items(
    pool: &ResourceContext,
    catalog: &[ResourceRef],
) -> Vec<ComposerResourcePoolItem> {
    let pool_ids = pool
        .resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let default_id = pool.current.clone();

    catalog
        .iter()
        .map(|resource| {
            let in_pool = pool_ids.contains(&resource.id);
            let is_default = default_id.as_ref() == Some(&resource.id);
            ComposerResourcePoolItem::new(
                resource.id.as_str().to_string(),
                resource.label.clone(),
                kind_icon(&resource.kind),
                resource.kind.as_str().to_string(),
                resource_primary_meta(resource),
                resource_pool_status(in_pool),
                resource_default_reason(is_default),
                resource.capabilities.len(),
                in_pool,
                is_default,
            )
        })
        .collect()
}

fn resource_primary_meta(resource: &ResourceRef) -> String {
    first_visible_alias(&resource.aliases)
        .or_else(|| {
            resource
                .scopes
                .first()
                .map(|scope| format!("{}: {}", scope.label, scope.value))
        })
        .unwrap_or_else(|| resource.kind.as_str().to_string())
}

fn resource_pool_status(in_pool: bool) -> String {
    if in_pool {
        t!("AgentUi.joined").to_string()
    } else {
        t!("AgentUi.available_to_add").to_string()
    }
}

fn resource_default_reason(is_default: bool) -> Option<String> {
    is_default.then(|| t!("AgentUi.default_target").to_string())
}

fn refresh_pool_resource_metadata(pool: &mut ResourceContext, catalog: &[ResourceRef]) -> bool {
    let mut changed = false;
    for resource in &mut pool.resources {
        let Some(updated) = catalog
            .iter()
            .find(|candidate| candidate.id == resource.id)
            .cloned()
        else {
            continue;
        };
        if *resource != updated {
            *resource = updated;
            changed = true;
        }
    }
    changed
}

fn add_resource_to_pool(pool: &mut ResourceContext, catalog: &[ResourceRef], id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    if pool.get(&rid).is_some() {
        return false;
    }
    let Some(resource) = catalog.iter().find(|resource| resource.id == rid).cloned() else {
        return false;
    };
    pool.resources.push(resource);
    if pool.current.is_none() {
        pool.current = Some(rid);
    }
    true
}

fn apply_mentioned_resources(
    pool: &mut ResourceContext,
    catalog: &[ResourceRef],
    mentions: &[MentionItem],
) -> bool {
    let mut changed = false;
    let mut first_mentioned_id: Option<ResourceId> = None;
    for mention in mentions {
        let rid = ResourceId::new(mention.id.clone());
        if first_mentioned_id.is_none() {
            first_mentioned_id = Some(rid.clone());
        }
        if pool.get(&rid).is_some() {
            continue;
        }
        if let Some(resource) = catalog.iter().find(|resource| resource.id == rid).cloned() {
            pool.resources.push(resource);
            changed = true;
        }
    }
    if let Some(id) = first_mentioned_id.filter(|id| pool.get(id).is_some()) {
        if pool.current.as_ref() != Some(&id) {
            pool.current = Some(id);
            changed = true;
        }
    }
    changed
}

fn remove_resource_from_pool(pool: &mut ResourceContext, id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    let before = pool.resources.len();
    pool.resources.retain(|resource| resource.id != rid);
    if pool.resources.len() == before {
        return false;
    }
    if pool.current.as_ref() == Some(&rid) {
        pool.current = pool.resources.first().map(|resource| resource.id.clone());
    }
    true
}

fn apply_resource_source(pool: &mut ResourceContext, catalog: &[ResourceRef], id: &str) -> bool {
    let resources = match id {
        "current" => pool.current().cloned().map(|resource| vec![resource]),
        "all" => Some(catalog.to_vec()),
        "ssh" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Ssh)
        })),
        "db" => Some(resources_matching(catalog, is_database_kind)),
        "redis" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Redis)
        })),
        "terminal" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Terminal)
        })),
        "pool" | "manual" | "workspace" | "tag" => None,
        _ => None,
    };
    let Some(resources) = resources else {
        return false;
    };
    replace_pool_resources(pool, resources)
}

fn resources_matching(
    catalog: &[ResourceRef],
    predicate: fn(&ResourceKind) -> bool,
) -> Vec<ResourceRef> {
    catalog
        .iter()
        .filter(|resource| predicate(&resource.kind))
        .cloned()
        .collect()
}

fn replace_pool_resources(pool: &mut ResourceContext, resources: Vec<ResourceRef>) -> bool {
    if resources.is_empty() {
        return false;
    }
    let next_current = pool
        .current
        .clone()
        .filter(|id| resources.iter().any(|resource| resource.id == *id))
        .or_else(|| resources.first().map(|resource| resource.id.clone()));
    let changed = pool.resources != resources || pool.current != next_current;
    if changed {
        pool.resources = resources;
        pool.current = next_current;
    }
    changed
}

fn target_from_resource(r: &ResourceRef) -> ComposerTarget {
    ComposerTarget::new(
        r.id.as_str().to_string(),
        r.label.clone(),
        kind_icon(&r.kind),
        r.kind.as_str().to_string(),
        format!("{} · {}", r.kind.as_str(), r.id),
    )
}

fn kind_icon(kind: &ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite => "DB",
        ResourceKind::Ssh => "SH",
        ResourceKind::Redis => "RD",
        ResourceKind::Mongo => "MG",
        ResourceKind::Terminal => "TM",
        ResourceKind::Other(kind) => match kind.as_str() {
            "rdp" => "RD",
            "vnc" => "VN",
            "port-forwarding" => "PF",
            _ => "OT",
        },
    }
}

/// 「问答」是一个任务类型而不是工具策略，两者共用同一个下拉 id 空间。
fn task_kind_from_id(id: &str) -> TaskKind {
    match id {
        "ask" => TaskKind::Ask,
        "plan" => TaskKind::Plan,
        _ => TaskKind::Agent,
    }
}

fn task_kind_from_settings(mode: AiChatToolExecutionMode) -> TaskKind {
    match mode {
        AiChatToolExecutionMode::Ask => TaskKind::Ask,
        _ => TaskKind::Agent,
    }
}

fn tool_execution_mode_from_id(id: &str) -> ToolExecutionMode {
    match id {
        "auto" => ToolExecutionMode::Auto,
        "readonly" => ToolExecutionMode::ReadOnly,
        _ => ToolExecutionMode::Manual,
    }
}

fn runtime_tool_execution_mode(mode: AiChatToolExecutionMode) -> ToolExecutionMode {
    match mode {
        AiChatToolExecutionMode::Auto => ToolExecutionMode::Auto,
        AiChatToolExecutionMode::ReadOnly => ToolExecutionMode::ReadOnly,
        AiChatToolExecutionMode::Manual => ToolExecutionMode::Manual,
        // 「问答」不下发工具，工具策略用不上；取最保守的一档，避免任何意外暴露。
        AiChatToolExecutionMode::Ask => ToolExecutionMode::Manual,
    }
}

/// 把执行模式下拉选中的项落盘到设置。
///
/// 任务类型优先：「问答」有独立的设置档位；其余情况按工具策略存。
fn settings_execution_mode(selection: ExecutionSelection) -> AiChatToolExecutionMode {
    match selection.task {
        TaskKind::Ask => AiChatToolExecutionMode::Ask,
        _ => settings_tool_execution_mode(selection.tool),
    }
}

fn settings_tool_execution_mode(mode: ToolExecutionMode) -> AiChatToolExecutionMode {
    match mode {
        ToolExecutionMode::Auto => AiChatToolExecutionMode::Auto,
        ToolExecutionMode::ReadOnly => AiChatToolExecutionMode::ReadOnly,
        ToolExecutionMode::Manual => AiChatToolExecutionMode::Manual,
    }
}

fn tool_execution_mode_label(mode: ToolExecutionMode) -> String {
    match mode {
        ToolExecutionMode::Auto => t!("AgentUi.auto").to_string(),
        ToolExecutionMode::ReadOnly => t!("AgentUi.readonly").to_string(),
        ToolExecutionMode::Manual => t!("AgentUi.manual_confirmation").to_string(),
    }
}

fn static_runtime_model_option(runtime: &Runtime) -> ComposerModelOption {
    let model = runtime.services().model.model_name().to_string();
    ComposerModelOption::new(
        "runtime:current",
        "runtime",
        t!("AgentUi.current_runtime").to_string(),
        model,
    )
    .with_hint(t!("AgentUi.fixed_runtime").to_string())
}

fn selected_model_from_config(config: &AgentChatViewConfig) -> Option<ComposerModelOption> {
    config
        .selected_model_id
        .as_ref()
        .and_then(|id| config.model_options.iter().find(|m| &m.id == id))
        .cloned()
        .or_else(|| config.model_options.first().cloned())
}

fn refreshed_model_selection(
    previous_id: Option<&SharedString>,
    selected_model_id: Option<&SharedString>,
    model_options: &[ComposerModelOption],
) -> (Option<ComposerModelOption>, Option<ComposerModelOption>) {
    let retained = previous_id
        .and_then(|id| model_options.iter().find(|model| &model.id == id))
        .cloned();
    let selected = retained
        .clone()
        .or_else(|| {
            selected_model_id
                .and_then(|id| model_options.iter().find(|model| &model.id == id))
                .cloned()
        })
        .or_else(|| model_options.first().cloned());
    (selected, retained)
}

fn runtime_specs_from_provider_configs(
    provider_configs: Vec<ProviderConfig>,
    registry: ToolRegistry,
) -> anyhow::Result<Vec<RuntimeBuildSpec>> {
    let mut specs = Vec::new();
    for config in provider_configs.into_iter().filter(|config| config.enabled) {
        let provider: Arc<dyn LlmProvider> = Arc::new(LlmConnector::from_config(&config)?);
        specs.extend(runtime_specs_for_provider_config(
            &config,
            provider,
            registry.clone(),
        ));
    }
    Ok(specs)
}

async fn runtime_specs_from_provider_state(
    provider_configs: Vec<ProviderConfig>,
    registry: ToolRegistry,
    provider_state: GlobalProviderState,
) -> anyhow::Result<Vec<RuntimeBuildSpec>> {
    let mut specs = Vec::new();
    for config in provider_configs.into_iter().filter(|config| config.enabled) {
        let provider = provider_state.manager().get_provider(&config).await?;
        specs.extend(runtime_specs_for_provider_config(
            &config,
            provider,
            registry.clone(),
        ));
    }
    Ok(specs)
}

fn runtime_specs_for_provider_config(
    config: &ProviderConfig,
    provider: Arc<dyn LlmProvider>,
    registry: ToolRegistry,
) -> Vec<RuntimeBuildSpec> {
    provider_models(config)
        .into_iter()
        .map(|model| {
            let option = ComposerModelOption::new(
                provider_model_option_id(config.id, &model),
                config.id.to_string(),
                provider_label(config),
                model.clone(),
            )
            .with_hint(format!(
                "{} · {}",
                config.provider_type.display_name(),
                t!("AgentUi.official_model")
            ));
            RuntimeBuildSpec {
                option,
                provider: provider.clone(),
                model: model.clone(),
                registry: registry.clone(),
                temperature: config.temperature,
                max_tokens: config.max_tokens.and_then(|v| u32::try_from(v).ok()),
                is_default: config.is_default && model == config.model,
            }
        })
        .collect()
}

fn provider_models(config: &ProviderConfig) -> Vec<String> {
    let mut models = Vec::new();
    if !config.model.is_empty() {
        models.push(config.model.clone());
    }
    for model in &config.models {
        if !model.is_empty() && !models.contains(model) {
            models.push(model.clone());
        }
    }
    models
}

fn provider_model_option_id(provider_id: i64, model: &str) -> String {
    format!("provider:{provider_id}:{model}")
}

fn provider_label(config: &ProviderConfig) -> String {
    if config.name.is_empty() {
        config.provider_type.display_name().to_string()
    } else {
        config.name.clone()
    }
}

fn selected_provider_model_id(specs: &[RuntimeBuildSpec]) -> Option<SharedString> {
    specs
        .iter()
        .find(|spec| spec.is_default)
        .or_else(|| specs.first())
        .map(|spec| spec.option.id.clone())
}

/// 本地后端的执行模式选项。
///
/// 「问答」是任务类型（不向模型暴露工具），其余三项是工具策略——两者共用一个下拉，
/// 与合并后的单一执行模式控件保持一致。
fn default_execution_mode_options() -> Vec<ComposerMenuOption> {
    vec![
        ComposerMenuOption::new("ask", t!("AgentUi.ask_mode").to_string())
            .with_hint(t!("AgentUi.ask_mode_hint").to_string()),
        ComposerMenuOption::new("auto", t!("AgentUi.auto").to_string()),
        ComposerMenuOption::new("readonly", t!("AgentUi.readonly").to_string()),
        ComposerMenuOption::new("manual", t!("AgentUi.manual_confirmation").to_string()),
    ]
}

/// ACP 后端的执行模式选项。
///
/// 两套来源：传统 `SessionModeState.available_modes`（waku/codex 等），以及
/// `SessionConfigOption{category=mode}`（opencode 等新式 agent）。两者只会有一套非空，
/// 优先传统那套以保持既有行为。
fn acp_execution_mode_options(state: &AcpSessionState) -> Vec<ComposerMenuOption> {
    let legacy = legacy_acp_mode_options(state);
    if !legacy.is_empty() {
        return legacy;
    }
    acp_config_mode_options(state)
}

/// 传统 `available_modes` 的下拉项。
fn legacy_acp_mode_options(state: &AcpSessionState) -> Vec<ComposerMenuOption> {
    state
        .available_modes()
        .iter()
        .map(|mode| {
            let option =
                ComposerMenuOption::new(acp_mode_option_id(mode.id.0.as_ref()), mode.name.clone());
            match mode.description.as_deref() {
                Some(description) => option.with_hint(description.to_string()),
                None => option,
            }
        })
        .collect()
}

/// `SessionConfigOption{category=mode}` 的下拉项。
fn acp_config_mode_options(state: &AcpSessionState) -> Vec<ComposerMenuOption> {
    state
        .mode_options()
        .into_iter()
        .map(|(value, name, description)| {
            let option = ComposerMenuOption::new(acp_config_mode_option_id(&value), name);
            match description {
                Some(description) => option.with_hint(description),
                None => option,
            }
        })
        .collect()
}

const ACP_MODE_OPTION_PREFIX: &str = "acp-mode:";

fn acp_mode_option_id(mode_id: &str) -> String {
    format!("{ACP_MODE_OPTION_PREFIX}{mode_id}")
}

const ACP_CONFIG_MODE_OPTION_PREFIX: &str = "acp-config-mode:";

fn acp_config_mode_option_id(value: &str) -> String {
    format!("{ACP_CONFIG_MODE_OPTION_PREFIX}{value}")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_cards::{
        ACP_PERMISSION_CARD, AcpPermissionCardData, AcpPermissionOptionData, TOOL_CARD,
        TOOL_CONFIRM_CARD, ToolCardData, ToolConfirmCardData,
    };
    use crate::find_shortcut::CloseTranscriptFind;
    use crate::{
        AcpAgentConfig, AcpAgentEntry, AcpConfigDiagnostic, AcpElicitationField,
        AcpElicitationFieldKind, AcpElicitationForm, AcpElicitationMode, AcpElicitationOption,
        AcpElicitationOutcome, AcpElicitationRequest, AcpPermissionOption, AcpPermissionRequest,
        AcpPublicMcpApprovalRequest,
    };
    use agent_runtime::RuntimeServices;
    use agent_runtime::model::MockModelClient;
    use agent_runtime::model::function_tool_call;
    use agent_runtime::model::{ModelClient, ModelRequest, ModelResponse, ModelStream};
    use agent_runtime::tools::ToolInvocation;
    use agent_runtime::tools::builtin::EchoTool;
    use agent_runtime::{
        ObservationData, RiskLevel, Tool, ToolError, ToolName, ToolObservation, ToolRegistry,
        ToolRouter, ToolSpec,
    };
    use async_trait::async_trait;
    use gpui::{
        Entity, IntoElement, Modifiers, ParentElement, Pixels, Render, ScrollDelta,
        ScrollWheelEvent, Styled, TestAppContext, TouchPhase, VisualTestContext, Window, div,
        point, px,
    };
    use one_core::llm::{ProviderConfig, ProviderType};
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct WriteTool;

    struct FixedSidebarHost {
        view: Entity<AgentChatView>,
        height: Pixels,
    }

    fn test_acp_permission_request() -> AcpPermissionRequest {
        AcpPermissionRequest {
            request_id: "session:call".into(),
            session_id: "session".into(),
            tool_call_id: "call".into(),
            tool_name: "Write file".into(),
            summary: "ACP Agent 请求执行工具：Write file".into(),
            details: json!({"path": "/tmp/a"}),
            options: vec![
                AcpPermissionOption {
                    option_id: "reject".into(),
                    name: "拒绝".into(),
                    kind: "reject_once".into(),
                },
                AcpPermissionOption {
                    option_id: "allow".into(),
                    name: "仅本次允许".into(),
                    kind: "allow_once".into(),
                },
            ],
        }
    }

    fn test_acp_elicitation_request() -> AcpElicitationRequest {
        AcpElicitationRequest {
            request_id: "elicitation-test".into(),
            session_id: "session".into(),
            message: "要部署到哪个环境？".into(),
            mode: AcpElicitationMode::Form(AcpElicitationForm {
                title: Some("部署目标".into()),
                description: None,
                fields: vec![
                    AcpElicitationField {
                        name: "env".into(),
                        title: "环境".into(),
                        description: None,
                        required: true,
                        default: None,
                        kind: AcpElicitationFieldKind::SingleSelect {
                            options: vec![
                                AcpElicitationOption {
                                    value: "prod".into(),
                                    title: "生产".into(),
                                },
                                AcpElicitationOption {
                                    value: "staging".into(),
                                    title: "预发".into(),
                                },
                            ],
                        },
                    },
                    AcpElicitationField {
                        name: "confirm".into(),
                        title: "同时刷新缓存".into(),
                        description: None,
                        required: false,
                        default: None,
                        kind: AcpElicitationFieldKind::Boolean,
                    },
                ],
            }),
        }
    }

    fn pending_submission(text: &str) -> crate::pending_submission::PendingSubmission {
        crate::pending_submission::PendingSubmission {
            text: text.to_string(),
            mentions: Vec::new(),
            images: Vec::new(),
        }
    }

    #[test]
    fn runtime_event_batch_drains_only_the_bounded_ready_prefix() {
        let (tx, mut rx) =
            tokio::sync::broadcast::channel(MAX_RUNTIME_EVENT_BATCH_SIZE.saturating_add(2));
        let session_id = SessionId::from_string("batch-session");
        let turn_id = TurnId::from_string("batch-turn");

        for index in 0..=MAX_RUNTIME_EVENT_BATCH_SIZE {
            tx.send(RuntimeEvent::AssistantMessageDelta {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                delta: index.to_string(),
            })
            .unwrap();
        }

        let first = rx.try_recv().unwrap();
        let batch = collect_ready_runtime_events(&mut rx, first, None);

        assert_eq!(MAX_RUNTIME_EVENT_BATCH_SIZE, batch.events.len());
        assert_eq!(0, batch.skipped, "没丢事件时不该报 lag");
        assert!(
            rx.try_recv().is_ok(),
            "batch must leave the overflow queued"
        );
    }

    /// 通道挤掉事件时必须把条数报上来。
    ///
    /// 这是丢终态能不能被补救的**唯一**入口：条数没报出来，上层就以为一切正常，
    /// 界面会停在「正在响应」而日志里一句都没有。
    #[test]
    fn runtime_event_batch_reports_the_events_the_channel_dropped() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(2);
        let session_id = SessionId::from_string("lag-session");
        let turn_id = TurnId::from_string("lag-turn");

        // 容量 2，先塞满再把接收端甩开 3 条，接收端必然 lag。
        for index in 0..5 {
            tx.send(RuntimeEvent::AssistantMessageDelta {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                delta: index.to_string(),
            })
            .unwrap();
        }

        // 第一条 `recv` 就会撞上 lag：丢的是最旧的那批。
        let Err(TryRecvError::Lagged(skipped)) = rx.try_recv() else {
            panic!("expected the receiver to lag behind a full channel");
        };
        assert!(skipped > 0, "lag 必须报出被挤掉的条数");

        let next = rx.try_recv().expect("lag 之后仍应能取到还留着的事件");
        let batch = collect_ready_runtime_events(&mut rx, next, Some(&session_id));
        assert!(
            !batch.events.is_empty(),
            "lag 之后游标会挪到最旧的可读位置，接着取仍是本会话的事件"
        );
    }

    /// 终态被通道挤掉之后，「正在响应」必须能被收回来。
    ///
    /// 复现形状：视图相信会话在跑（`running_sessions` 里有它），但 runtime 那边
    /// 这一轮早就结束了——区别只在于清 running 的那个终态事件被挤掉了。没有重同步
    /// 的话界面就永久停在「正在响应」，而日志里只有驱动侧那行，看不出事件没到。
    #[gpui::test]
    fn dropped_terminal_event_releases_a_session_the_driver_already_finished(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.set_session_running(&session_uid, true, cx);
            assert!(view.running_sessions.contains(&session_uid));

            // 前置条件：runtime 里这个会话确实在，而且没有在跑的轮次。
            // 也就是说视图的 running 是**残留**，而不是 runtime 还在忙。
            assert!(
                !view
                    .runtime
                    .session(&view.session_id)
                    .expect("view's session must be registered with the runtime")
                    .is_busy()
            );

            view.on_runtime_events_dropped(7, cx);

            assert!(
                !view.running_sessions.contains(&session_uid),
                "runtime 侧已经没有在跑的轮次：running 是终态被丢掉的残留，必须收回"
            );
            assert!(!view.is_running, "当前会话的输入框也要跟着退出运行态");
        });
    }

    /// 问不出结论时**不许**猜：不在 runtime 里的会话不能被误清。
    ///
    /// 误清的代价比多转一会儿大得多——用户会以为那一轮结束了，对着一个其实还在
    /// 跑的会话继续追问。
    #[gpui::test]
    fn dropped_terminal_event_keeps_a_session_the_driver_cannot_be_asked_about(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = "history-only-session".to_string();
            view.set_session_running(&session_uid, true, cx);

            view.on_runtime_events_dropped(3, cx);

            assert!(
                view.running_sessions.contains(&session_uid),
                "runtime 里没有这个会话、问不出结论：宁可多转一会儿，也不能误清"
            );
        });
    }

    /// ACP 侧同理：连接已经收掉时没有可问的对象，不能凭「没有连接」就断定它跑完了。
    #[gpui::test]
    fn dropped_terminal_event_keeps_an_acp_turn_without_a_connection(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            let session_uid = "acp-owner-session".to_string();
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: SessionId::from_string("acp:owner"),
                session_uid: session_uid.clone(),
                turn_id: TurnId::from_string("turn-owner"),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.set_session_running(&session_uid, true, cx);

            view.on_runtime_events_dropped(2, cx);

            assert!(view.running_sessions.contains(&session_uid));
            assert!(
                !view.acp_turn_owners.is_empty(),
                "没确证结束就不该动 owner —— 动了等于把这一轮白送给下一条消息"
            );
        });
    }

    /// steer：ACP 前台轮次在飞时提交新消息，不排队、直接插入在飞轮次。
    /// 连接不在时退回排队语义（RetryLater），消息不丢。
    #[gpui::test]
    fn steering_submission_while_acp_turn_in_flight(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            let session_uid = view.current_session.clone();
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: SessionId::from_string("acp:steer"),
                session_uid: session_uid.clone(),
                turn_id: TurnId::from_string("turn-steer"),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.set_session_running(&session_uid, true, cx);

            // 无连接：steer 发送会失败退回排队，但用户消息与插话提示都已落转录。
            view.submit(
                "插一句：别忘了看并发测试".to_string(),
                Vec::new(),
                Vec::new(),
                cx,
            );

            assert!(
                view.acp_turn_owners
                    .iter()
                    .any(|owner| owner.turn_id.as_str() == "turn-steer"),
                "原在飞轮次的 owner 不受插话影响"
            );
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content.contains("插一句")),
                "插话消息应落进当前转录"
            );
        });
    }

    /// 终态收尾不能被转录去重挡掉。
    ///
    /// 复现形状：同一个终态来了两次（直播一次、迟到的重放再来一次）。第二次转录
    /// 会判重、不再写入内容，但「这一轮结束了」这件事是**每次都要落地**的。收尾
    /// 一旦被 `applied == false` 挡掉，界面就永久停在「正在响应」——而且因为转录里
    /// 确实有内容，看起来完全不像卡住。
    #[gpui::test]
    fn a_duplicate_terminal_event_still_releases_running(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            let session_id = view.session_id.clone();
            let turn_id = TurnId::from_string("duplicate-terminal-turn");
            let terminal = || RuntimeEvent::TurnFailed {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                reason: "boom".to_string(),
            };

            view.set_session_running(&session_uid, true, cx);
            view.apply_runtime_event(terminal(), cx);
            assert!(
                !view.running_sessions.contains(&session_uid),
                "第一次终态就该清掉 running"
            );

            // 同一个终态再来一次：转录判重，但收尾照样要跑。
            view.set_session_running(&session_uid, true, cx);
            view.apply_runtime_event(terminal(), cx);

            assert!(
                !view.running_sessions.contains(&session_uid),
                "重复的终态也要清 running —— 收尾不该由转录去重来把关"
            );
        });
    }

    #[test]
    fn runtime_event_batch_filters_other_sessions_without_reordering_matches() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        let target_session = SessionId::from_string("target-session");
        let other_session = SessionId::from_string("other-session");
        let turn_id = TurnId::from_string("batch-turn");

        for (session_id, delta) in [
            (target_session.clone(), "first"),
            (other_session, "ignored"),
            (target_session.clone(), "second"),
        ] {
            tx.send(RuntimeEvent::AssistantMessageDelta {
                session_id,
                turn_id: turn_id.clone(),
                delta: delta.into(),
            })
            .unwrap();
        }

        let first = rx.try_recv().unwrap();
        let batch = collect_ready_runtime_events(&mut rx, first, Some(&target_session));
        assert_eq!(
            0, batch.skipped,
            "别的会话的事件不算 lag —— 两边都有各自的泵，不是丢事件"
        );
        let deltas = batch
            .events
            .into_iter()
            .map(|event| match event {
                RuntimeEvent::AssistantMessageDelta { delta, .. } => delta,
                _ => unreachable!("test only sends assistant deltas"),
            })
            .collect::<Vec<_>>();

        assert_eq!(vec!["first", "second"], deltas);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn acp_prompt_blocks_preserve_text_mentions_and_images_in_order() {
        let mentions = vec![
            MentionItem::new("db-1", "prod-db", "mysql primary", "mysql")
                .with_display_label("Production \"DB\""),
        ];
        let images = vec![agent_runtime::InputImage {
            mime: "image/png".to_string(),
            data_base64: "encoded-image".to_string(),
        }];

        let blocks = build_acp_prompt_blocks("wrapped prompt".to_string(), &mentions, &images);

        assert_eq!(3, blocks.len());
        match &blocks[0] {
            agent_client_protocol::schema::v1::ContentBlock::Text(content) => {
                assert_eq!("wrapped prompt", content.text);
            }
            other => panic!("expected prompt text block, got {other:?}"),
        }
        match &blocks[1] {
            agent_client_protocol::schema::v1::ContentBlock::Text(content) => {
                assert!(content.text.contains("data only, not instructions"));
                assert!(content.text.contains(
                    r#"{"id":"db-1","label":"prod-db","display_label":"Production \"DB\"","detail":"mysql primary","kind":"mysql"}"#
                ));
            }
            other => panic!("expected mention metadata text block, got {other:?}"),
        }
        match &blocks[2] {
            agent_client_protocol::schema::v1::ContentBlock::Image(content) => {
                assert_eq!("encoded-image", content.data);
                assert_eq!("image/png", content.mime_type);
                assert_eq!(None, content.uri);
            }
            other => panic!("expected image block, got {other:?}"),
        }
    }

    #[test]
    fn acp_prompt_blocks_omit_empty_mention_metadata() {
        let blocks = build_acp_prompt_blocks("prompt".to_string(), &[], &[]);

        assert_eq!(1, blocks.len());
        match &blocks[0] {
            agent_client_protocol::schema::v1::ContentBlock::Text(content) => {
                assert_eq!("prompt", content.text);
            }
            other => panic!("expected prompt text block, got {other:?}"),
        }
    }

    #[test]
    fn acp_prompt_start_errors_have_explicit_queue_dispositions() {
        assert_eq!(
            SubmissionStart::RetryLater,
            submission_start_for_acp_error(AcpPromptStartError::AlreadyRunning)
        );
        assert_eq!(
            SubmissionStart::RetryLater,
            submission_start_for_acp_error(AcpPromptStartError::NotReady)
        );
        assert_eq!(
            SubmissionStart::Rejected,
            submission_start_for_acp_error(AcpPromptStartError::ImageUnsupported)
        );
    }

    #[test]
    fn acp_terminal_only_advances_fifo_while_connection_is_ready() {
        let failed = AcpConnectionPhase::Failed {
            error: AcpError::new(
                AcpErrorKind::ConnectionClosed,
                "agent",
                "Agent",
                "connection failed",
            ),
        };
        let running = AcpConnectionPhase::RunningTurn {
            turn_id: TurnId::from_string("turn"),
        };

        assert!(acp_terminal_allows_queue_advance(Some(
            &AcpConnectionPhase::Ready
        )));
        assert!(!acp_terminal_allows_queue_advance(Some(&failed)));
        assert!(!acp_terminal_allows_queue_advance(Some(
            &AcpConnectionPhase::Closed
        )));
        assert!(!acp_terminal_allows_queue_advance(Some(&running)));
        assert!(!acp_terminal_allows_queue_advance(None));

        assert!(acp_connection_is_unavailable(Some(&failed)));
        assert!(acp_connection_is_unavailable(Some(
            &AcpConnectionPhase::Closed
        )));
        assert!(!acp_connection_is_unavailable(Some(
            &AcpConnectionPhase::Ready
        )));
    }

    #[test]
    fn acp_availability_distinguishes_temporary_and_terminal_unavailability() {
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(false, true, false, false, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(false, false, true, false, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(false, false, false, true, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(true, false, false, true, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(true, true, false, false, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(true, false, true, false, false)
        );
        assert_eq!(
            Some(SubmissionStart::RetryLater),
            submission_start_for_acp_availability(false, false, false, false, true)
        );
        assert_eq!(
            Some(SubmissionStart::Rejected),
            submission_start_for_acp_availability(false, false, false, false, false)
        );
        assert_eq!(
            None,
            submission_start_for_acp_availability(true, false, false, false, false)
        );
    }

    #[test]
    fn acp_stop_action_distinguishes_prompt_control_and_failed_transition_states() {
        assert_eq!(
            AcpStopAction::CancelActivePrompt,
            acp_stop_action(true, true, false, false, false, None)
        );
        assert_eq!(
            AcpStopAction::ReturnToLocal,
            acp_stop_action(false, false, false, true, false, None)
        );
        assert_eq!(
            AcpStopAction::ReturnToLocal,
            acp_stop_action(false, false, false, false, true, None)
        );
        assert_eq!(
            AcpStopAction::ReturnToLocal,
            acp_stop_action(
                false,
                false,
                false,
                false,
                false,
                Some(AcpSessionTransitionPhase::Creating),
            )
        );
        assert_eq!(
            AcpStopAction::AbandonFailedTransition,
            acp_stop_action(
                false,
                true,
                false,
                false,
                false,
                Some(AcpSessionTransitionPhase::Failed),
            )
        );
        assert_eq!(
            AcpStopAction::ReturnToLocal,
            acp_stop_action(
                false,
                false,
                false,
                false,
                false,
                Some(AcpSessionTransitionPhase::Failed),
            )
        );
        assert_eq!(
            AcpStopAction::ClearQueueOnly,
            acp_stop_action(false, false, false, false, false, None)
        );
    }

    #[test]
    fn a_second_stop_settles_the_turn_instead_of_repeating_the_cancel() {
        // 取消已经发过、agent 就是不回终态:再点停止必须做点别的,否则按钮等于坏的。
        assert_eq!(
            AcpStopAction::ForceLocalStop,
            acp_stop_action(true, true, true, false, false, None)
        );
        // 连接已经没了,就没什么可结的账了,照旧走原有分支。
        assert_eq!(
            AcpStopAction::ClearQueueOnly,
            acp_stop_action(true, false, true, false, false, None)
        );
    }

    #[test]
    fn acp_turn_owner_marks_cancel_once_for_its_session() {
        let mut owner = AcpTurnOwner {
            event_session_id: SessionId::from_string("acp:cancel-once"),
            session_uid: "session-a".into(),
            turn_id: TurnId::from_string("turn-a"),
            backgrounded: false,
            cancel_requested: false,
        };

        assert!(!owner.mark_cancel_requested("session-b", true));
        assert!(!owner.mark_cancel_requested("session-a", false));
        assert!(owner.mark_cancel_requested("session-a", true));
        assert!(!owner.mark_cancel_requested("session-a", true));
    }

    fn acp_session_summary(id: &str, title: Option<&str>) -> AcpSessionSummary {
        AcpSessionSummary {
            id: id.to_string(),
            title: title.map(str::to_string),
            updated_at: None,
            cwd: std::path::PathBuf::from("/work"),
        }
    }

    /// 列表只在「ACP 后端 + agent 声明支持 session/list」时出现；两个条件任一不满足就不显示。
    #[gpui::test]
    fn acp_session_list_model_needs_backend_and_capability(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _| {
            view.acp_sessions = vec![acp_session_summary("s-1", Some("Plan review"))];

            // 后端不是 ACP:能力有了也不显示,否则本地会话侧栏会冒出别的 agent 的历史。
            view.acp_sessions_supported = true;
            assert!(view.acp_session_list_model().is_none());

            view.backend = Backend::Acp;
            let model = view
                .acp_session_list_model()
                .expect("visible on acp backend");
            assert_eq!(1, model.sessions.len());
            assert_eq!("Plan review", model.sessions[0].label());

            // 能力缺失就不显示(不变量 11)。
            view.acp_sessions_supported = false;
            assert!(view.acp_session_list_model().is_none());
        });
    }

    /// 切后端 / 换 agent 时抹状态必须顺带推进世代，否则在飞的旧响应还能把列表写回来。
    #[gpui::test]
    fn clearing_acp_sessions_drops_rows_and_in_flight_generation(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _| {
            view.backend = Backend::Acp;
            view.acp_sessions_supported = true;
            view.acp_sessions = vec![acp_session_summary("s-1", None)];
            view.acp_sessions_error = Some("boom".to_string());
            view.acp_sessions_loading = true;
            let generation = view.acp_sessions_generation;

            view.clear_acp_sessions();

            assert!(view.acp_sessions.is_empty());
            assert!(view.acp_sessions_error.is_none());
            assert!(!view.acp_sessions_loading);
            assert!(!view.acp_sessions_supported);
            assert_ne!(
                generation, view.acp_sessions_generation,
                "in-flight list responses must be invalidated"
            );
            assert!(view.acp_session_list_model().is_none());
        });
    }

    /// 拿不到打开方式(既不能 load 也不能 resume)时点击必须是哑的：
    /// 留下半截 transition 会把后续提交全卡在排队里。
    #[gpui::test]
    fn open_acp_session_is_inert_without_a_supported_open_kind(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            view.acp_sessions_supported = true;

            view.open_acp_session("s-1", cx);

            assert!(view.acp_session_transition.is_none());
            assert!(view.acp.is_none());
            assert!(view.acp_session_id_snapshot().is_none());
        });
    }

    #[gpui::test]
    fn cached_session_transcripts_evict_oldest_idle_entry(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _| {
            for index in 0..=MAX_CACHED_SESSION_TRANSCRIPTS {
                view.cache_session_transcript(format!("cached-{index}"), AgentTranscript::new());
            }

            assert_eq!(
                MAX_CACHED_SESSION_TRANSCRIPTS,
                view.session_transcripts.len()
            );
            assert!(!view.session_transcripts.contains_key("cached-0"));
            assert!(
                view.session_transcripts
                    .contains_key(&format!("cached-{MAX_CACHED_SESSION_TRANSCRIPTS}"))
            );
            assert!(
                !view.closed_sessions.contains("cached-0"),
                "cache eviction must not create a closed-session tombstone"
            );
        });
    }

    #[gpui::test]
    fn cached_session_transcripts_preserve_active_sessions_until_release(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _| {
            let running_uid = "cached-running".to_string();
            let pending_uid = "cached-pending".to_string();
            let owner_uid = "cached-owner".to_string();
            let transition_uid = "cached-transition".to_string();
            let idle_uid = "cached-idle".to_string();

            for uid in [
                &running_uid,
                &pending_uid,
                &owner_uid,
                &transition_uid,
                &idle_uid,
            ] {
                view.cache_session_transcript(uid.clone(), AgentTranscript::new());
            }
            view.running_sessions.insert(running_uid.clone());
            view.pending_submissions
                .enqueue(&pending_uid, pending_submission("queued"));
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: SessionId::from_string("acp:cached-owner"),
                session_uid: owner_uid.clone(),
                turn_id: TurnId::from_string("turn-cached-owner"),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.acp_session_transition = Some(AcpSessionTransition {
                operation: AcpOperationToken(1),
                agent_id: "cached-agent".into(),
                session_uid: transition_uid.clone(),
                phase: AcpSessionTransitionPhase::Creating,
            });

            view.trim_session_transcripts_to(1);

            assert_eq!(4, view.session_transcripts.len());
            assert!(view.session_transcripts.contains_key(&running_uid));
            assert!(view.session_transcripts.contains_key(&pending_uid));
            assert!(view.session_transcripts.contains_key(&owner_uid));
            assert!(view.session_transcripts.contains_key(&transition_uid));
            assert!(!view.session_transcripts.contains_key(&idle_uid));

            view.running_sessions.remove(&running_uid);
            view.pending_submissions.remove_session(&pending_uid);
            view.acp_turn_owners.clear();
            view.acp_session_transition = None;
            view.trim_session_transcripts_to(1);

            assert_eq!(1, view.session_transcripts.len());
            assert!(view.session_transcripts.contains_key(&transition_uid));
        });
    }

    #[gpui::test]
    fn cached_session_transcript_access_refreshes_recency(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _| {
            view.cache_session_transcript("cached-a".into(), AgentTranscript::new());
            view.cache_session_transcript("cached-b".into(), AgentTranscript::new());
            view.touch_session_transcript("cached-a");
            view.cache_session_transcript("cached-c".into(), AgentTranscript::new());
            view.trim_session_transcripts_to(2);

            assert!(view.session_transcripts.contains_key("cached-a"));
            assert!(!view.session_transcripts.contains_key("cached-b"));
            assert!(view.session_transcripts.contains_key("cached-c"));
        });
    }

    #[gpui::test]
    fn removing_a_queued_item_drops_only_that_entry(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let session_uid = view.current_session.clone();
            let queue = |view: &AgentChatView| {
                view.pending_submissions
                    .items(&session_uid)
                    .into_iter()
                    .map(|item| item.text.clone())
                    .collect::<Vec<_>>()
            };
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("第一条"));
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("第二条"));
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("第三条"));
            let input = view.input.clone();

            view.on_input_event(
                &input,
                &AgentInputEvent::RemoveQueued { index: 1 },
                window,
                cx,
            );

            assert_eq!(vec!["第一条", "第三条"], queue(view));

            // 越界删除必须是 no-op，不能把队首误删。
            view.on_input_event(
                &input,
                &AgentInputEvent::RemoveQueued { index: 9 },
                window,
                cx,
            );
            assert_eq!(vec!["第一条", "第三条"], queue(view));
        });
    }

    #[gpui::test]
    fn editing_a_queued_item_pulls_it_back_into_the_composer_and_dequeues_it(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let session_uid = view.current_session.clone();
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("要改的这条"));
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("留在队列里"));
            view.sync_pending_preview(cx);
            let input = view.input.clone();

            view.on_input_event(
                &input,
                &AgentInputEvent::EditQueued { index: 0 },
                window,
                cx,
            );

            // 队列里只剩没被编辑的那条，且编辑项已从队列移除而不是复制一份。
            let remaining: Vec<String> = view
                .pending_submissions
                .items(&session_uid)
                .into_iter()
                .map(|item| item.text.clone())
                .collect();
            assert_eq!(vec!["留在队列里"], remaining);

            // 文本回到输入框，用户可以改完再发。
            let restored = view.input.read(cx).composer_text(cx);
            assert_eq!("要改的这条", restored);
        });
    }

    #[gpui::test]
    fn acp_connecting_keeps_pending_submission_for_retry(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.acp = None;
            view.acp_connecting = true;
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("queued while connecting"));
            let message_count = view.transcript.messages.len();

            view.start_next_pending(&session_uid, cx);

            assert_eq!(1, view.pending_submissions.len(&session_uid));
            assert_eq!(
                "queued while connecting",
                view.pending_submissions.front(&session_uid).unwrap().text
            );
            assert_eq!(message_count, view.transcript.messages.len());
        });
    }

    #[gpui::test]
    fn disconnected_acp_rejects_pending_submission(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.acp = None;
            view.acp_connecting = false;
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("cannot start"));
            let message_count = view.transcript.messages.len();

            view.start_next_pending(&session_uid, cx);

            assert_eq!(0, view.pending_submissions.len(&session_uid));
            assert_eq!(message_count + 1, view.transcript.messages.len());
        });
    }

    impl FixedSidebarHost {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_height(px(640.0), window, cx)
        }

        fn short(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_height(px(200.0), window, cx)
        }

        fn with_height(height: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Self {
            let config =
                AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
                    .sidebar_mode(true);
            let view = cx.new(|cx| AgentChatView::new(config, window, cx));
            Self { view, height }
        }
    }

    impl Render for FixedSidebarHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .debug_selector(|| "fixed-sidebar-host".to_string())
                .w(px(420.0))
                .h(self.height)
                .overflow_hidden()
                .child(
                    div()
                        .debug_selector(|| "fixed-sidebar-header".to_string())
                        .h(px(34.0))
                        .w_full()
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .debug_selector(|| "fixed-sidebar-content-slot".to_string())
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(self.view.clone()),
                )
        }
    }

    #[async_trait]
    impl Tool for WriteTool {
        fn name(&self) -> ToolName {
            ToolName::new("write_data")
        }

        fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
            ToolSpec::new(
                "write_data",
                "写入测试数据。",
                json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                }),
            )
            .with_risk(RiskLevel::Low)
        }

        async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
            Ok(ToolObservation::success(
                invocation.call_id,
                invocation.tool_name,
                "write executed",
                ObservationData::Text("executed".into()),
            ))
        }
    }

    #[test]
    fn target_maps_label_kind_icon() {
        let r = ResourceRef::new("c1", ResourceKind::Redis, "prod-redis");
        let t = target_from_resource(&r);
        assert_eq!(t.label.as_ref(), "prod-redis");
        assert_eq!(t.kind.as_ref(), "redis");
        assert_eq!(t.icon.as_ref(), "RD");
    }

    #[test]
    fn auto_scroll_state_consumes_pending_scroll_in_render() {
        let mut state = AutoScrollState::default();

        assert!(!state.take_pending_for_render());
        state.request();
        assert!(state.take_pending_for_render());
        assert!(state.take_pending_for_render());
        assert!(!state.take_pending_for_render());
    }

    #[test]
    fn auto_scroll_state_terminal_request_spans_multiple_renders() {
        let mut state = AutoScrollState::default();

        state.request_settle();
        for _ in 0..5 {
            assert!(state.take_pending_for_render());
        }
        assert!(!state.take_pending_for_render());
    }

    #[gpui::test]
    fn resource_context_change_requests_scroll_to_latest_message(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let resource = ResourceRef::new("db-b", ResourceKind::Mysql, "secondary-db");
            let resources = ResourceContext::new().with_resource(resource.clone());

            assert_eq!(0, view.auto_scroll.pending_bottom_scroll_frames);
            view.set_resource_context_with_catalog(resources, Vec::new(), vec![resource], cx);
            assert_eq!(2, view.auto_scroll.pending_bottom_scroll_frames);
        });
    }

    #[gpui::test]
    fn sidebar_resource_context_change_scrolls_until_layout_settles(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let resource = ResourceRef::new("db-b", ResourceKind::Mysql, "secondary-db");
            let resources = ResourceContext::new().with_resource(resource.clone());

            assert_eq!(0, view.auto_scroll.pending_bottom_scroll_frames);
            view.set_resource_context_with_catalog(resources, Vec::new(), vec![resource], cx);
            assert_eq!(5, view.auto_scroll.pending_bottom_scroll_frames);
        });
    }

    #[gpui::test]
    fn sidebar_show_scrolls_until_layout_settles(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            assert_eq!(0, view.auto_scroll.pending_bottom_scroll_frames);
            view.on_sidebar_shown(cx);
            assert_eq!(5, view.auto_scroll.pending_bottom_scroll_frames);
        });
    }

    #[test]
    fn build_context_without_target_is_empty() {
        let ctx = build_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
        );
        assert!(ctx.target.is_none());
        assert!(ctx.scopes.is_empty());
        assert!(ctx.capabilities.is_empty());
        assert_eq!(
            ctx.execution_mode_label.as_ref(),
            t!("AgentUi.auto").as_ref()
        );
    }

    #[test]
    fn build_context_with_target_fills_scopes_and_caps() {
        let resources = ResourceContext::new().with_resource(
            ResourceRef::new("c1", ResourceKind::Mysql, "prod-mysql")
                .with_scope(agent_runtime::ResourceScope::new(
                    "database", "Database", "ai_app",
                ))
                .with_scope(agent_runtime::ResourceScope::new(
                    "schema", "Schema", "public",
                )),
        );
        let ctx = build_context(
            &resources,
            ExecutionSelection::tool(ToolExecutionMode::ReadOnly),
            Some(&ComposerModelOption::new(
                "openai:gpt-4.1",
                "openai",
                "OpenAI",
                "gpt-4.1",
            )),
        );
        assert_eq!(ctx.target.unwrap().label.as_ref(), "prod-mysql");
        assert_eq!(ctx.scopes.len(), 2);
        assert_eq!(ctx.scopes[0].value.as_ref(), "ai_app");
        assert_eq!(ctx.scopes[1].value.as_ref(), "public");
        assert_eq!(
            ctx.execution_mode_label.as_ref(),
            t!("AgentUi.readonly").as_ref()
        );
    }

    #[test]
    fn build_context_marks_current_resource_as_default_target() {
        let resources = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

        let context = build_context(
            &resources,
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
        );

        assert_eq!(context.resource_pool.total_resources, 2);
        assert_eq!(
            context
                .resource_pool
                .default_target_id
                .as_ref()
                .map(|id| id.as_ref()),
            Some("ssh-a")
        );
        assert_eq!(context.resource_pool.default_label.as_ref(), "prod-a");
    }

    #[test]
    fn build_context_counts_resource_types_for_filters() {
        let resources = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("db-a", ResourceKind::Postgres, "prod-db"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));

        let context = build_context(
            &resources,
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
        );

        let filters = context
            .resource_type_filters
            .iter()
            .map(|filter| (filter.id.as_ref(), filter.count))
            .collect::<Vec<_>>();

        assert_eq!(
            vec![("all", 3), ("postgres", 1), ("redis", 1), ("ssh", 1)],
            filters
        );
    }

    #[test]
    fn agent_config_defaults_available_resources_to_pool_resources() {
        let resources = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));

        let config = AgentChatViewConfig::new(test_runtime("m"), resources.clone(), Vec::new());

        assert_eq!(config.available_resources, resources.resources);
    }

    #[test]
    fn agent_config_can_start_with_empty_scope_and_non_empty_catalog() {
        let catalog = agent_runtime::ResourceCatalog::new(vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let scope = agent_runtime::AgentResourceScope::empty();

        let config = AgentChatViewConfig::new_with_scope(
            test_runtime("m"),
            scope,
            catalog.clone(),
            Vec::new(),
        );

        assert!(config.resources.is_empty());
        assert_eq!(catalog.resources, config.available_resources);
    }

    #[test]
    fn agent_config_accepts_available_resource_catalog() {
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        let config = AgentChatViewConfig::new(test_runtime("m"), pool, Vec::new())
            .with_available_resources(catalog.clone());

        assert_eq!(config.available_resources, catalog);
    }

    #[gpui::test]
    fn gpui_refreshing_resource_catalog_preserves_current_scope(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let initial_catalog = vec![ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a")];
        let config = AgentChatViewConfig::new(test_runtime("m"), pool, Vec::new())
            .with_available_resources(initial_catalog);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.set_resource_catalog(
                vec![
                    MentionItem::new("ssh-a", "prod-a", "ssh", "ssh"),
                    MentionItem::new("db-a", "prod-db", "mysql", "mysql"),
                ],
                vec![
                    ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a-renamed"),
                    ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
                ],
                cx,
            );
        });

        let (pool_labels, default_id, catalog_labels) = view.read_with(cx, |view, _| {
            (
                view.resources
                    .resources
                    .iter()
                    .map(|resource| resource.label.as_str().to_string())
                    .collect::<Vec<_>>(),
                view.resources
                    .current
                    .as_ref()
                    .map(|id| id.as_str().to_string()),
                view.available_resources
                    .iter()
                    .map(|resource| resource.label.as_str().to_string())
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(vec!["prod-a-renamed"], pool_labels);
        assert_eq!(Some("ssh-a".to_string()), default_id);
        assert_eq!(vec!["prod-a-renamed", "prod-db"], catalog_labels);
    }

    #[gpui::test]
    fn local_stop_ack_immediately_clears_running_state(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.set_running(true, cx);
            view.stop(cx);
        });

        assert!(!view.read_with(cx, |view, _| view.is_running));
    }

    #[gpui::test]
    fn running_submission_is_queued_without_adding_transcript_message(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::text("first answer"),
            ModelResponse::text("second answer"),
        ]));
        let runtime = test_runtime_with_model(model);
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            for text in ["first prompt", "second prompt"] {
                view.on_input_event(
                    &input,
                    &AgentInputEvent::Submit {
                        text: text.into(),
                        mentions: Vec::new(),
                        images: Vec::new(),
                    },
                    window,
                    cx,
                );
            }
        });

        let (queued, user_messages, running) = view.read_with(cx, |view, _| {
            (
                view.pending_submissions
                    .items(&view.current_session)
                    .into_iter()
                    .map(|item| item.text.clone())
                    .collect::<Vec<_>>(),
                view.transcript
                    .messages
                    .iter()
                    .filter(|message| message.role == crate::ChatRole::User)
                    .map(|message| message.content.clone())
                    .collect::<Vec<_>>(),
                view.is_running,
            )
        });

        assert_eq!(vec!["second prompt"], queued);
        assert_eq!(vec!["first prompt"], user_messages);
        assert!(running);
    }

    #[gpui::test]
    fn completed_turn_starts_pending_submissions_in_fifo_order(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::text("answer one"),
            ModelResponse::text("answer two"),
            ModelResponse::text("answer three"),
        ]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            for text in ["prompt one", "prompt two", "prompt three"] {
                view.on_input_event(
                    &input,
                    &AgentInputEvent::Submit {
                        text: text.into(),
                        mentions: Vec::new(),
                        images: Vec::new(),
                    },
                    window,
                    cx,
                );
            }
        });

        run_gpui_until(cx, || model.request_count() >= 3);
        cx.run_until_parked();

        let requests = model.received_requests();
        assert_eq!(3, requests.len());
        for (request, expected) in requests
            .iter()
            .zip(["prompt one", "prompt two", "prompt three"])
        {
            assert_eq!(
                expected,
                request
                    .messages
                    .last()
                    .expect("request user message")
                    .content_as_text()
            );
        }
        let (queued, running) = view.read_with(cx, |view, _| {
            (
                view.pending_submissions.len(&view.current_session),
                view.is_running,
            )
        });
        assert_eq!(0, queued);
        assert!(!running);
    }

    #[gpui::test]
    fn failed_turn_starts_next_pending_submission(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new(std::iter::empty::<ModelResponse>()));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            for text in ["failing prompt", "queued after failure"] {
                view.on_input_event(
                    &input,
                    &AgentInputEvent::Submit {
                        text: text.into(),
                        mentions: Vec::new(),
                        images: Vec::new(),
                    },
                    window,
                    cx,
                );
            }
        });

        run_gpui_until(cx, || model.request_count() >= 2);
        cx.run_until_parked();

        assert_eq!(
            0,
            view.read_with(cx, |view, _| view
                .pending_submissions
                .len(&view.current_session))
        );
        assert!(!view.read_with(cx, |view, _| view.is_running));
    }

    #[gpui::test]
    fn turn_boundaries_emit_host_events_for_workspace_checkpoints(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_for_subscribe = seen.clone();
        cx.update(|_window: &mut gpui::Window, cx: &mut gpui::App| {
            cx.subscribe(
                &view,
                move |_, event: &AgentChatViewEvent, _cx| match event {
                    AgentChatViewEvent::TurnStarted { .. } => {
                        seen_for_subscribe.lock().unwrap().push("started");
                    }
                    AgentChatViewEvent::TurnFinished { success, .. } => {
                        seen_for_subscribe.lock().unwrap().push(if *success {
                            "finished:ok"
                        } else {
                            "finished:fail"
                        });
                    }
                    _ => {}
                },
            )
            .detach();
        });

        view.update(cx, |view, cx| {
            let session_id = view.session_id.clone();
            view.apply_runtime_event(
                RuntimeEvent::TurnStarted {
                    session_id: session_id.clone(),
                    turn_id: agent_runtime::TurnId::from_string("turn-checkpoint"),
                },
                cx,
            );
            view.apply_runtime_event(
                RuntimeEvent::TurnCompleted {
                    session_id,
                    turn_id: agent_runtime::TurnId::from_string("turn-checkpoint"),
                    answer: None,
                },
                cx,
            );
        });

        cx.run_until_parked();
        let events = seen.lock().unwrap().clone();
        assert_eq!(
            vec!["started", "finished:ok"],
            events,
            "宿主依赖这些事件捕获 worktree checkpoint"
        );
    }

    #[gpui::test]
    fn restoring_a_turn_emits_a_host_request_for_that_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_for_subscribe = seen.clone();
        cx.update(|_window: &mut gpui::Window, cx: &mut gpui::App| {
            cx.subscribe(&view, move |_, event: &AgentChatViewEvent, _cx| {
                if let AgentChatViewEvent::RestoreTurn {
                    session_id,
                    turn_id,
                } = event
                {
                    seen_for_subscribe
                        .lock()
                        .unwrap()
                        .push((session_id.clone(), turn_id.clone()));
                }
            })
            .detach();
        });

        let session_id = view.read_with(cx, |view, _| view.current_session.clone());
        view.update(cx, |view, cx| {
            view.apply_message_list_action(
                MessageListAction::RestoreTurn {
                    turn_id: "turn-2".into(),
                },
                cx,
            );
        });
        cx.run_until_parked();

        assert_eq!(
            vec![(session_id, "turn-2".to_string())],
            seen.lock().unwrap().clone(),
            "宿主靠这个事件知道该回滚哪个会话的哪一轮"
        );
    }

    #[gpui::test]
    fn opening_a_changed_file_emits_a_host_request(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<(String, Option<String>)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_for_subscribe = seen.clone();
        cx.update(|_window: &mut gpui::Window, cx: &mut gpui::App| {
            cx.subscribe(&view, move |_, event: &AgentChatViewEvent, _cx| {
                if let AgentChatViewEvent::OpenFileInReview { path, turn_id, .. } = event {
                    seen_for_subscribe
                        .lock()
                        .unwrap()
                        .push((path.clone(), turn_id.clone()));
                }
            })
            .detach();
        });

        view.update(cx, |view, cx| {
            view.apply_message_list_action(
                MessageListAction::OpenFileInReview {
                    path: "src/lib.rs".into(),
                    turn_id: Some("turn-3".into()),
                },
                cx,
            );
        });
        cx.run_until_parked();

        assert_eq!(
            vec![("src/lib.rs".to_string(), Some("turn-3".to_string()))],
            seen.lock().unwrap().clone(),
            "视图只发路径与轮次，落到哪个面板、裁哪一份快照由宿主决定"
        );
    }

    #[gpui::test]
    fn a_message_resolves_to_the_turn_it_belongs_to(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_id = view.session_id.clone();
            view.apply_runtime_event(
                RuntimeEvent::AssistantMessage {
                    session_id,
                    turn_id: agent_runtime::TurnId::from_string("turn-card"),
                    text: "改完了".into(),
                },
                cx,
            );
        });
        cx.run_until_parked();

        let message_id = view.read_with(cx, |view, _| {
            view.transcript
                .messages
                .last()
                .map(|message| message.id.clone())
                .expect("一条助手消息")
        });

        assert_eq!(
            Some("turn-card".to_string()),
            view.read_with(cx, |view, _| view.turn_id_for_message(&message_id)),
            "工具卡片拿自己的消息 id 反查轮次，反查结果决定审阅面板裁哪一轮的快照"
        );
        assert_eq!(
            None,
            view.read_with(cx, |view, _| view.turn_id_for_message("no-such-message")),
            "查不到的消息不能猜一个轮次出来 —— 猜错就是拿别人的 diff 冒充"
        );
    }

    #[gpui::test]
    fn restorable_turns_are_bucketed_by_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        let current = view.read_with(cx, |view, _| view.current_session.clone());
        view.update(cx, |view, cx| {
            view.set_restorable_turns(current.clone(), HashSet::from(["turn-a".to_string()]), cx);
            view.set_restorable_turns(
                "another-session".to_string(),
                HashSet::from(["turn-b".to_string()]),
                cx,
            );
        });
        cx.run_until_parked();

        view.read_with(cx, |view, _| {
            assert_eq!(
                Some(&HashSet::from(["turn-a".to_string()])),
                view.restorable_turns.get(&current),
                "切回本会话时只有本会话的轮次算可回滚"
            );
            assert_eq!(
                Some(&HashSet::from(["turn-b".to_string()])),
                view.restorable_turns.get("another-session")
            );
            assert!(
                !view
                    .restorable_turns
                    .get(&current)
                    .is_some_and(|turns| turns.contains("turn-b")),
                "另一会话的轮次绝不能漏进本会话——那会渲染出点了没反应的入口"
            );
        });

        // 宿主发空列表 = 该会话已无可回滚轮次，桶要整个消失。
        view.update(cx, |view, cx| {
            view.set_restorable_turns(current.clone(), HashSet::new(), cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.restorable_turns.contains_key(&current));
        });
    }

    #[gpui::test]
    fn need_user_input_and_cancelled_turn_do_not_advance_pending_queue(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_id = view.session_id.clone();
            view.pending_submissions
                .enqueue(&view.current_session, pending_submission("queued"));
            view.set_running(true, cx);
            view.apply_runtime_event(
                RuntimeEvent::NeedUserInput {
                    session_id: session_id.clone(),
                    turn_id: agent_runtime::TurnId::from_string("turn-needs-input"),
                    question: "approve?".into(),
                    pending_tool_call_id: None,
                    tool_name: None,
                    arguments: None,
                    pending_tool_calls: Vec::new(),
                },
                cx,
            );
            assert_eq!(
                1,
                view.pending_submissions.len(&view.current_session),
                "NeedUserInput must keep the next-turn queue paused"
            );

            view.set_running(true, cx);
            view.apply_runtime_event(
                RuntimeEvent::TurnCancelled {
                    session_id,
                    turn_id: agent_runtime::TurnId::from_string("turn-cancelled"),
                },
                cx,
            );
        });

        let (queued, running) = view.read_with(cx, |view, _| {
            (
                view.pending_submissions.len(&view.current_session),
                view.is_running,
            )
        });
        assert_eq!(1, queued);
        assert!(!running);
    }

    #[gpui::test]
    fn local_tool_approval_keeps_running_and_queues_new_submit(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_id = view.session_id.clone();
            let session_uid = view.current_session.clone();
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("already queued"));
            view.set_running(true, cx);
            view.apply_runtime_event(
                RuntimeEvent::NeedUserInput {
                    session_id,
                    turn_id: TurnId::from_string("turn-tool-approval"),
                    question: "approve tool?".into(),
                    pending_tool_call_id: Some(ToolCallId::from_string("call-tool-approval")),
                    tool_name: Some(ToolName::new("write")),
                    arguments: Some(json!({"path": "/tmp/a"})),
                    pending_tool_calls: Vec::new(),
                },
                cx,
            );

            assert!(
                view.is_running,
                "manual tool approval still owns the current local turn"
            );
            view.submit("queued while approving".into(), Vec::new(), Vec::new(), cx);
        });

        view.read_with(cx, |view, _| {
            assert!(view.is_running);
            assert_eq!(2, view.pending_submissions.len(&view.current_session));
            assert_eq!(
                vec!["already queued", "queued while approving"],
                view.pending_submissions
                    .items(&view.current_session)
                    .into_iter()
                    .map(|submission| submission.text.as_str())
                    .collect::<Vec<_>>()
            );
        });
    }

    #[gpui::test]
    fn stale_local_failure_callback_does_not_touch_new_generation(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            let stale_generation = view.next_local_operation_generation(&session_uid);
            let current_generation = view.next_local_operation_generation(&session_uid);
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("still queued"));
            view.set_session_running(&session_uid, true, cx);
            let message_count = view.transcript.messages.len();

            view.finish_submission_without_event(
                &session_uid,
                stale_generation,
                "late failure from an older turn".into(),
                cx,
            );

            assert_eq!(
                Some(current_generation),
                view.current_local_operation_generation(&session_uid)
            );
            assert!(view.running_sessions.contains(&session_uid));
            assert_eq!(message_count, view.transcript.messages.len());
            assert_eq!(1, view.pending_submissions.len(&session_uid));
        });
    }

    #[gpui::test]
    fn acp_operation_generation_separates_same_agent_callbacks(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _cx| {
            let agent_id = SharedString::from("same-agent");
            let origin_session_uid = view.current_session.clone();
            view.acp_connecting_id = Some(agent_id.clone());
            view.acp_connect_origin_session = Some(origin_session_uid.clone());
            let stale_operation = view.next_acp_operation();
            let current_operation = view.next_acp_operation();

            assert!(!view.is_current_acp_connection_operation(
                stale_operation,
                &agent_id,
                &origin_session_uid,
            ));
            assert!(view.is_current_acp_connection_operation(
                current_operation,
                &agent_id,
                &origin_session_uid,
            ));
            assert!(!view.is_current_acp_connection_operation(
                current_operation,
                &agent_id,
                "different-session",
            ));
        });
    }

    #[gpui::test]
    fn acp_reconnect_keeps_its_origin_after_switching_sessions(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let origin_uid = view.current_session.clone();
            let target = view.runtime.create_session(view.resources.clone());
            let target_uid = target.id().to_string();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.pending_submissions
                .enqueue(&origin_uid, pending_submission("origin waits"));
            view.pending_submissions
                .enqueue(&target_uid, pending_submission("current waits"));

            let (operation, _permission_provider) = view
                .prepare_current_pending_reconnect(cx)
                .expect("the origin session should prepare its reconnect");
            view.switch_session(&target_uid, cx);

            assert_eq!(origin_uid, operation.session_uid);
            assert_eq!(
                Some(origin_uid.as_str()),
                view.acp_connect_origin_session.as_deref()
            );
            assert_eq!(target_uid, view.current_session);
            assert!(view.is_current_acp_connection_operation(
                operation.token,
                &agent_id,
                &origin_uid,
            ));
            assert_eq!(
                vec![origin_uid.clone(), target_uid.clone()],
                view.acp_pending_schedule_candidates(&origin_uid)
            );
            assert_eq!(1, view.pending_submissions.len(&origin_uid));
            assert_eq!(1, view.pending_submissions.len(&target_uid));
        });
    }

    #[gpui::test]
    fn acp_pending_schedule_skips_closed_origins_and_deduplicates_current(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _cx| {
            let current_uid = view.current_session.clone();
            assert_eq!(
                vec![current_uid.clone()],
                view.acp_pending_schedule_candidates(&current_uid)
            );

            let closed_origin = "closed-origin".to_string();
            view.closed_sessions.insert(closed_origin.clone());
            assert_eq!(
                vec![current_uid],
                view.acp_pending_schedule_candidates(&closed_origin)
            );
        });
    }

    #[gpui::test]
    fn acp_session_transition_keeps_fifo_paused_until_ready(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let agent_id = SharedString::from("agent");
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            let operation =
                view.begin_acp_session_transition(agent_id.clone(), session_uid.clone());
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("for the new session"));
            let message_count = view.transcript.messages.len();

            view.start_next_pending(&session_uid, cx);

            assert_eq!(
                Some(AcpSessionTransitionPhase::Creating),
                view.acp_session_transition_phase(&session_uid)
            );
            assert_eq!(1, view.pending_submissions.len(&session_uid));
            assert_eq!(message_count, view.transcript.messages.len());

            assert!(view.mark_acp_session_transition_failed(operation, &agent_id, &session_uid));
            view.start_next_pending(&session_uid, cx);

            assert_eq!(
                Some(AcpSessionTransitionPhase::Failed),
                view.acp_session_transition_phase(&session_uid)
            );
            assert_eq!(1, view.pending_submissions.len(&session_uid));
            assert_eq!(message_count, view.transcript.messages.len());
        });
    }

    #[gpui::test]
    fn failed_acp_transition_exposes_queue_and_stop_without_running_turn(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let agent_id = SharedString::from("agent");
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            let operation =
                view.begin_acp_session_transition(agent_id.clone(), session_uid.clone());
            assert!(view.mark_acp_session_transition_failed(operation, &agent_id, &session_uid));
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("retry after recovery"));
            view.sync_pending_preview(cx);

            assert!(!view.is_running);
            assert!(!view.input.read(cx).is_running());
        });

        let cx: &mut VisualTestContext = cx;
        assert!(
            cx.debug_bounds("agent-input-queue-send").is_some(),
            "a blocked FIFO must keep the queue-send control visible"
        );
        assert!(
            cx.debug_bounds("agent-input-stop").is_some(),
            "a blocked FIFO must expose an explicit way to abandon it"
        );
        assert!(
            cx.debug_bounds("agent-input-send-control").is_none(),
            "a blocked FIFO must not fall back to the ordinary send control"
        );
    }

    #[gpui::test]
    fn selecting_acp_immediately_routes_submissions_away_from_local(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let prepared = view.prepare_acp_connect(agent_id.clone(), cx);

            assert!(prepared.is_some());
            assert_eq!(Backend::Acp, view.backend);
            assert_eq!(Some(&agent_id), view.current_acp_id.as_ref());
            assert!(view.acp_connecting);
            assert_eq!(
                SubmissionStart::RetryLater,
                view.start_submission(
                    &view.current_session.clone(),
                    &pending_submission("must wait for ACP"),
                    cx,
                )
            );
        });
    }

    #[gpui::test]
    fn switching_acp_targets_immediately_replaces_the_current_route(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let first_id = SharedString::from("first");
        let second_id = SharedString::from("second");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![
                    AcpAgentEntry::ready(AcpAgentConfig::new(
                        first_id.clone(),
                        "First",
                        "definitely-missing-first-acp-binary",
                    )),
                    AcpAgentEntry::ready(AcpAgentConfig::new(
                        second_id.clone(),
                        "Second",
                        "definitely-missing-second-acp-binary",
                    )),
                ]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            view.current_acp_id = Some(first_id);

            let prepared = view.prepare_acp_connect(second_id.clone(), cx);

            assert!(prepared.is_some());
            assert_eq!(Backend::Acp, view.backend);
            assert_eq!(Some(&second_id), view.current_acp_id.as_ref());
            assert!(view.acp_connecting);
            assert_eq!(Some(&second_id), view.acp_connecting_id.as_ref());
            assert_eq!(
                SubmissionStart::RetryLater,
                view.start_submission(
                    &view.current_session.clone(),
                    &pending_submission("must wait for the new ACP target"),
                    cx,
                )
            );
        });
    }

    #[gpui::test]
    fn disconnected_acp_with_fifo_can_retry_the_same_target(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.acp = None;
            view.acp_connecting = false;
            view.acp_pending = None;
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("resume after reconnect"));

            assert!(view.can_select_backend(Some(&agent_id)));
            let prepared = view.prepare_acp_connect(agent_id.clone(), cx);

            assert!(prepared.is_some());
            assert!(view.acp_connecting);
            assert_eq!(Some(&agent_id), view.acp_connecting_id.as_ref());
            assert_eq!(1, view.pending_submissions.len(&session_uid));
        });
    }

    #[gpui::test]
    fn disconnected_acp_keeps_existing_fifo_when_more_input_is_queued(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("first queued prompt"));

            view.enqueue_submission(&session_uid, pending_submission("second queued prompt"), cx);
            let (operation, _permission_provider) = view
                .prepare_current_pending_reconnect(cx)
                .expect("the disconnected ACP target should prepare a reconnect");

            assert_eq!(2, view.pending_submissions.len(&session_uid));
            assert_eq!(
                ["first queued prompt", "second queued prompt"],
                view.pending_submissions
                    .items(&session_uid)
                    .into_iter()
                    .map(|submission| submission.text.as_str())
                    .collect::<Vec<_>>()
                    .as_slice()
            );
            assert_eq!(session_uid, operation.session_uid);
            assert!(view.acp_connecting);
            assert_eq!(Some(&agent_id), view.acp_connecting_id.as_ref());
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .all(|message| { message.content != t!("AgentUi.acp_not_connected").as_ref() })
            );
        });
    }

    #[gpui::test]
    fn disconnected_acp_submit_enqueues_and_starts_same_target_reconnect(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.acp = None;

            view.enqueue_submission(
                &session_uid,
                pending_submission("resume after disconnect"),
                cx,
            );
            let (operation, _permission_provider) = view
                .prepare_current_pending_reconnect(cx)
                .expect("the queued submission should prepare the same ACP reconnect");

            assert_eq!(1, view.pending_submissions.len(&session_uid));
            assert_eq!(
                "resume after disconnect",
                view.pending_submissions.front(&session_uid).unwrap().text
            );
            assert_eq!(session_uid, operation.session_uid);
            assert!(view.acp_connecting);
            assert_eq!(Some(&agent_id), view.acp_connecting_id.as_ref());
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .all(|message| { message.content != t!("AgentUi.acp_not_connected").as_ref() })
            );
        });
    }

    #[gpui::test]
    fn switching_to_disconnected_acp_session_reconnects_without_consuming_fifo(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let target = view.runtime.create_session(view.resources.clone());
            let target_uid = target.id().to_string();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.acp = None;
            view.pending_submissions
                .enqueue(&target_uid, pending_submission("queued in target"));
            // Keep the switch synchronous: an already-started connect blocks the switch hook
            // from spawning another real ACP process in this deterministic GPUI test.
            view.acp_connecting = true;
            view.acp_connecting_id = Some(agent_id.clone());

            view.switch_session(&target_uid, cx);

            assert_eq!(target_uid, view.current_session);
            assert_eq!(1, view.pending_submissions.len(&target_uid));
            assert_eq!(
                "queued in target",
                view.pending_submissions.front(&target_uid).unwrap().text
            );
            view.acp_connecting = false;
            view.acp_connecting_id = None;
            let (operation, _permission_provider) = view
                .prepare_current_pending_reconnect(cx)
                .expect("the switched-to session should prepare its reconnect");
            assert_eq!(target_uid, operation.session_uid);
            assert!(view.acp_connecting);
            assert_eq!(Some(&agent_id), view.acp_connecting_id.as_ref());
        });
    }

    #[gpui::test]
    fn acp_terminal_falls_through_from_empty_owner_fifo_to_current_session(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let owner_uid = view.current_session.clone();
            view.start_fresh_session(cx);
            let current_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.acp = None;
            view.pending_submissions
                .enqueue(&current_uid, pending_submission("current waits"));

            assert_eq!(
                vec![owner_uid.clone(), current_uid.clone()],
                view.acp_pending_schedule_candidates(&owner_uid)
            );
            assert_eq!(
                PendingAdvance::Idle,
                view.start_next_pending(&owner_uid, cx)
            );
            let (operation, _permission_provider) = view
                .prepare_current_pending_reconnect(cx)
                .expect("the current session should reconnect after the empty owner queue");

            assert_eq!(1, view.pending_submissions.len(&current_uid));
            assert_eq!(current_uid, operation.session_uid);
            assert!(view.acp_connecting);
            assert_eq!(Some(&agent_id), view.acp_connecting_id.as_ref());
        });
    }

    #[gpui::test]
    fn stop_during_acp_connect_returns_to_local_and_invalidates_callback(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            let prepared = view.prepare_acp_connect(agent_id, cx);
            let operation = AcpOperationToken(view.acp_operation_generation);
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("cancel with connect"));

            assert!(prepared.is_some());
            view.stop(cx);

            assert_eq!(Backend::Local, view.backend);
            assert_eq!(None, view.current_acp_id);
            assert!(!view.acp_connecting);
            assert_eq!(None, view.acp_connecting_id);
            assert!(!view.is_current_acp_operation(operation));
            assert_eq!(0, view.pending_submissions.len(&session_uid));
            assert!(!view.input.read(cx).is_running());
        });
    }

    #[gpui::test]
    fn stop_during_background_acp_connect_clears_origin_only(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let origin_uid = view.current_session.clone();
            let target = view.runtime.create_session(view.resources.clone());
            let target_uid = target.id().to_string();
            view.pending_submissions
                .enqueue(&origin_uid, pending_submission("cancel origin"));
            view.pending_submissions
                .enqueue(&target_uid, pending_submission("preserve target"));
            let (operation, _permission_provider) = view
                .prepare_acp_connect(agent_id, cx)
                .expect("the origin session should start connecting");
            view.switch_session(&target_uid, cx);
            view.transcript.push_system("preserve target transcript");

            view.stop(cx);

            assert_eq!(Backend::Local, view.backend);
            assert_eq!(0, view.pending_submissions.len(&origin_uid));
            assert_eq!(1, view.pending_submissions.len(&target_uid));
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| { message.content == "preserve target transcript" })
            );
            assert!(!view.is_current_acp_operation(operation.token));
            assert_eq!(None, view.acp_connect_origin_session);
        });
    }

    #[gpui::test]
    fn stop_during_acp_session_creation_returns_to_local(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let agent_id = SharedString::from("agent");
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            let operation = view.begin_acp_session_transition(agent_id, session_uid.clone());
            view.input
                .update(cx, |input, cx| input.set_running(true, cx));
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("cancel with new session"));

            view.stop(cx);

            assert_eq!(Backend::Local, view.backend);
            assert_eq!(None, view.acp_session_transition_phase(&session_uid));
            assert!(!view.is_current_acp_operation(operation));
            assert_eq!(0, view.pending_submissions.len(&session_uid));
            assert!(!view.input.read(cx).is_running());
        });
    }

    #[gpui::test]
    fn cancel_acp_auth_clears_fifo_and_fully_restores_local(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(SharedString::from("agent"));
            view.acp_connecting = true;
            view.acp_connecting_id = view.current_acp_id.clone();
            view.pending_submissions
                .enqueue(&session_uid, pending_submission("cancel with auth"));

            view.cancel_acp_auth(cx);

            assert_eq!(Backend::Local, view.backend);
            assert_eq!(None, view.current_acp_id);
            assert!(!view.acp_connecting);
            assert_eq!(0, view.pending_submissions.len(&session_uid));
            assert!(!view.input.read(cx).is_running());
        });
    }

    #[gpui::test]
    fn cancel_background_acp_auth_preserves_current_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let origin_uid = view.current_session.clone();
            let target = view.runtime.create_session(view.resources.clone());
            let target_uid = target.id().to_string();
            let agent_id = SharedString::from("agent");
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            view.acp_connecting = true;
            view.acp_connecting_id = Some(agent_id);
            view.acp_connect_origin_session = Some(origin_uid.clone());
            view.pending_submissions
                .enqueue(&origin_uid, pending_submission("cancel origin auth"));
            view.pending_submissions
                .enqueue(&target_uid, pending_submission("preserve target"));
            view.switch_session(&target_uid, cx);
            view.transcript.push_system("preserve target transcript");

            view.cancel_acp_auth(cx);

            assert_eq!(Backend::Local, view.backend);
            assert_eq!(0, view.pending_submissions.len(&origin_uid));
            assert_eq!(1, view.pending_submissions.len(&target_uid));
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| { message.content == "preserve target transcript" })
            );
            assert_eq!(None, view.acp_connect_origin_session);
        });
    }

    #[gpui::test]
    fn switching_to_local_invalidates_acp_operation(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            let agent_id = SharedString::from("agent");
            view.current_acp_id = Some(agent_id.clone());
            let session_uid = view.current_session.clone();
            let stale_operation = view.begin_acp_session_transition(agent_id, session_uid.clone());

            view.select_local_backend(cx);

            assert_eq!(Backend::Local, view.backend);
            assert!(!view.is_current_acp_operation(stale_operation));
            assert_eq!(None, view.acp_session_transition_phase(&session_uid));
        });
    }

    /// 切到 ACP 后端前，必须把本地那段对话落进会话缓存。
    ///
    /// 本地转录归 navop 所有，ACP 会话历史归 agent（靠 load/resume 取回），
    /// 两者不能共用同一个 `clear`：否则「本地 → ACP → 切回本地」之后本地内容凭空消失。
    #[gpui::test]
    fn leaving_local_backend_stashes_the_local_transcript(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.transcript.push_system(LOCAL_TRANSCRIPT_MARK);

            view.begin_acp_connect(&agent, &session_uid, cx);

            assert!(
                view.session_transcripts.contains_key(&session_uid),
                "切到 ACP 前本地转录必须落进会话缓存"
            );
            assert!(
                !view
                    .transcript
                    .messages
                    .iter()
                    .any(|message| message.content == LOCAL_TRANSCRIPT_MARK),
                "ACP 会话内容未知，屏幕不该继续显示上一段本地对话"
            );
        });
    }

    /// 从 ACP 切回本地时，屏幕上要能把本地那段对话还回来。
    #[gpui::test]
    fn returning_to_local_restores_the_stashed_transcript(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            view.transcript.push_system(LOCAL_TRANSCRIPT_MARK);
            view.begin_acp_connect(&agent, &session_uid, cx);

            view.select_local_backend_for_session(&session_uid, cx);

            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content == LOCAL_TRANSCRIPT_MARK),
                "切回本地必须恢复该会话的本地转录"
            );
        });
    }

    /// 缓存被淘汰也不能让本地会话在切回时变成空白：能从 Runtime 快照重建。
    #[gpui::test]
    fn returning_to_local_rebuilds_from_runtime_when_cache_is_gone(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            let session_id = view.session_id.clone();
            view.runtime
                .session(&session_id)
                .expect("current session is live")
                .record_user_input("本地快照里的话");
            view.transcript.push_system(LOCAL_TRANSCRIPT_MARK);
            view.begin_acp_connect(&agent, &session_uid, cx);
            // 模拟缓存被 LRU 淘汰 / 从未落缓存。
            view.remove_cached_session_transcript(&session_uid);

            view.select_local_backend_for_session(&session_uid, cx);

            assert!(
                view.transcript.messages.iter().any(|message| {
                    matches!(&message.content, content if content.contains("本地快照里的话"))
                }),
                "缓存缺失时应从 Runtime 快照重建本地转录"
            );
        });
    }

    /// 本地这段对话不会进入 agent 的上下文，切过去时必须说在明处。
    #[gpui::test]
    fn switching_to_acp_tells_the_user_where_the_local_context_goes(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(vec![AcpAgentEntry::ready(agent.clone())], cx);
            let session_uid = view.current_session.clone();
            view.transcript.push_system(LOCAL_TRANSCRIPT_MARK);

            view.begin_acp_connect(&agent, &session_uid, cx);

            assert!(
                view.transcript.messages.iter().any(|message| {
                    message.content == t!("AgentUi.context_boundary_to_acp", name = "Agent")
                }),
                "切到 ACP 必须提示本地上下文不会带过去，实际内容：{:?}",
                view.transcript
                    .messages
                    .iter()
                    .map(|message| message.content.clone())
                    .collect::<Vec<_>>()
            );
        });
    }

    /// 本地会话本来就是空的，切到 ACP 不必提示「本地那段不会带过去」——没有可失的东西。
    ///
    /// 顺带钉住空转录**不进缓存**：缓存里留一个空壳会遮蔽 Runtime 快照重建，
    /// 让「切回本地」看起来像历史丢了。
    #[gpui::test]
    fn switching_to_acp_stays_quiet_when_the_local_session_is_empty(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(vec![AcpAgentEntry::ready(agent.clone())], cx);
            let session_uid = view.current_session.clone();

            view.begin_acp_connect(&agent, &session_uid, cx);

            let boundary_notice = t!("AgentUi.context_boundary_to_acp", name = "Agent").to_string();
            assert!(
                !view
                    .transcript
                    .messages
                    .iter()
                    .any(|message| message.content == boundary_notice),
                "本地没有内容时不该提示上下文边界"
            );
            assert!(
                !view.session_transcripts.contains_key(&session_uid),
                "空转录不该进会话缓存"
            );
        });
    }

    /// 切回本地时要说清 agent 那段对话去哪了：它在 agent 一侧，不进入本地模型的上下文。
    #[gpui::test]
    fn switching_back_to_local_explains_where_the_agent_conversation_went(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(vec![AcpAgentEntry::ready(agent.clone())], cx);
            let session_uid = view.current_session.clone();
            view.transcript.push_system(LOCAL_TRANSCRIPT_MARK);
            view.begin_acp_connect(&agent, &session_uid, cx);
            // 屏幕上此刻是 agent 的内容。
            view.transcript.push_system(ACP_TRANSCRIPT_MARK);

            view.select_local_backend_for_session(&session_uid, cx);

            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content == LOCAL_TRANSCRIPT_MARK),
                "切回本地要恢复本地那段对话"
            );
            assert!(
                view.transcript.messages.iter().any(|message| {
                    message.content == t!("AgentUi.context_boundary_to_local", name = "Agent")
                }),
                "切回本地必须说明 agent 那段对话留在哪一侧"
            );
        });
    }

    /// 缓存里留一个空壳不能让切回本地变空白：Runtime 里的历史还在，必须重建出来。
    ///
    /// 空壳是真实会出现的：连接失败 / 状态推送会给非当前会话建一块空白转录。
    #[gpui::test]
    fn an_empty_session_cache_does_not_shadow_the_runtime_history(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            let session_uid = view.current_session.clone();
            let session_id = view.session_id.clone();
            view.runtime
                .session(&session_id)
                .expect("current session is live")
                .record_user_input("本地快照里的话");
            view.begin_acp_connect(&agent, &session_uid, cx);
            // 缓存没了，只剩一个空壳。
            view.remove_cached_session_transcript(&session_uid);
            view.cache_session_transcript(session_uid.clone(), AgentTranscript::new());

            view.select_local_backend_for_session(&session_uid, cx);

            assert!(
                view.transcript.messages.iter().any(|message| {
                    matches!(&message.content, content if content.contains("本地快照里的话"))
                }),
                "空缓存不能遮蔽 Runtime 快照重建"
            );
        });
    }

    /// 切会话时，缓存里一块空壳转录同样不能遮蔽 Runtime 里的历史。
    #[gpui::test]
    fn switching_sessions_rebuilds_when_the_cached_transcript_is_empty(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.runtime
                .session(&first_id)
                .expect("current session is live")
                .record_user_input("第一段会话的话");

            // 换到另一个会话，并把第一个会话的缓存换成一块空壳。
            view.start_fresh_session(cx);
            assert_ne!(first, view.current_session);
            view.cache_session_transcript(first.clone(), AgentTranscript::new());

            view.switch_session(&first, cx);

            assert!(
                view.transcript.messages.iter().any(|message| {
                    matches!(&message.content, content if content.contains("第一段会话的话"))
                }),
                "空壳缓存不能遮蔽 Runtime 快照"
            );
        });
    }

    /// 会话草稿跟随会话：切走时捕获到 Runtime 会话，切回来时回到输入框。
    /// 每个会话各存各的，互不串台。
    #[gpui::test]
    fn switching_sessions_carries_the_composer_draft_per_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.input.update(cx, |input, cx| {
                input.set_composer_text("第一个会话的半句话", window, cx)
            });
            // 第一个会话有了历史（否则不会被持久化），草稿才有落盘意义。
            view.runtime
                .session(&first_id)
                .expect("first session is live")
                .record_user_input("第一段会话的话");

            // 新建会话：输入框应清空（新会话没有草稿）。
            view.new_session(cx);
            let second = view.current_session.clone();
            assert_ne!(first, second);
            view.apply_pending_input_draft(window, cx);
            assert_eq!(
                view.input.read(cx).composer_text(cx),
                "",
                "新会话必须从空输入框开始"
            );

            // 在第二个会话里留半句，再切回第一个。
            view.input.update(cx, |input, cx| {
                input.set_composer_text("第二个会话的半句话", window, cx)
            });
            view.switch_session(&first, cx);
            view.apply_pending_input_draft(window, cx);
            assert_eq!(
                view.input.read(cx).composer_text(cx),
                "第一个会话的半句话",
                "切回来必须恢复该会话自己的草稿"
            );

            // 两个会话的 Runtime 草稿互不覆盖。
            assert_eq!(
                view.runtime
                    .session(&SessionId::from_string(first.clone()))
                    .expect("first still live")
                    .draft()
                    .as_deref(),
                Some("第一个会话的半句话")
            );
            assert_eq!(
                view.runtime
                    .session(&SessionId::from_string(second.clone()))
                    .expect("second still live")
                    .draft()
                    .as_deref(),
                Some("第二个会话的半句话")
            );
        });
    }

    /// 测试用的图片附件：内容字节不重要（这些用例不渲染缩略图），重要的是
    /// 它是一份**跟着会话走的用户内容**。
    fn test_image_attachment(name: &str) -> crate::ImageAttachment {
        crate::ImageAttachment {
            id: format!("img-{name}"),
            name: format!("{name}.png"),
            image: std::sync::Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                vec![0x89, 0x50, 0x4E, 0x47],
            )),
        }
    }

    /// 附件也跟着会话走：切走时捕获、
    /// 切回来时恢复；目标会话没有附件草稿时，输入框里**不许残留**上一会话的
    /// 图片——否则它会跟着新会话一起发出去。
    #[gpui::test]
    fn switching_sessions_carries_the_composer_attachments_per_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.input.update(cx, |input, cx| {
                input.set_composer_text("第一个会话的半句话", window, cx);
                input.set_composer_attachments(vec![test_image_attachment("a")], cx);
            });
            // 第一个会话有了历史（否则不会被持久化），草稿才有落盘意义。
            view.runtime
                .session(&first_id)
                .expect("first session is live")
                .record_user_input("第一段会话的话");

            // 新会话：输入框要**连图片一起**清空（新会话没有附件草稿）。
            view.new_session(cx);
            let second = view.current_session.clone();
            assert_ne!(first, second);
            view.apply_pending_input_draft(window, cx);
            assert_eq!(view.input.read(cx).composer_text(cx), "");
            assert!(
                view.input.read(cx).composer_attachments().is_empty(),
                "切到没有附件草稿的会话，上一会话的图片不能留在输入框里"
            );

            // 切回第一个会话：文字与图片都要回来。
            view.switch_session(&first, cx);
            view.apply_pending_input_draft(window, cx);
            assert_eq!(
                view.input.read(cx).composer_text(cx),
                "第一个会话的半句话",
                "切回来必须恢复该会话自己的文字草稿"
            );
            let attachments = view.input.read(cx).composer_attachments();
            assert_eq!(1, attachments.len(), "切回来必须恢复该会话自己的附件");
            assert_eq!("img-a", attachments[0].id, "恢复的是同一张图");
        });
    }

    /// 输入框只挂了图片、没有文字时，这个会话也**不算白纸**：
    /// 用户已经开始准备内容了。
    #[gpui::test]
    fn a_session_with_only_composer_attachments_is_not_blank(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, cx| {
            assert!(
                view.current_session_is_blank(cx),
                "刚打开、什么都没挂的会话才是白纸"
            );
            view.input.update(cx, |input, cx| {
                input.set_composer_attachments(vec![test_image_attachment("x")], cx);
            });
            assert!(
                !view.current_session_is_blank(cx),
                "只挂了图片（没有文字）也不是白纸"
            );
        });
    }

    /// 发送后附件草稿作废：切走再切回，已发送的图片不能「复活」回输入框
    /// （文字已有同款保护，这条补齐附件侧）。
    #[gpui::test]
    fn submit_consumes_the_attachment_draft(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let first = view.current_session.clone();
            view.input.update(cx, |input, cx| {
                input.set_composer_text("带图的这句话", window, cx);
                input.set_composer_attachments(vec![test_image_attachment("s")], cx);
            });
            // 先把附件捕获进这个会话的草稿位（否则 map 里本来就没有它，
            // 下面的「作废」断言就成了无中生有的空转绿）。
            view.capture_session_draft(cx);
            assert!(
                view.session_draft_attachments.contains_key(&first),
                "前置：附件草稿已进入该会话的草稿位"
            );
            // 排队一个提交并标记运行中：submit 走入队路径，不触发模型调用。
            view.pending_submissions
                .enqueue(&first, pending_submission("already queued"));
            view.set_running(true, cx);

            view.submit(
                "带图的这句话".into(),
                Vec::new(),
                vec![test_image_attachment("s")],
                cx,
            );

            assert!(
                !view.session_draft_attachments.contains_key(&first),
                "发送后该会话的附件草稿必须作废"
            );
            // 排队路径没有丢内容：图片跟着提交进了队列。
            assert_eq!(
                1,
                view.pending_submissions
                    .items(&first)
                    .last()
                    .expect("submitted item is queued")
                    .images
                    .len()
            );
        });
    }

    /// 空白草稿只在**同一个工作区**内复用：工作区 B 里点「新建」，不能把用户
    /// 切到工作区 A 留下的白纸上（那张纸的快照归属、技能上下文都是 A 的）。
    #[gpui::test]
    fn a_blank_session_from_another_workspace_is_not_reused(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_workspace_root(std::path::PathBuf::from("/tmp/navop-test-ws-a"));
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.runtime
                .session(&first_id)
                .expect("first session is live")
                .record_user_input("第一段会话的话");

            // 在 ws-a 里留下一张白纸。
            view.new_session(cx);
            let blank = view.current_session.clone();
            view.switch_session(&first, cx);
            assert_eq!(Some(blank.clone()), view.draft_session, "前置：白纸被记下");

            // 壳层切到工作区 B（直接改字段：绕开 set_workspace_root 的设置落盘）。
            view.workspace_root = std::path::PathBuf::from("/tmp/navop-test-ws-b");

            // B 里点「新建」：不能切回 A 的白纸，要给 B 造一张新纸。
            view.new_session(cx);
            assert_ne!(
                blank, view.current_session,
                "别的工作区留下的白纸不能被复用"
            );
            assert_eq!(
                Some(blank.clone()),
                view.draft_session,
                "指针要留着：那张纸在它自己的工作区里仍然有效"
            );
            assert!(
                view.session_roots
                    .get(&view.current_session)
                    .is_some_and(|root| root == "/tmp/navop-test-ws-b"),
                "新纸必须归属当前工作区"
            );

            // 切回工作区 A：那张白纸又能被复用了。
            view.workspace_root = std::path::PathBuf::from("/tmp/navop-test-ws-a");
            view.new_session(cx);
            assert_eq!(
                blank, view.current_session,
                "回到原工作区后，自己的白纸照常复用"
            );
        });
    }

    /// 只有系统提示的转录不算「聊过」：否则「正在创建 ACP 会话」这类提示
    /// 会让空白会话看起来像有内容，空白复用与切后端提示都会跟着错。
    #[test]
    fn a_transcript_with_only_system_notes_has_no_conversation() {
        let mut transcript = AgentTranscript::new();
        assert!(!transcript.has_conversation(), "空转录当然没聊过");
        transcript.push_system("正在创建 ACP 会话");
        assert!(!transcript.has_conversation(), "只有系统提示不算聊过");
        transcript.push_user("你好", 0);
        assert!(transcript.has_conversation());
    }

    /// 已经站在一张白纸上时点「新建会话」是 no-op：否则每点一次就多一张
    /// 侧栏里的空条目。
    #[gpui::test]
    fn new_session_on_an_untouched_blank_session_does_not_pile_up_sessions(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, cx| {
            let first = view.current_session.clone();
            assert!(
                view.current_session_is_blank(cx),
                "刚打开的会话就是一张白纸"
            );
            view.new_session(cx);
            assert_eq!(
                first, view.current_session,
                "白纸上再点一次「新建」不该produce第二张纸"
            );
            assert_eq!(None, view.draft_session, "no-op 不该留下草稿指针");
        });
    }

    /// 只有系统提示的历史仍算白纸。这条真会出现在落盘数据里：一段只跑到
    /// 「正在创建 ACP 会话」就失败、或者只有系统告示的会话，往返回来就是
    /// `HistoryItem::System`。把它判成「聊过」会让白纸复用失效、侧栏开始堆空条目。
    #[gpui::test]
    fn a_session_whose_history_is_only_system_notes_is_still_reusable(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, _cx| {
            let uid = "sess_system_only";
            view.runtime
                .restore_session(agent_runtime::SessionSnapshot {
                    id: SessionId::from_string(uid),
                    resources: ResourceContext::new(),
                    history: vec![HistoryItem::System("正在创建 ACP 会话".into())],
                    plan: None,
                    system_instruction: None,
                    skills: agent_runtime::SkillContext::new(),
                    workspace_root: None,
                    draft: None,
                    context_tokens: None,
                    acp: None,
                });

            assert!(
                view.runtime_history_is_blank(uid),
                "只挂了一条系统提示 = 还是白纸"
            );
            assert!(
                view.reusable_blank_session(uid),
                "白纸应当可以被「新建会话」复用"
            );

            // 真聊过之后就不再是白纸。
            view.runtime
                .session(&SessionId::from_string(uid))
                .expect("restored session is live")
                .record_user_input("你好");
            assert!(!view.runtime_history_is_blank(uid));
            assert!(!view.reusable_blank_session(uid), "聊过的会话不能复用");
        });
    }

    /// 离开一张白纸后，再点「新建会话」要回到那张纸，而不是造第三张。
    #[gpui::test]
    fn new_session_reuses_the_blank_session_left_behind(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, window, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.runtime
                .session(&first_id)
                .expect("first session is live")
                .record_user_input("第一段会话的话");

            // 第一个会话聊过了：新建必须真的产生第二张纸。
            view.new_session(cx);
            let blank = view.current_session.clone();
            assert_ne!(first, blank);
            view.apply_pending_input_draft(window, cx);

            // 离开这张白纸去看第一个会话：它应当被记成可复用的草稿。
            view.switch_session(&first, cx);
            assert_eq!(
                Some(blank.clone()),
                view.draft_session,
                "离开的空白会话要被记下来"
            );

            // 再点「新建」：回到那张纸，不是造第三张。
            view.new_session(cx);
            assert_eq!(blank, view.current_session, "应当复用被留在身后的白纸");
            assert_eq!(None, view.draft_session, "草稿被消费后要清掉");
        });
    }

    /// 记下的白纸一旦真有了历史，就不能再被当白纸复用——否则「新建会话」
    /// 会把用户送进一段他以为已经清空的对话。
    #[gpui::test]
    fn a_recorded_blank_session_stops_being_reusable_once_it_has_history(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, cx| {
            let first = view.current_session.clone();
            let first_id = view.session_id.clone();
            view.runtime
                .session(&first_id)
                .expect("first session is live")
                .record_user_input("第一段会话的话");

            view.new_session(cx);
            let blank = view.current_session.clone();
            view.switch_session(&first, cx);
            assert_eq!(Some(blank.clone()), view.draft_session);

            // 模拟后台在这张「白纸」上落了内容（跑完的一轮 / 恢复的提交）。
            view.runtime
                .session(&SessionId::from_string(blank.clone()))
                .expect("blank session is live")
                .record_user_input("其实已经聊过了");

            view.new_session(cx);
            assert_ne!(
                blank, view.current_session,
                "已经有历史的会话不能再当白纸复用"
            );
            assert_eq!(None, view.draft_session, "失效的草稿指针要清掉");
        });
    }

    /// 后退/前进沿访问历史走：a → b → c，后退两次、前进两次，栈空后 no-op；
    /// 会话消失后导航栈里不残留它的痕迹。
    #[gpui::test]
    fn session_back_and_forward_navigation_walks_the_visit_history(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let a = view.current_session.clone();
            view.start_fresh_session(cx);
            let b = view.current_session.clone();
            view.start_fresh_session(cx);
            let c = view.current_session.clone();

            view.navigate_session_back(cx);
            assert_eq!(b, view.current_session, "第一次后退回到上一个会话");
            view.navigate_session_back(cx);
            assert_eq!(a, view.current_session, "第二次后退回到最初会话");
            view.navigate_session_forward(cx);
            assert_eq!(b, view.current_session, "前进逐步恢复");
            view.navigate_session_forward(cx);
            assert_eq!(c, view.current_session, "前进恢复到最新");

            // 栈空时是 no-op，不是错误。
            view.navigate_session_forward(cx);
            assert_eq!(c, view.current_session);

            // 会话被删除/归档：导航栈里不能有它，后退要跳过它。
            view.session_navigation.remove(&a);
            view.navigate_session_back(cx);
            assert_eq!(
                b, view.current_session,
                "a 已消失，后退应直接落到 b（而不是 a）"
            );
        });
    }

    /// 只有「用户预期与实际不符」的那两档才开口提示。
    #[test]
    fn only_mismatched_session_continuity_reaches_the_user() {
        let cases = [
            (AcpSessionContinuity::StartedFresh, false),
            (AcpSessionContinuity::ReusedWithHistory, false),
            (AcpSessionContinuity::ReusedWithoutHistory, true),
            (AcpSessionContinuity::RestartedAfterReuseFailure, true),
        ];

        for (continuity, expect_notice) in cases {
            assert_eq!(
                expect_notice,
                super::acp_ui::session_continuity_notice(Some(continuity), "Agent").is_some(),
                "{continuity:?} 的提示策略不对"
            );
        }
        assert!(
            super::acp_ui::session_continuity_notice(None, "Agent").is_none(),
            "还没走到开会话这一步时不该猜一个结论"
        );
    }

    /// 本组提示依赖四个词条。缺词条时 `t!` 会原样返回 key，占位符写错则会留下 `%{..}`——
    /// 两种都会把内部标识直接显示给用户。
    #[test]
    fn context_notice_locales_resolve_and_interpolate() {
        let notices = [
            (
                "AgentUi.context_boundary_to_acp",
                t!("AgentUi.context_boundary_to_acp", name = "Agent").to_string(),
            ),
            (
                "AgentUi.context_boundary_to_local",
                t!("AgentUi.context_boundary_to_local", name = "Agent").to_string(),
            ),
            (
                "AgentUi.acp_session_restarted",
                t!("AgentUi.acp_session_restarted", name = "Agent").to_string(),
            ),
            (
                "AgentUi.acp_session_resumed_without_history",
                t!(
                    "AgentUi.acp_session_resumed_without_history",
                    name = "Agent"
                )
                .to_string(),
            ),
        ];

        for (key, text) in notices {
            assert_ne!(key, text, "词条 `{key}` 在 locales/ai_chat_view.yml 里缺失");
            assert!(
                !text.contains("%{"),
                "词条 `{key}` 的占位符没被替换，实际文案：{text}"
            );
        }
    }

    /// 连接前必须把「这个会话上次用的 ACP 会话」带上：不带的话重连等于一路新开空会话，
    /// agent 那边的上下文就接不上了。
    #[gpui::test]
    fn acp_connect_reuses_the_remembered_protocol_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");

        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(vec![AcpAgentEntry::ready(agent)], cx);
            let session_uid = view.current_session.clone();
            AppSettings::update(cx, |settings| {
                settings
                    .ai_chat
                    .remember_acp_session(&session_uid, "agent", "acp-remembered");
            });

            let (operation, _providers) = view
                .prepare_acp_connect(SharedString::from("agent"), cx)
                .expect("ready agent must start a connect");

            assert_eq!(Some("acp-remembered".to_string()), operation.resume);
        });
    }

    fn snapshot_of_external_session(
        uid: &str,
        agent_id: &str,
        protocol_id: &str,
    ) -> agent_runtime::SessionSnapshot {
        agent_runtime::SessionSnapshot {
            id: SessionId::from_string(uid.to_string()),
            resources: ResourceContext::new(),
            history: Vec::new(),
            plan: None,
            system_instruction: None,
            skills: agent_runtime::SkillContext::new(),
            workspace_root: None,
            draft: None,
            context_tokens: None,
            acp: Some(agent_runtime::AcpSessionRef {
                agent_id: agent_id.to_string(),
                session_id: protocol_id.to_string(),
            }),
        }
    }

    /// 从侧栏回到一条外部 agent 会话：要按快照里的地址把 agent 那边接回来，
    /// 而不是当成一条空的本地会话（那样用户会以为对话没了）。
    #[gpui::test]
    fn reopening_an_external_session_reconnects_to_its_agent(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.runtime
                .restore_session(snapshot_of_external_session("sess_acp", "codex", "acp-42"));
            let reference = view
                .runtime
                .session(&SessionId::from_string("sess_acp".to_string()))
                .expect("会话应在运行时里")
                .acp_ref()
                .expect("外部地址要跟着快照回到会话上");

            // 还没连在这个 agent 上：绕不开重新握手（不能就那么当成本地会话）。
            let plan = view.plan_acp_reopen("sess_acp", &reference, cx);
            assert!(
                matches!(
                    plan,
                    Some(super::acp_sessions::AcpReopenPlan::Reconnect(ref id, ref session))
                        if id.as_ref() == "codex" && session == "acp-42"
                ),
                "没连在这个 agent 上时应当重新握手，并带上要重开的协议会话"
            );
            assert_eq!(
                Some("acp-42"),
                AppSettings::current(cx)
                    .ai_chat
                    .remembered_acp_session("sess_acp", "codex"),
                "地址要补回记忆里，重连才会复用那条对话而不是新开一条"
            );

            // 真去握手时，重连带的就是那条协议会话。
            view.current_session = "sess_acp".to_string();
            let (operation, _providers) = view
                .prepare_acp_connect(agent_id, cx)
                .expect("agent 就绪时应当能发起握手");
            assert_eq!(Some("acp-42".to_string()), operation.resume);
        });
    }

    /// 快照里的地址与记忆分叉时，重开必须以**记忆**为准，不能顺手改回旧地址。
    ///
    /// 两个来源写入时机不同会分叉；快照地址可能是很早以前那条会话，用它重开 agent
    /// 会回到一段不含近期工作的旧对话（用户观感就是「它不记得前面做过的事」）。
    #[gpui::test]
    fn reopening_prefers_the_remembered_protocol_session_over_a_stale_snapshot(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let agent_id = SharedString::from("codex");
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new())
                .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                    agent_id.clone(),
                    "Codex",
                    "definitely-missing-acp-binary",
                ))]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.runtime.restore_session(snapshot_of_external_session(
                "sess_acp",
                "codex",
                "acp-stale",
            ));
            AppSettings::update(cx, |settings| {
                settings
                    .ai_chat
                    .remember_acp_session("sess_acp", "codex", "acp-current");
            });
            let reference = view
                .runtime
                .session(&SessionId::from_string("sess_acp".to_string()))
                .expect("会话应在运行时里")
                .acp_ref()
                .expect("外部地址要跟着快照回到会话上");

            let plan = view.plan_acp_reopen("sess_acp", &reference, cx);
            assert!(
                matches!(
                    plan,
                    Some(super::acp_sessions::AcpReopenPlan::Reconnect(ref id, ref session))
                        if id.as_ref() == "codex" && session == "acp-current"
                ),
                "记忆里的协议会话比快照地址新，重开要用记忆"
            );
            assert_eq!(
                Some("acp-current"),
                AppSettings::current(cx)
                    .ai_chat
                    .remembered_acp_session("sess_acp", "codex"),
                "不能把记忆改回快照里的旧地址"
            );
        });
    }

    /// agent 已经不在了：说清楚接不回来，不要默默变成一条空会话。
    #[gpui::test]
    fn reopening_an_external_session_without_its_agent_says_so(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.runtime.restore_session(snapshot_of_external_session(
                "sess_gone",
                "removed-agent",
                "acp-7",
            ));

            view.select_session("sess_gone", cx);

            assert!(!view.acp_connecting);
            assert!(view.acp_reopen_pending.is_none());
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content.contains("removed-agent")),
                "接不回来要写在转录里，别默认用户知道"
            );
        });
    }

    /// 重开历史会话的待办，绝不能因为「连接此刻不在手上」而被静默吞掉。
    ///
    /// 这就是「点开 ACP 会话消息列表是空的」的成因：`activate_acp` 先跑
    /// `reload_acp_sessions`（它为了发 `session/list` 会 `take()` 走连接），后跑重开
    /// 逻辑——那时 `open_protocol_session` 只能 `take()` 到 `None` 而直接返回，待办却
    /// 已经被吃掉，那段历史再也没有第二次机会补回来，日志里也看不到任何失败。
    #[gpui::test]
    fn a_pending_acp_reopen_survives_while_the_connection_is_away(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.acp_reopen_pending = Some("acp-42".to_string());
            // 连接不在手上：正是 `reload_acp_sessions` 把连接 `take()` 走之后的真实状态。
            view.acp = None;

            view.promote_pending_acp_session("acp-42", cx);

            assert_eq!(
                Some("acp-42".to_string()),
                view.acp_reopen_pending,
                "连接不在时不能消费待办——消费掉就再也没人去 load 那段历史了"
            );
        });
    }

    /// 待办只认它自己那条协议会话：换 agent / 换会话时不能被顺手用掉。
    #[gpui::test]
    fn a_pending_acp_reopen_is_not_spent_on_another_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.acp_reopen_pending = Some("acp-42".to_string());
            view.acp = None;

            view.promote_pending_acp_session("acp-99", cx);

            assert_eq!(
                Some("acp-42".to_string()),
                view.acp_reopen_pending,
                "连回来的不是待办里那条会话时，待办要原样留着"
            );
        });
    }

    /// 没记过的会话不能被凭空「恢复」到别人的会话上；换 agent 也不能串台。
    #[gpui::test]
    fn acp_connect_starts_fresh_without_a_matching_memory(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let agent = AcpAgentConfig::new("agent", "Agent", "noop");
        let other = AcpAgentConfig::new("other-agent", "Other", "noop");

        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(
                vec![AcpAgentEntry::ready(agent), AcpAgentEntry::ready(other)],
                cx,
            );
            let session_uid = view.current_session.clone();
            AppSettings::update(cx, |settings| {
                settings
                    .ai_chat
                    .remember_acp_session(&session_uid, "agent", "acp-for-agent");
            });

            let (first, _providers) = view
                .prepare_acp_connect(SharedString::from("agent"), cx)
                .expect("ready agent must start a connect");
            assert_eq!(Some("acp-for-agent".to_string()), first.resume);

            // 前一条连接已经结束，现在才轮得到为另一个 agent 发起连接。
            view.acp_connecting = false;
            view.acp_connecting_id = None;
            let (second, _providers) = view
                .prepare_acp_connect(SharedString::from("other-agent"), cx)
                .expect("another ready agent must start its own connect");
            assert_eq!(
                None, second.resume,
                "换 agent 时不能把前一个 agent 的会话 id 带过去"
            );
        });
    }

    #[gpui::test]
    fn acp_session_operation_guard_rejects_changed_or_closed_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, _cx| {
            let agent_id = SharedString::from("agent");
            let session_uid = view.current_session.clone();
            view.backend = Backend::Acp;
            view.current_acp_id = Some(agent_id.clone());
            let operation = view.next_acp_operation();

            assert!(view.is_current_acp_session_operation(operation, &agent_id, &session_uid));

            view.current_session = "replacement-session".into();
            assert!(!view.is_current_acp_session_operation(operation, &agent_id, &session_uid));

            view.current_session = session_uid.clone();
            view.closed_sessions.insert(session_uid.clone());
            assert!(!view.is_current_acp_session_operation(operation, &agent_id, &session_uid));
        });
    }

    #[gpui::test]
    fn stop_clears_only_current_session_pending_queue(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let current = view.current_session.clone();
            view.pending_submissions
                .enqueue(&current, pending_submission("current"));
            view.pending_submissions
                .enqueue("other-session", pending_submission("other"));
            view.set_running(true, cx);
            view.stop(cx);
        });

        view.read_with(cx, |view, _| {
            assert_eq!(0, view.pending_submissions.len(&view.current_session));
            assert_eq!(1, view.pending_submissions.len("other-session"));
            assert!(!view.is_running);
        });
    }

    #[gpui::test]
    fn queued_preview_follows_current_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let current = view.current_session.clone();
            view.pending_submissions
                .enqueue(&current, pending_submission("visible queue"));
            view.sync_pending_preview(cx);
        });
        let cx: &mut VisualTestContext = cx;
        assert!(
            cx.debug_bounds("agent-input-queued").is_some(),
            "the current session queue should render above the editor"
        );

        view.update(cx, |view, cx| view.start_fresh_session(cx));
        assert!(
            cx.debug_bounds("agent-input-queued").is_none(),
            "a fresh session must not show the previous session queue"
        );
    }

    #[gpui::test]
    fn background_session_completion_advances_its_own_queue(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::text("background one"),
            ModelResponse::text("background two"),
        ]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        let background_uid = view.update_in(cx, |view, window, cx| {
            let background_uid = view.current_session.clone();
            let input = view.input.clone();
            for text in ["background prompt one", "background prompt two"] {
                view.on_input_event(
                    &input,
                    &AgentInputEvent::Submit {
                        text: text.into(),
                        mentions: Vec::new(),
                        images: Vec::new(),
                    },
                    window,
                    cx,
                );
            }
            view.start_fresh_session(cx);
            background_uid
        });

        run_gpui_until(cx, || model.request_count() >= 2);
        cx.run_until_parked();

        view.read_with(cx, |view, _| {
            assert_ne!(background_uid, view.current_session);
            assert_eq!(0, view.pending_submissions.len(&background_uid));
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .all(|message| message.role != crate::ChatRole::User),
                "background queued prompts must not leak into the current transcript"
            );
            let background = view
                .session_transcripts
                .get(&background_uid)
                .expect("background transcript");
            let prompts = background
                .messages
                .iter()
                .filter(|message| message.role == crate::ChatRole::User)
                .map(|message| message.content.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                vec!["background prompt one", "background prompt two"],
                prompts
            );
        });
    }

    #[gpui::test]
    fn acp_events_must_match_the_prompt_owner_token_and_turn(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: SessionId::from_string("acp:owner"),
                session_uid: view.current_session.clone(),
                turn_id: agent_runtime::TurnId::from_string("turn-owner"),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.set_running(true, cx);
            let message_count = view.transcript.messages.len();

            view.apply_runtime_event(
                RuntimeEvent::TurnFailed {
                    session_id: SessionId::from_string("acp:stale"),
                    turn_id: agent_runtime::TurnId::from_string("turn-owner"),
                    reason: "stale token".into(),
                },
                cx,
            );
            view.apply_runtime_event(
                RuntimeEvent::TurnFailed {
                    session_id: SessionId::from_string("acp:owner"),
                    turn_id: agent_runtime::TurnId::from_string("turn-stale"),
                    reason: "stale turn".into(),
                },
                cx,
            );

            assert_eq!(message_count, view.transcript.messages.len());
            assert!(view.is_running);
            assert!(!view.acp_turn_owners.is_empty());
        });
    }

    /// 点开一条 ACP 历史会话时，`session/load` 重放出来的事件必须落到转录里。
    ///
    /// 这类事件**没有轮次可归**（没有 prompt、没有 owner），之前被过滤逻辑一律丢弃 ——
    /// 用户点开历史会话看到的就是一片空白。
    #[gpui::test]
    fn acp_history_replay_lands_without_a_turn_owner(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            view.acp_turn_owners.clear();
            let event_session_id = SessionId::from_string("acp:replay");
            let turn_id = TurnId::from_string("acp-replay:1");
            view.acp_history_replay = Some(AcpHistoryReplay {
                event_session_id: event_session_id.clone(),
                session_uid: view.current_session.clone(),
                turn_id: turn_id.clone(),
            });
            let message_count = view.transcript.messages.len();

            // 别的连接推来的事件：不放行。
            view.apply_runtime_event(
                RuntimeEvent::UserMessage {
                    session_id: SessionId::from_string("acp:other"),
                    turn_id: turn_id.clone(),
                    text: "别的会话的历史".into(),
                },
                cx,
            );
            // 同一个连接、但不是回放轮次：也不放行 —— 窗口只认那一个轮次。
            view.apply_runtime_event(
                RuntimeEvent::UserMessage {
                    session_id: event_session_id.clone(),
                    turn_id: TurnId::from_string("turn-live"),
                    text: "不是回放".into(),
                },
                cx,
            );
            assert_eq!(message_count, view.transcript.messages.len());

            view.apply_runtime_event(
                RuntimeEvent::UserMessage {
                    session_id: event_session_id.clone(),
                    turn_id: turn_id.clone(),
                    text: "历史上问过的问题".into(),
                },
                cx,
            );
            view.apply_runtime_event(
                RuntimeEvent::AssistantMessage {
                    session_id: event_session_id,
                    turn_id,
                    text: "历史上答过的话".into(),
                },
                cx,
            );

            assert!(!view.is_running, "回放不是一轮，不能把界面带回「正在响应」");
        });

        view.read_with(cx, |view, _| {
            let texts = view
                .transcript
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>();
            assert!(texts.contains(&"历史上问过的问题"));
            assert!(texts.contains(&"历史上答过的话"));
            assert!(!texts.contains(&"别的会话的历史"));
            assert!(!texts.contains(&"不是回放"));
        });
    }

    /// 事件泵必须放行**子代理详情会话**的事件。
    ///
    /// 这条判据挡错一次，表现是「点了卡片、右侧面板一片空白」，而且日志里什么都不
    /// 会留下：事件在进视图之前就被当成「别的会话的」丢掉了。
    #[test]
    fn the_event_pump_accepts_subagent_detail_sessions() {
        let main = SessionId::from_string("acp:3f1a");
        let detail = SessionId::from_string("acp-sub:ses_child");

        let detail_event = RuntimeEvent::AssistantMessage {
            session_id: detail,
            turn_id: detail_turn_id_for("ses_child"),
            text: "子代理的推理".into(),
        };
        let other_event = RuntimeEvent::AssistantMessage {
            session_id: SessionId::from_string("acp:other"),
            turn_id: TurnId::from_string("t"),
            text: "别的连接".into(),
        };

        assert!(
            runtime_event_matches_session(&detail_event, Some(&main)),
            "详情会话的事件同属这条连接，挡下来就等于整段推理静默丢失"
        );
        assert!(
            !runtime_event_matches_session(&other_event, Some(&main)),
            "别的连接的事件仍然要挡"
        );
        // 没有过滤时一律放行（本地后端）。
        assert!(runtime_event_matches_session(&other_event, None));
    }

    /// 详情事件必须落进**它自己**的转录，而不是主转录。
    #[gpui::test]
    fn subagent_detail_events_land_in_their_own_transcript(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        let detail_uid = view.update(cx, |view, cx| {
            view.backend = Backend::Acp;
            let detail_uid = detail_session_id_for("ses_child");
            let main_before = view.transcript.messages.len();
            view.apply_runtime_event(
                RuntimeEvent::AssistantMessage {
                    session_id: SessionId::from_string(detail_uid.clone()),
                    turn_id: detail_turn_id_for("ses_child"),
                    text: "子代理查了连接数".into(),
                },
                cx,
            );
            assert_eq!(
                main_before,
                view.transcript.messages.len(),
                "详情回放不能灌进主转录"
            );
            detail_uid
        });

        view.read_with(cx, |view, _| {
            let detail = view
                .subagent_details
                .get(&detail_uid)
                .expect("详情事件要落进自己的转录");
            assert!(
                detail
                    .messages
                    .iter()
                    .any(|message| message.content.contains("子代理查了连接数")),
                "详情转录要带上回放的内容"
            );
            assert!(
                !view.is_running,
                "详情回放不是一轮，不能把界面带回「正在响应」"
            );
            assert!(
                view.session_transcripts.is_empty(),
                "详情会话不该被当成「另一条真实会话」缓存，那会被 LRU 挤掉"
            );
        });
    }

    #[gpui::test]
    fn closed_session_late_event_does_not_recreate_transcript(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        let closed_uid = view.update(cx, |view, cx| {
            let closed_uid = view.current_session.clone();
            view.start_fresh_session(cx);
            view.discard_live_session(&closed_uid);
            view.apply_runtime_event(
                RuntimeEvent::TurnFailed {
                    session_id: SessionId::from_string(closed_uid.clone()),
                    turn_id: TurnId::from_string("late-closed-turn"),
                    reason: "late failure".into(),
                },
                cx,
            );
            closed_uid
        });

        view.read_with(cx, |view, _| {
            assert!(
                !view.session_transcripts.contains_key(&closed_uid),
                "a late event must not recreate a deleted or archived transcript"
            );
            assert!(
                !view
                    .live_sessions
                    .iter()
                    .any(|summary| summary.id == closed_uid),
                "a late event must not recreate a deleted or archived session summary"
            );
        });
    }

    #[gpui::test]
    fn acp_need_user_input_keeps_owner_running_and_queue_paused(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let event_session_id = SessionId::from_string("acp:need-input");
            let turn_id = TurnId::from_string("turn-needs-input");
            view.backend = Backend::Acp;
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: event_session_id.clone(),
                session_uid: view.current_session.clone(),
                turn_id: turn_id.clone(),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.pending_submissions
                .enqueue(&view.current_session, pending_submission("queued"));
            view.set_running(true, cx);

            view.apply_runtime_event(
                RuntimeEvent::NeedUserInput {
                    session_id: event_session_id,
                    turn_id,
                    question: "approve?".into(),
                    pending_tool_call_id: None,
                    tool_name: None,
                    arguments: None,
                    pending_tool_calls: Vec::new(),
                },
                cx,
            );
        });

        view.read_with(cx, |view, _| {
            assert!(view.is_running);
            assert!(!view.acp_turn_owners.is_empty());
            assert_eq!(1, view.pending_submissions.len(&view.current_session));
        });
    }

    #[gpui::test]
    fn acp_turn_cancelled_without_ready_connection_keeps_post_stop_queue(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let event_session_id = SessionId::from_string("acp:cancel");
            let turn_id = TurnId::from_string("turn-cancelled");
            view.backend = Backend::Acp;
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: event_session_id.clone(),
                session_uid: view.current_session.clone(),
                turn_id: turn_id.clone(),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.pending_submissions.enqueue(
                &view.current_session,
                pending_submission("queued after stop"),
            );
            view.set_running(true, cx);

            view.apply_runtime_event(
                RuntimeEvent::TurnCancelled {
                    session_id: event_session_id,
                    turn_id,
                },
                cx,
            );
        });

        view.read_with(cx, |view, _| {
            assert!(!view.is_running);
            assert!(view.acp_turn_owners.is_empty());
            assert_eq!(
                1,
                view.pending_submissions.len(&view.current_session),
                "without a Ready connection, the post-stop queue must remain for retry"
            );
        });
    }

    #[gpui::test]
    fn discarded_acp_owner_is_retained_until_terminal_without_recreating_closed_session(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        let closed_uid = view.update(cx, |view, cx| {
            let closed_uid = view.current_session.clone();
            view.start_fresh_session(cx);
            let current_uid = view.current_session.clone();
            let event_session_id = SessionId::from_string("acp:discarded-owner");
            let turn_id = TurnId::from_string("turn-discarded-owner");
            view.backend = Backend::Acp;
            view.acp_turn_owners = vec![AcpTurnOwner {
                event_session_id: event_session_id.clone(),
                session_uid: closed_uid.clone(),
                turn_id: turn_id.clone(),
                backgrounded: false,
                cancel_requested: false,
            }];
            view.set_session_running(&closed_uid, true, cx);
            view.pending_submissions
                .enqueue(&current_uid, pending_submission("current remains queued"));

            view.discard_live_session(&closed_uid);

            assert!(
                !view.acp_turn_owners.is_empty(),
                "the owner token must survive until its matching terminal event"
            );
            view.apply_runtime_event(
                RuntimeEvent::TurnCancelled {
                    session_id: event_session_id,
                    turn_id,
                },
                cx,
            );
            closed_uid
        });

        view.read_with(cx, |view, _| {
            assert!(view.acp_turn_owners.is_empty());
            assert!(!view.running_sessions.contains(&closed_uid));
            assert!(!view.session_transcripts.contains_key(&closed_uid));
            assert!(
                !view
                    .live_sessions
                    .iter()
                    .any(|summary| summary.id == closed_uid)
            );
            assert_eq!(
                1,
                view.pending_submissions.len(&view.current_session),
                "a closed-owner terminal without a Ready connection must not consume current FIFO"
            );
        });
    }

    #[gpui::test]
    fn ignored_local_turn_event_does_not_touch_new_turn_state(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let ignored_turn = TurnId::from_string("stopped-old-turn");
            view.ignored_local_turns.insert(ignored_turn.clone());
            view.set_running(true, cx);
            view.apply_runtime_event(
                RuntimeEvent::TurnCancelled {
                    session_id: view.session_id.clone(),
                    turn_id: ignored_turn.clone(),
                },
                cx,
            );
            assert!(
                !view.ignored_local_turns.contains(&ignored_turn),
                "the ignored terminal marker should be released"
            );
        });

        assert!(view.read_with(cx, |view, _| view.is_running));
    }

    #[test]
    fn applying_mentioned_resource_adds_from_catalog_and_sets_default() {
        let mut resources = ResourceContext::new();
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
        ];
        let mentions = vec![MentionItem::new("db-a", "prod-db", "mysql", "mysql")];

        assert!(apply_mentioned_resources(
            &mut resources,
            &catalog,
            &mentions
        ));

        assert_eq!(1, resources.resources.len());
        assert_eq!(
            Some("prod-db"),
            resources.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn resource_pool_items_mark_pool_membership_and_default_target() {
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        let items = resource_pool_items(&pool, &catalog);

        assert_eq!(2, items.len());
        assert_eq!(items[0].id.as_ref(), "ssh-a");
        assert!(items[0].in_pool);
        assert!(items[0].is_default);
        assert_eq!(items[1].id.as_ref(), "ssh-b");
        assert!(!items[1].in_pool);
        assert!(!items[1].is_default);
    }

    #[test]
    fn resource_pool_item_primary_meta_does_not_fallback_to_uuid() {
        let resource = ResourceRef::new(
            "fa9476d8-de90-4f7d-9b63-6f4783594211",
            ResourceKind::Other("rdp".into()),
            "a82 bi 服务",
        );

        assert_eq!(resource_primary_meta(&resource), "rdp");
    }

    #[test]
    fn resource_pool_item_primary_meta_skips_uuid_alias() {
        let resource = ResourceRef::new("rdp-a", ResourceKind::Other("rdp".into()), "a82 bi 服务")
            .with_alias("abfcee0a-2827-4588-9f6-587a7a95d1e9")
            .with_alias("10.1.131.181");

        assert_eq!(resource_primary_meta(&resource), "10.1.131.181");
    }

    #[test]
    fn resource_pool_item_uses_specific_icons_for_known_other_kinds() {
        assert_eq!(kind_icon(&ResourceKind::Other("rdp".into())), "RD");
        assert_eq!(kind_icon(&ResourceKind::Other("vnc".into())), "VN");
        assert_eq!(
            kind_icon(&ResourceKind::Other("port-forwarding".into())),
            "PF"
        );
    }

    #[test]
    fn resource_source_options_mark_all_when_pool_matches_catalog() {
        let pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
        let catalog = pool.resources.clone();

        let options = resource_source_options(&pool, &catalog);

        assert!(source_option(&options, "all").selected);
        assert_eq!(source_option(&options, "all").count, 2);
        assert!(!source_option(&options, "current").selected);
    }

    #[test]
    fn resource_source_options_mark_manual_for_mixed_subset() {
        let pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
            ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
        ];

        let options = resource_source_options(&pool, &catalog);

        assert!(source_option(&options, "manual").selected);
        assert_eq!(source_option(&options, "ssh").count, 2);
        assert_eq!(source_option(&options, "redis").count, 1);
    }

    fn source_option<'a>(
        options: &'a [ComposerResourceSourceOption],
        id: &str,
    ) -> &'a ComposerResourceSourceOption {
        options
            .iter()
            .find(|option| option.id.as_ref() == id)
            .unwrap()
    }

    #[test]
    fn apply_resource_source_all_replaces_pool_with_catalog() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        assert!(apply_resource_source(&mut pool, &catalog, "all"));
        assert_eq!(2, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn apply_resource_source_ssh_selects_only_ssh_resources() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "redis-a",
            ResourceKind::Redis,
            "cache",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
        ];

        assert!(apply_resource_source(&mut pool, &catalog, "ssh"));
        assert_eq!(1, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn add_resource_to_pool_uses_catalog_resource() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        assert!(add_resource_to_pool(&mut pool, &catalog, "ssh-b"));
        assert_eq!(2, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn mentioned_catalog_resources_are_added_to_pool_and_set_default() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "db-a",
            ResourceKind::Mysql,
            "prod-db",
        ));
        let catalog = vec![
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];
        let mentions = vec![
            MentionItem::new("ssh-a", "prod-a", "ssh", "ssh"),
            MentionItem::new("ssh-b", "prod-b", "ssh", "ssh"),
        ];

        assert!(apply_mentioned_resources(&mut pool, &catalog, &mentions));
        assert_eq!(3, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
        assert!(
            pool.resources
                .iter()
                .any(|resource| resource.label == "prod-b")
        );
    }

    #[test]
    fn remove_default_resource_reassigns_default_target() {
        let mut pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

        assert!(remove_resource_from_pool(&mut pool, "ssh-a"));
        assert_eq!(1, pool.resources.len());
        assert_eq!(
            Some("prod-b"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn execution_mode_ids_map_to_runtime_modes() {
        assert_eq!(ToolExecutionMode::Auto, tool_execution_mode_from_id("auto"));
        assert_eq!(
            ToolExecutionMode::ReadOnly,
            tool_execution_mode_from_id("readonly")
        );
        assert_eq!(
            ToolExecutionMode::Manual,
            tool_execution_mode_from_id("manual")
        );
        assert_eq!(
            ToolExecutionMode::Manual,
            tool_execution_mode_from_id("nope")
        );
    }

    #[test]
    fn settings_execution_mode_round_trips_to_runtime_mode() {
        assert_eq!(
            ToolExecutionMode::Auto,
            runtime_tool_execution_mode(AiChatToolExecutionMode::Auto)
        );
        assert_eq!(
            ToolExecutionMode::ReadOnly,
            runtime_tool_execution_mode(AiChatToolExecutionMode::ReadOnly)
        );
        assert_eq!(
            ToolExecutionMode::Manual,
            runtime_tool_execution_mode(AiChatToolExecutionMode::Manual)
        );
        assert_eq!(
            AiChatToolExecutionMode::Auto,
            settings_tool_execution_mode(ToolExecutionMode::Auto)
        );
    }

    #[test]
    fn composer_exposes_ask_then_tool_execution_modes() {
        let options = default_execution_mode_options();

        // 「问答」是任务类型，与工具策略共用同一个下拉 id 空间，排在首位。
        assert_eq!(
            vec!["ask", "auto", "readonly", "manual"],
            options
                .iter()
                .map(|option| option.id.as_ref())
                .collect::<Vec<_>>()
        );
        assert_eq!(options[0].label.as_ref(), t!("AgentUi.ask_mode").as_ref());
        assert_eq!(options[1].label.as_ref(), t!("AgentUi.auto").as_ref());
    }

    #[test]
    fn execution_mode_ids_round_trip_to_task_kind_and_tool_policy() {
        // 「问答」改任务类型且不动工具策略；其余改工具策略且任务类型回到常规。
        assert_eq!(TaskKind::Ask, task_kind_from_id("ask"));
        assert_eq!(TaskKind::Agent, task_kind_from_id("auto"));
        assert_eq!(TaskKind::Agent, task_kind_from_id("readonly"));
        assert_eq!(TaskKind::Agent, task_kind_from_id("manual"));

        // 未知 id 不应把任务类型带偏。
        assert_eq!(TaskKind::Agent, task_kind_from_id("acp-mode:plan"));

        assert_eq!(
            TaskKind::Ask,
            task_kind_from_settings(AiChatToolExecutionMode::Ask)
        );
        assert_eq!(
            TaskKind::Agent,
            task_kind_from_settings(AiChatToolExecutionMode::Manual)
        );
    }

    #[test]
    fn acp_modes_are_namespaced_so_they_cannot_collide_with_local_ids() {
        // ACP 的会话模式 id 由 agent 自己给定，可能与本地的 `auto` / `manual` 同名，
        // 因此统一切到 `acp-mode:` 前缀，避免 `select_execution_mode` 走错分支。
        let id = acp_mode_option_id("auto");
        assert_eq!("acp-mode:auto", id);
        assert_eq!(Some("auto"), id.strip_prefix(ACP_MODE_OPTION_PREFIX));
        assert_eq!(TaskKind::Agent, task_kind_from_id(&id));
    }

    #[test]
    fn acp_backend_exposes_agent_declared_session_modes() {
        use agent_client_protocol::schema::v1::{
            NewSessionResponse, SessionMode, SessionModeState,
        };

        let mut state = AcpSessionState::default();
        state.apply_new_session_response(&NewSessionResponse::new("s1").modes(
            SessionModeState::new(
                "ask",
                vec![
                    SessionMode::new("ask", "Ask"),
                    SessionMode::new("code", "Code"),
                ],
            ),
        ));

        let options = acp_execution_mode_options(&state);

        // ACP 下拉里是 agent 自己声明的档位（带前缀防止与本地 id 撞车），
        // 不是本地那套 auto/readonly/manual。
        assert_eq!(
            vec!["acp-mode:ask", "acp-mode:code"],
            options
                .iter()
                .map(|option| option.id.as_ref())
                .collect::<Vec<_>>()
        );
        assert_eq!(options[1].label.as_ref(), "Code");
        assert_eq!(Some("Ask".to_string()), acp_mode_label(&state));
    }

    #[test]
    fn acp_config_mode_options_are_exposed_and_namespaced() {
        use agent_client_protocol::schema::v1::{
            NewSessionResponse, SessionConfigOption, SessionConfigOptionCategory,
            SessionConfigSelectOption,
        };

        let mut state = AcpSessionState::default();
        state.apply_new_session_response(
            &NewSessionResponse::new("s1").config_options(vec![
                SessionConfigOption::select(
                    "mode",
                    "Session Mode",
                    "build",
                    vec![
                        SessionConfigSelectOption::new("build", "build")
                            .description("The default agent."),
                        SessionConfigSelectOption::new("plan", "plan"),
                        SessionConfigSelectOption::new("scout", "scout"),
                    ],
                )
                .category(SessionConfigOptionCategory::Mode),
            ]),
        );

        let options = acp_execution_mode_options(&state);

        assert_eq!(
            vec![
                "acp-config-mode:build",
                "acp-config-mode:plan",
                "acp-config-mode:scout",
            ],
            options
                .iter()
                .map(|option| option.id.as_ref())
                .collect::<Vec<_>>()
        );
        // 走 config 分支的 id 不能撞上传统前缀，且必须解析回 Agent。
        assert_eq!(TaskKind::Agent, task_kind_from_id(options[0].id.as_ref()));
        assert_eq!(
            Some("build"),
            options[0]
                .id
                .as_ref()
                .strip_prefix(ACP_CONFIG_MODE_OPTION_PREFIX)
        );
        assert_eq!(Some("build".to_string()), acp_mode_label(&state));
    }

    #[test]
    fn composer_context_includes_plan_items_for_local_and_acp_backends() {
        let plan = PlanCardData {
            goal: "上线检查".to_string(),
            status: "running".to_string(),
            steps: vec![crate::agent_cards::PlanStepData {
                title: "检查连接".to_string(),
                description: "确认服务可达".to_string(),
                status: "running".to_string(),
                risk: "只读".to_string(),
                tool: Some("ping".to_string()),
            }],
        };
        let acp_id = SharedString::from("codex");
        let acp_agents = vec![AcpAgentEntry::ready(AcpAgentConfig::new(
            acp_id.clone(),
            "Codex ACP",
            "codex",
        ))];

        let local = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
            Some(&plan),
            &[],
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );
        let acp = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
            Some(&plan),
            &[],
            Backend::Acp,
            &acp_agents,
            Some(&acp_id),
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );

        assert_eq!(local.plan_items, acp.plan_items);
        assert_eq!(local.plan_items[0].title.as_ref(), "检查连接");
        assert_eq!(local.plan_items[0].description.as_ref(), "确认服务可达");
        assert_eq!(local.plan_items[0].risk.as_ref(), "只读");
        assert_eq!(
            local.plan_items[0].tool.as_ref().map(|s| s.as_ref()),
            Some("ping")
        );
        assert!(local.agent_options[0].selected);
        assert!(acp.agent_options[1].selected);
    }

    #[test]
    fn local_backend_option_is_not_named_after_a_specific_cli() {
        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
            None,
            &[],
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );

        assert_eq!(ctx.agent_options[0].label.as_ref(), "One Agent");
    }

    #[test]
    fn composer_context_includes_running_subagents() {
        let subagents = vec![
            SubAgentCardData {
                subagent_id: "sub_1".into(),
                name: "reviewer".into(),
                task: "检查事件流".into(),
                running: true,
                success: None,
                summary: "正在读取事件".into(),
            },
            SubAgentCardData {
                subagent_id: "sub_2".into(),
                name: "done".into(),
                task: "已完成任务".into(),
                running: false,
                success: Some(true),
                summary: "完成".into(),
            },
        ];

        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
            None,
            &subagents,
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );

        assert_eq!(ctx.subagent_items.len(), 2);
        assert_eq!(ctx.subagent_items[0].name.as_ref(), "reviewer");
        assert_eq!(ctx.subagent_items[0].task.as_ref(), "检查事件流");
        assert_eq!(ctx.subagent_items[0].summary.as_ref(), "正在读取事件");
        assert_eq!(ctx.subagent_items[0].status.as_ref(), "running");
        assert_eq!(ctx.subagent_items[1].name.as_ref(), "done");
        assert_eq!(ctx.subagent_items[1].status.as_ref(), "completed");
    }

    #[test]
    fn header_agent_switcher_lists_and_labels_multiple_acp_agents() {
        let codex_id = SharedString::from("codex");
        let opencode_id = SharedString::from("opencode");
        let acp_agents = vec![
            AcpAgentEntry::ready(AcpAgentConfig::new(codex_id.clone(), "Codex", "codex")),
            AcpAgentEntry::ready(AcpAgentConfig::new(
                opencode_id.clone(),
                "OpenCode",
                "opencode",
            )),
        ];

        let options = composer_agent_options(Backend::Acp, &acp_agents, Some(&opencode_id), false);
        let labels = options
            .iter()
            .map(|option| option.label.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(vec!["One Agent", "Codex", "OpenCode"], labels);
        assert!(options[2].selected);
        assert_eq!(
            "OpenCode",
            current_agent_label(Backend::Acp, &acp_agents, Some(&opencode_id), false).as_ref()
        );
    }

    #[test]
    fn invalid_acp_agent_remains_visible_but_disabled() {
        let diagnostic = AcpConfigDiagnostic::new("缺少环境变量 OPENAI_API_KEY");
        let entries = vec![AcpAgentEntry::invalid("codex", "Codex", diagnostic.clone())];

        let options = composer_agent_options(Backend::Local, &entries, None, false);

        assert_eq!(2, options.len());
        assert_eq!("Codex", options[1].label.as_ref());
        assert!(!options[1].enabled);
        assert_eq!(diagnostic.message, options[1].subtitle.as_ref());
        assert!(agent_option_disabled(&options[1]));
    }

    #[test]
    fn disconnected_acp_agent_is_not_treated_as_an_active_selection() {
        let selected = SharedString::from("codex");

        assert!(!acp_options::agent_selection_is_active(
            Backend::Local,
            Some(&selected),
            false,
            &selected,
        ));
        assert!(!acp_options::agent_selection_is_active(
            Backend::Acp,
            Some(&selected),
            false,
            &selected,
        ));
        assert!(acp_options::agent_selection_is_active(
            Backend::Acp,
            Some(&selected),
            true,
            &selected,
        ));
    }

    #[gpui::test]
    fn gpui_refresh_acp_agents_updates_header_switcher_options(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                "codex", "Codex", "codex",
            ))]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(
                vec![
                    AcpAgentEntry::ready(AcpAgentConfig::new("codex", "Codex", "codex")),
                    AcpAgentEntry::ready(AcpAgentConfig::new("opencode", "OpenCode", "opencode")),
                ],
                cx,
            );
        });

        let labels = view.read_with(cx, |view, _| {
            composer_agent_options(
                view.backend,
                &view.acp_agents,
                view.current_acp_id.as_ref(),
                view.acp_connecting,
            )
            .iter()
            .map(|option| option.label.as_ref().to_string())
            .collect::<Vec<_>>()
        });

        assert_eq!(vec!["One Agent", "Codex", "OpenCode"], labels);
    }

    #[test]
    fn header_agent_switcher_keeps_local_available_while_acp_connects() {
        let acp_agents = vec![AcpAgentEntry::ready(AcpAgentConfig::new(
            "codex", "Codex", "codex",
        ))];
        let options = composer_agent_options(Backend::Local, &acp_agents, None, true);

        assert!(!agent_option_disabled(&options[0]));
        assert!(agent_option_disabled(&options[1]));
        assert_eq!(
            t!("AgentUi.connecting").as_ref(),
            current_agent_label(Backend::Local, &acp_agents, None, true).as_ref()
        );
    }

    #[test]
    fn composer_context_maps_acp_state_to_visible_context() {
        use agent_client_protocol::schema::v1::{
            AgentCapabilities, AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate,
            SessionInfoUpdate, SessionMode, SessionModeState, SessionUpdate, UsageUpdate,
        };

        let mut state = AcpSessionState::default();
        state.set_agent_capabilities(AgentCapabilities::new().load_session(true));
        state.apply_new_session_response(
            &agent_client_protocol::schema::v1::NewSessionResponse::new("s1").modes(
                SessionModeState::new(
                    "ask",
                    vec![
                        SessionMode::new("ask", "Ask"),
                        SessionMode::new("code", "Code"),
                    ],
                ),
            ),
        );
        state.apply_session_update(&SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![AvailableCommand::new("plan", "Create plan")]),
        ));
        state.apply_session_update(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            "code",
        )));
        state.apply_session_update(&SessionUpdate::SessionInfoUpdate(
            SessionInfoUpdate::new().title("ACP 工作会话"),
        ));
        state.apply_session_update(&SessionUpdate::UsageUpdate(UsageUpdate::new(42, 100)));

        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            None,
            None,
            &[],
            Backend::Acp,
            &[],
            None,
            false,
            Some(state),
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );

        assert_eq!(ctx.target.unwrap().label.as_ref(), "ACP 工作会话");
        assert_eq!(ctx.scopes[0].value.as_ref(), "Code");
        assert_eq!(ctx.scopes[1].value.as_ref(), "42/100 tokens");
        assert!(ctx.capabilities.contains(&SharedString::from("ACP")));
        assert!(
            ctx.capabilities
                .contains(&SharedString::from(t!("AgentUi.load_session").to_string()))
        );
        assert!(
            ctx.capabilities
                .contains(&SharedString::from(format!("{}:1", t!("AgentUi.commands"))))
        );
    }

    #[test]
    fn local_backend_reports_context_usage_with_a_best_effort_window() {
        let model = ComposerModelOption::new(
            "deepseek-chat-option",
            "deepseek",
            "DeepSeek",
            "deepseek-chat",
        );
        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            Some(&model),
            None,
            &[],
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            Some(1500),
        );

        let usage = ctx
            .scopes
            .iter()
            .find(|scope| scope.key == "local-usage")
            .expect("本地会话报过计量就应有用量 scope");
        assert_eq!(usage.value.as_ref(), "1500/128000 tokens");
    }

    #[test]
    fn local_usage_without_a_known_window_shows_tokens_only() {
        let model =
            ComposerModelOption::new("custom-option", "custom", "Custom", "my-private-finetune");
        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            Some(&model),
            None,
            &[],
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            Some(1500),
        );

        let usage = ctx
            .scopes
            .iter()
            .find(|scope| scope.key == "local-usage")
            .expect("窗口未知也应展示 token 数");
        assert_eq!(usage.value.as_ref(), "1500 tokens");
    }

    #[test]
    fn local_usage_scope_is_absent_until_the_model_reports_something() {
        let model = ComposerModelOption::new(
            "deepseek-chat-option",
            "deepseek",
            "DeepSeek",
            "deepseek-chat",
        );
        let ctx = build_composer_context(
            &ResourceContext::new(),
            ExecutionSelection::tool(ToolExecutionMode::Auto),
            Some(&model),
            None,
            &[],
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
            None,
        );

        assert!(
            !ctx.scopes.iter().any(|scope| scope.key == "local-usage"),
            "模型还没报过计量,不该装作测过(空 chip 比假数据好)"
        );
    }

    #[test]
    fn slash_commands_come_from_acp_state_verbatim() {
        use agent_client_protocol::schema::v1::{
            AvailableCommand, AvailableCommandInput, AvailableCommandsUpdate, SessionUpdate,
            UnstructuredCommandInput,
        };

        let state = AcpSessionState::default();
        assert!(slash_commands_from_acp_state(&state).is_empty());

        let mut state = AcpSessionState::default();
        state.apply_session_update(&SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![
                AvailableCommand::new("plan", "Create plan"),
                AvailableCommand::new("research", "Research the repo").input(
                    AvailableCommandInput::Unstructured(UnstructuredCommandInput::new(
                        "what to research",
                    )),
                ),
            ]),
        ));

        let items = slash_commands_from_acp_state(&state);
        assert_eq!(
            vec!["plan", "research"],
            items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(None, items[0].input_hint);
        assert_eq!(Some("what to research"), items[1].input_hint.as_deref());
        assert_eq!("/research ", items[1].insert_text());
    }

    #[test]
    fn usage_scope_appends_cost_when_the_agent_reports_one() {
        use agent_client_protocol::schema::v1::Cost;

        let unbilled = AcpUsage {
            used: 42,
            size: 100,
            cost: None,
        };
        assert_eq!("42/100 tokens", format_acp_usage(&unbilled));

        let billed = AcpUsage {
            used: 42,
            size: 100,
            cost: Some(Cost::new(0.0123, "USD")),
        };
        assert_eq!("42/100 tokens · 0.0123 USD", format_acp_usage(&billed));

        let fractional = AcpUsage {
            used: 1,
            size: 2,
            cost: Some(Cost::new(2.5, "EUR")),
        };
        assert_eq!("1/2 tokens · 2.5 EUR", format_acp_usage(&fractional));
    }

    #[gpui::test]
    fn gpui_defaults_to_manual_execution_mode(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.read_with(cx, |view, _| {
            assert_eq!(ToolExecutionMode::Manual, view.tool_execution_mode);
        });
    }

    #[gpui::test]
    fn gpui_explicit_auto_execution_mode_takes_effect(cx: &mut TestAppContext) {
        init_test_ui(cx);
        cx.update(|cx| {
            AppSettings::update(cx, |settings| {
                settings.ai_chat.tool_execution_mode = AiChatToolExecutionMode::Auto;
            });
        });
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.read_with(cx, |view, _| {
            assert_eq!(ToolExecutionMode::Auto, view.tool_execution_mode);
        });
    }

    #[gpui::test]
    fn gpui_submit_readonly_tool_mode_filters_write_tools(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::ReadOnly;
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "只读分析".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        let tool_names = requests[0]
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"echo"));
        assert!(!tool_names.contains(&"write_data"));
    }

    #[gpui::test]
    fn gpui_tool_approval_click_is_not_blocked_by_running_flag(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        view.update(cx, |view, cx| {
            view.is_running = true;
            view.resolve_tool_call("c_write".into(), true, cx);
        });
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_acp_permission_action_resolves_message_card_with_original_option_id(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            view.receive_acp_permission(envelope, cx);
        });
        cx.dispatch_action(SelectAcpPermissionOption {
            request_id: "session:call".into(),
            option_id: "allow".into(),
        });
        cx.run_until_parked();

        assert_eq!(
            AcpPermissionOutcome::Selected {
                option_id: "allow".into(),
            },
            outcome_rx.try_recv().expect("ACP permission response")
        );
        let data = view.read_with(cx, |view, _| {
            let message = view
                .transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .expect("ACP permission card");
            AcpPermissionCardData::from_json(&message.content).expect("card data")
        });
        assert_eq!("approved", data.status);
        assert_eq!("仅本次允许", data.selected_option_name);
        assert_eq!(
            t!(
                "AgentUi.acp_safety_confirmation_notice",
                summary = test_acp_permission_request().summary
            ),
            data.summary
        );
    }

    #[gpui::test]
    fn gpui_acp_permission_card_omits_second_approval_notice_in_auto_mode(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let request = test_acp_permission_request();
        let (envelope, _outcome_rx) = AcpPermissionEnvelope::new(request.clone());

        view.update(cx, |view, cx| {
            view.tool_execution_mode = ToolExecutionMode::Auto;
            view.receive_acp_permission(envelope, cx);
        });

        let data = view.read_with(cx, |view, _| {
            let message = view
                .transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .expect("ACP permission card");
            AcpPermissionCardData::from_json(&message.content).expect("card data")
        });
        assert_eq!(request.summary, data.summary);
    }

    #[gpui::test]
    fn gpui_acp_permission_button_resolves_without_opening_dialog(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let granted = Arc::new(AtomicUsize::new(0));
        let revoked = Arc::new(AtomicUsize::new(0));
        let granted_arguments = Arc::new(std::sync::Mutex::new(None));
        cx.update({
            let granted = granted.clone();
            let revoked = revoked.clone();
            let granted_arguments = granted_arguments.clone();
            move |cx| {
                crate::set_acp_permission_grant_provider(
                    cx,
                    move |request, option, _public_mcp_provider| {
                        if !option.kind.starts_with("allow") {
                            return None;
                        }
                        granted.fetch_add(1, Ordering::SeqCst);
                        *granted_arguments.lock().expect("granted arguments lock") =
                            request.raw_input().cloned();
                        let revoked = revoked.clone();
                        Some(crate::AcpPermissionGrant::new(move || {
                            revoked.fetch_add(1, Ordering::SeqCst);
                        }))
                    },
                );
            }
        });
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| {
            let _ = view.start_acp_client_session(cx);
            view.transcript.apply(&RuntimeEvent::ToolCallStarted {
                session_id: view.session_id.clone(),
                turn_id: agent_runtime::TurnId::from_string("turn"),
                call_id: ToolCallId::from_string("call"),
                tool_name: ToolName::new("terminal.exec"),
                kind: agent_runtime::ToolAction::Execute,
                arguments: json!({
                    "target": "haiwai comi",
                    "command": "du -xhd1 /"
                }),
            });
            view.receive_acp_permission(envelope, cx);
        });
        cx.run_until_parked();
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("ACP allow button should render in the message list");
        cx.simulate_click(allow.center(), Modifiers::default());
        cx.run_until_parked();

        assert_eq!(
            AcpPermissionOutcome::Selected {
                option_id: "allow".into(),
            },
            outcome_rx.try_recv().expect("ACP permission response")
        );
        assert_eq!(1, granted.load(Ordering::SeqCst));
        assert_eq!(0, revoked.load(Ordering::SeqCst));
        assert_eq!(
            Some(json!({
                "target": "haiwai comi",
                "command": "du -xhd1 /"
            })),
            granted_arguments
                .lock()
                .expect("granted arguments lock")
                .clone()
        );
    }

    #[gpui::test]
    fn gpui_failed_acp_permission_delivery_revokes_public_mcp_grant(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let revoked = Arc::new(AtomicUsize::new(0));
        cx.update({
            let revoked = revoked.clone();
            move |cx| {
                crate::set_acp_permission_grant_provider(
                    cx,
                    move |_request, option, _public_mcp_provider| {
                        if !option.kind.starts_with("allow") {
                            return None;
                        }
                        let revoked = revoked.clone();
                        Some(crate::AcpPermissionGrant::new(move || {
                            revoked.fetch_add(1, Ordering::SeqCst);
                        }))
                    },
                );
            }
        });
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());
        drop(outcome_rx);

        view.update(cx, |view, cx| {
            let _ = view.start_acp_client_session(cx);
            view.receive_acp_permission(envelope, cx);
        });
        cx.run_until_parked();
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("ACP allow button should render in the message list");
        cx.simulate_click(allow.center(), Modifiers::default());
        cx.run_until_parked();

        assert_eq!(1, revoked.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn gpui_public_mcp_safety_confirmation_resolves_inside_message_flow(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let request_id = "public-mcp:approval-1";
        let (envelope, mut outcome_rx) =
            AcpPublicMcpApprovalEnvelope::new(AcpPublicMcpApprovalRequest {
                request_id: request_id.into(),
                tool_name: "terminal.exec".into(),
                summary: "Call Execute in terminal".into(),
                details: json!({
                    "requestArguments": {
                        "target": "haiwai comi",
                        "command": "du -xhd1 / 2>/dev/null | sort -h"
                    }
                }),
            });

        view.update(cx, |view, cx| {
            view.receive_public_mcp_approval(envelope, cx)
        });
        cx.run_until_parked();

        let data = view.read_with(cx, |view, _| {
            view.transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(TOOL_CONFIRM_CARD))
                .and_then(|message| ToolConfirmCardData::from_json(&message.content))
                .expect("Public MCP confirmation card")
        });
        assert!(data.input_json.contains("haiwai comi"));
        assert_eq!(
            t!(
                "AgentUi.public_mcp_safety_confirmation",
                summary = "Call Execute in terminal"
            ),
            data.question
        );

        cx.dispatch_action(ApproveToolCall {
            call_id: request_id.into(),
        });
        cx.run_until_parked();

        assert_eq!(
            AcpPublicMcpApprovalOutcome::Approved,
            outcome_rx.try_recv().expect("Public MCP approval response")
        );
        assert!(!view.read_with(cx, |view, _| {
            view.transcript.has_pending_tool_confirm(request_id)
        }));
    }

    #[gpui::test]
    fn gpui_public_mcp_safety_confirmation_can_be_rejected(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let request_id = "public-mcp:approval-reject";
        let (envelope, mut outcome_rx) =
            AcpPublicMcpApprovalEnvelope::new(AcpPublicMcpApprovalRequest {
                request_id: request_id.into(),
                tool_name: "terminal.exec".into(),
                summary: "Call Execute in terminal".into(),
                details: json!({
                    "requestArguments": {"command": "rm -rf /tmp/example"}
                }),
            });

        view.update(cx, |view, cx| {
            view.receive_public_mcp_approval(envelope, cx)
        });
        cx.dispatch_action(RejectToolCall {
            call_id: request_id.into(),
        });
        cx.run_until_parked();

        assert_eq!(
            AcpPublicMcpApprovalOutcome::Denied,
            outcome_rx
                .try_recv()
                .expect("Public MCP rejection response")
        );
        assert!(!view.read_with(cx, |view, _| {
            view.transcript.has_pending_tool_confirm(request_id)
        }));
    }

    #[gpui::test]
    fn gpui_acp_permission_card_uses_full_width_details_and_compact_actions(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                ACP_PERMISSION_CARD,
                AcpPermissionCardData {
                    request_id: "session:call-layout".into(),
                    session_id: "session".into(),
                    tool_call_id: "call-layout".into(),
                    tool_name: "ACP tool".into(),
                    summary: "ACP Agent 请求执行工具：ACP tool".into(),
                    details_json: r#"{
  "tool": "terminal.exec",
  "kind": "write",
  "scope": "session"
}"#
                    .into(),
                    options: vec![
                        AcpPermissionOptionData {
                            option_id: "allow".into(),
                            name: "Allow".into(),
                            kind: "allow_once".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "allow-session".into(),
                            name: "Allow for This Session".into(),
                            kind: "allow_for_session".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "allow-always".into(),
                            name: "Allow and Don't Ask Again".into(),
                            kind: "allow_always".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "decline".into(),
                            name: "Decline".into(),
                            kind: "reject_once".into(),
                        },
                    ],
                    status: "pending".into(),
                    selected_option_name: String::new(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let details = cx
            .debug_bounds("acp-permission-details")
            .expect("ACP details should render");
        let frame = cx
            .debug_bounds("agent-tool-json-frame")
            .expect("ACP details frame should render");
        let input = cx
            .debug_bounds("agent-tool-json-input-slot")
            .expect("ACP details input should render");
        for (name, bounds) in [("details", details), ("frame", frame), ("input", input)] {
            assert!(
                bounds.size.width > column.size.width * 0.75,
                "ACP {name} should use the available message width: column={column:?}, bounds={bounds:?}"
            );
        }

        let actions = cx
            .debug_bounds("acp-permission-actions")
            .expect("ACP actions should render");
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("allow button should render");
        let reject = cx
            .debug_bounds("acp-permission-reject_once")
            .expect("reject button should render");
        let more = cx
            .debug_bounds("acp-permission-more-options")
            .expect("more-options trigger should render");
        assert_eq!(allow.origin.y, reject.origin.y);
        assert_eq!(allow.origin.y, more.origin.y);
        assert!(actions.size.height <= allow.size.height + px(4.0));
    }

    #[gpui::test]
    fn gpui_pending_elicitation_renders_and_submits_collected_answer(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) =
            AcpElicitationEnvelope::new(test_acp_elicitation_request());

        view.update(cx, |view, cx| {
            let _ = view.start_acp_client_session(cx);
            view.receive_acp_elicitation(envelope, cx);
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("agent-input-elicitation").is_some(),
            "agent 提问面板应渲染在输入框上方"
        );

        // 必填项空着时提交只应给出提示，不能把半截答案回给 agent。
        let submit = cx
            .debug_bounds("elicitation-submit")
            .expect("submit button");
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(outcome_rx.try_recv().is_err(), "必填项没填就不该提交");

        let prod = cx
            .debug_bounds("elicitation-radio-env-prod")
            .expect("env option");
        cx.simulate_click(prod.center(), Modifiers::default());
        cx.run_until_parked();
        let submit = cx
            .debug_bounds("elicitation-submit")
            .expect("submit button");
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();

        let AcpElicitationOutcome::Accept(content) = outcome_rx.try_recv().expect("answer") else {
            panic!("expected accept outcome");
        };
        assert_eq!(Some(&json!("prod")), content.get("env"));
        // 布尔字段没动过也要给值，否则 agent 收到的对象缺字段。
        assert_eq!(Some(&json!(false)), content.get("confirm"));
        assert!(
            cx.debug_bounds("agent-input-elicitation").is_none(),
            "答完应收起面板"
        );
    }

    #[gpui::test]
    fn gpui_declining_elicitation_reports_decline_and_closes_the_panel(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) =
            AcpElicitationEnvelope::new(test_acp_elicitation_request());

        view.update(cx, |view, cx| {
            view.receive_acp_elicitation(envelope, cx);
        });
        cx.run_until_parked();
        let decline = cx
            .debug_bounds("elicitation-decline")
            .expect("decline button");
        cx.simulate_click(decline.center(), Modifiers::default());
        cx.run_until_parked();

        assert_eq!(
            AcpElicitationOutcome::Decline,
            outcome_rx.try_recv().expect("decline outcome")
        );
        assert!(cx.debug_bounds("agent-input-elicitation").is_none());
    }

    #[gpui::test]
    fn gpui_resetting_acp_connection_cancels_pending_elicitation(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) =
            AcpElicitationEnvelope::new(test_acp_elicitation_request());

        view.update(cx, |view, cx| {
            view.receive_acp_elicitation(envelope, cx);
            view.reset_acp_client_session(cx);
        });

        assert_eq!(
            AcpElicitationOutcome::Cancel,
            outcome_rx.try_recv().expect("cancelled elicitation")
        );
    }

    #[gpui::test]
    fn gpui_resetting_acp_connection_cancels_pending_permission_card(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| {
            view.receive_acp_permission(envelope, cx);
            view.reset_acp_client_session(cx);
        });

        assert_eq!(
            AcpPermissionOutcome::Cancelled,
            outcome_rx.try_recv().expect("cancelled ACP permission")
        );
        let data = view.read_with(cx, |view, _| {
            let message = view
                .transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .expect("ACP permission card");
            AcpPermissionCardData::from_json(&message.content).expect("card data")
        });
        assert_eq!("cancelled", data.status);
        assert!(data.selected_option_name.is_empty());
    }

    #[gpui::test]
    fn gpui_tool_approval_action_dispatch_submits_approval(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        cx.dispatch_action(ApproveToolCall {
            call_id: "c_write".into(),
        });
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_tool_approval_button_click_submits_approval(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render");
        cx.simulate_click(approve.center(), Modifiers::default());
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_tool_approval_button_click_submits_after_scrolling(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config =
            AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]).sidebar_mode(true);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            for index in 0..16 {
                view.transcript.push_system(format!(
                    "滚动前置消息 {index}: 用于让确认卡进入可滚动区域。"
                ));
            }
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        let approve_before_scroll = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render before scrolling");
        cx.simulate_event(ScrollWheelEvent {
            position: approve_before_scroll.center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-280.0))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();

        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render after scrolling");
        cx.simulate_click(approve.center(), Modifiers::default());
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_system_instruction_is_sent_to_runtime_prompt(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.set_system_instruction(Some("始终用 DBA 视角回答。".into()), cx);
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        assert_eq!(1, requests.len());
        assert!(
            requests[0].messages[0]
                .content_as_text()
                .contains("始终用 DBA 视角回答。")
        );
    }

    #[gpui::test]
    fn gpui_custom_system_prompt_from_settings_seeds_new_view(cx: &mut TestAppContext) {
        init_test_ui(cx);
        cx.update(|cx| {
            let mut settings = AppSettings::default();
            settings.ai_chat.custom_system_prompt = "  始终用 DBA 视角回答。\n".into();
            cx.set_global(settings);
        });
        let model = Arc::new(MockModelClient::new([ModelResponse::text("好的。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        assert!(
            requests[0].messages[0]
                .content_as_text()
                .contains("始终用 DBA 视角回答。")
        );
    }

    #[gpui::test]
    fn gpui_settings_prompt_filled_after_empty_view_applies_to_new_session(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
        });
        let model = Arc::new(MockModelClient::new([ModelResponse::text("好的。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        // 视图打开时设置为空；随后填写设置，再新建会话应跟随设置刷新。
        view.update(cx, |view, cx| {
            let mut settings = AppSettings::current(cx);
            settings.ai_chat.custom_system_prompt = "空白后填写的人设。".into();
            cx.set_global(settings);
            view.start_fresh_session(cx);
        });
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        assert!(
            requests[0].messages[0]
                .content_as_text()
                .contains("空白后填写的人设。"),
            "设置为空时打开视图，之后填写设置，新会话仍应跟随设置"
        );
    }

    #[gpui::test]
    fn gpui_external_system_instruction_wins_over_settings_seed(cx: &mut TestAppContext) {
        init_test_ui(cx);
        cx.update(|cx| {
            let mut settings = AppSettings::default();
            settings.ai_chat.custom_system_prompt = "设置里的人设。".into();
            cx.set_global(settings);
        });
        let model = Arc::new(MockModelClient::new([ModelResponse::text("好的。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.set_system_instruction(Some("外部显式指令。".into()), cx);
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        let prompt = requests[0].messages[0].content_as_text();
        assert!(prompt.contains("外部显式指令。"));
        assert!(!prompt.contains("设置里的人设。"));
    }

    #[gpui::test]
    fn gpui_session_switch_restores_snapshot_instruction_for_local_sessions(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime.clone(), ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let first_id = view.read_with(cx, |view, _| view.session_id.clone());
        view.update(cx, |view, cx| {
            // 第一个会话带外部显式指令（模拟旧版行为），随后新建第二个会话。
            view.set_system_instruction(Some("旧会话指令。".into()), cx);
            view.start_fresh_session(cx);
        });
        let second_id = view.read_with(cx, |view, _| view.session_id.clone());
        assert_ne!(first_id, second_id);

        // 切回第一个会话：快照值优先，且外部指令不被全局设置覆盖。
        view.update(cx, |view, cx| {
            view.switch_session(&first_id.to_string(), cx);
        });
        view.read_with(cx, |view, _| {
            assert!(!view.system_instruction_from_settings);
        });
        let session = runtime.session(&first_id).expect("session should exist");
        assert_eq!(
            session.system_instruction().as_deref(),
            Some("旧会话指令。")
        );
    }

    #[gpui::test]
    fn gpui_system_instruction_survives_new_local_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime.clone(), ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let session_id = view.update(cx, |view, cx| {
            view.set_system_instruction(Some("只输出 SQL 审计建议。".into()), cx);
            view.start_fresh_session(cx);
            view.session_id.clone()
        });

        let session = runtime.session(&session_id).expect("session should exist");
        assert_eq!(
            session.system_instruction().as_deref(),
            Some("只输出 SQL 审计建议。")
        );
    }

    #[gpui::test]
    fn gpui_system_instruction_survives_model_switch(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let runtimes = Arc::new(std::sync::Mutex::new(Vec::<Arc<Runtime>>::new()));
        let factory_runtimes = runtimes.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |option| {
            let runtime = test_runtime(option.model.as_ref());
            factory_runtimes.lock().unwrap().push(runtime.clone());
            Ok(runtime)
        });
        let initial_runtime = test_runtime("gpt-a");
        let config = AgentChatViewConfig::new(initial_runtime, ResourceContext::new(), vec![])
            .with_models(
                vec![first, second],
                Some(SharedString::from("openai:gpt-a")),
                factory,
            );

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.set_system_instruction(Some("只输出 SQL 审计建议。".into()), cx);
            view.select_model("ollama:qwen", "ollama", "qwen3:14b", cx);
        });

        let runtime = runtimes.lock().unwrap().last().cloned().unwrap();
        let session_id = view.read_with(cx, |view, _| view.session_id.clone());
        let session = runtime.session(&session_id).expect("session should exist");
        assert_eq!(
            session.system_instruction().as_deref(),
            Some("只输出 SQL 审计建议。")
        );
    }

    /// 切换模型只换模型，不换会话：同一个 session id、同一段转录，历史跟着走。
    #[gpui::test]
    fn gpui_model_switch_keeps_the_same_conversation(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let runtimes = Arc::new(std::sync::Mutex::new(Vec::<Arc<Runtime>>::new()));
        let factory_runtimes = runtimes.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |option| {
            let runtime = test_runtime(option.model.as_ref());
            factory_runtimes.lock().unwrap().push(runtime.clone());
            Ok(runtime)
        });
        let config =
            AgentChatViewConfig::new(test_runtime("gpt-a"), ResourceContext::new(), vec![])
                .with_models(vec![first.clone(), second], Some(first.id.clone()), factory);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let original_session = view.read_with(cx, |view, _| view.session_id.clone());
        view.update(cx, |view, cx| {
            // 会话里已有对话，屏幕上已有转录。
            view.runtime
                .session(&view.session_id)
                .expect("current session should exist")
                .record_user_input("列出所有表");
            view.transcript.push_system("已在屏幕上的内容");
            view.select_model("ollama:qwen", "ollama", "qwen3:14b", cx);
        });

        view.read_with(cx, |view, _| {
            assert_eq!(view.session_id, original_session);
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content == "已在屏幕上的内容"),
                "切模型不该清掉屏幕上的对话"
            );
        });
        let runtime = runtimes
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("factory should have built the new runtime");
        view.read_with(cx, |view, _| {
            assert!(Arc::ptr_eq(&view.runtime, &runtime));
        });
        let session = runtime
            .session(&original_session)
            .expect("session should be carried into the new runtime");
        assert_eq!(
            session.history_snapshot().len(),
            1,
            "历史应随会话延续到新模型"
        );
    }

    #[gpui::test]
    fn gpui_model_switch_failure_preserves_current_state(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let initial_runtime = test_runtime("gpt-a");
        let expected_runtime = initial_runtime.clone();
        let factory: AgentRuntimeFactory =
            Arc::new(|_| Err(anyhow::anyhow!("duplicate agent tool names: load_skill")));
        let config = AgentChatViewConfig::new(initial_runtime, ResourceContext::new(), vec![])
            .with_models(vec![first.clone(), second], Some(first.id.clone()), factory);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let original_session = view.read_with(cx, |view, _| view.session_id.clone());
        view.update(cx, |view, cx| {
            view.transcript.push_system("existing transcript message");
            view.select_model("ollama:qwen", "ollama", "qwen3:14b", cx);
        });

        view.read_with(cx, |view, _| {
            assert!(Arc::ptr_eq(&view.runtime, &expected_runtime));
            assert_eq!(view.session_id, original_session);
            assert_eq!(
                view.selected_model
                    .as_ref()
                    .map(|option| option.id.as_ref()),
                Some("openai:gpt-a")
            );
            assert!(
                view.transcript
                    .messages
                    .iter()
                    .any(|message| message.content == "existing transcript message")
            );
            assert!(view.transcript.messages.iter().any(|message| {
                message
                    .content
                    .contains("duplicate agent tool names: load_skill")
            }));
        });
    }

    #[gpui::test]
    fn gpui_submit_agent_recovers_from_pseudo_tool_call(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call("c_bad", "tool", "db.schema")),
            ModelResponse::tool_call(function_tool_call(
                "c_plan",
                "update_plan",
                json!({
                    "plan": [
                        {"step": "创建计划清单", "status": "completed"},
                        {"step": "给出总结", "status": "in_progress"}
                    ]
                })
                .to_string(),
            )),
            ModelResponse::text("已创建计划清单。"),
        ]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime.clone(), ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let session_id = view.read_with(cx, |view, _| view.session_id.clone());
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "先创建一个包含几个步骤的计划清单。".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 3);

        assert_eq!(3, model.request_count());
        let session = runtime.session(&session_id).expect("session should exist");
        let history = session.history_snapshot();
        assert!(history.items().iter().any(|item| {
            matches!(
                item,
                agent_runtime::HistoryItem::Observation(observation)
                    if !observation.success && observation.tool_name.as_str() == "tool"
            )
        }));
        assert!(
            session.current_plan().is_some(),
            "伪工具调用纠偏后应继续完成 update_plan"
        );
    }

    #[test]
    fn config_defaults_to_full_view_and_builder_enables_sidebar() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        assert!(!config.sidebar_mode, "默认应为全宽视图");
        assert!(
            config.sidebar_mode(true).sidebar_mode,
            "builder 应开启侧边栏视图"
        );
    }

    #[test]
    fn sidebar_header_visibility_can_be_disabled_for_framed_hosts() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        assert!(config.show_sidebar_header);
        assert!(!config.show_sidebar_frame_controls);

        let embedded = config.show_sidebar_header(false);
        assert!(!embedded.show_sidebar_header);
    }

    #[test]
    fn sidebar_mode_header_actions_include_close() {
        assert_eq!(
            vec!["new", "history", "close"],
            sidebar_mode_header_action_ids(false)
        );
    }

    #[test]
    fn sidebar_mode_header_actions_can_include_frame_options() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true)
            .show_sidebar_frame_controls(true, SidebarPlacement::Bottom);

        assert!(config.show_sidebar_frame_controls);
        assert_eq!(SidebarPlacement::Bottom, config.sidebar_frame_placement);
        assert_eq!(
            vec!["new", "history", "frame-options", "close"],
            sidebar_mode_header_action_ids(config.show_sidebar_frame_controls)
        );
    }

    #[test]
    fn agent_history_labels_use_task_language() {
        assert_eq!(t!("AgentUi.history_tasks"), agent_history_title(false));
        assert_eq!(t!("AgentUi.archived_tasks"), agent_history_title(true));
        assert_eq!(t!("AgentUi.current_agent_task"), current_agent_task_title());
    }

    #[test]
    fn workbench_sidebar_merges_current_and_background_running_sessions() {
        let persisted = vec![SessionSummary::new("saved", "已保存任务", 10)];
        let live = vec![
            SessionSummary::new("current", "当前任务", 30),
            SessionSummary::new("running", "后台任务", 20),
        ];
        let running = HashSet::from(["running".to_string()]);

        let merged = merge_live_session_summaries(persisted, &live, "current", &running, false);

        assert_eq!(
            vec!["current", "running", "saved"],
            merged
                .iter()
                .map(|summary| summary.id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn selecting_session_does_not_move_it_to_front() {
        let persisted = vec![
            SessionSummary::new("newest", "最新任务", 30),
            SessionSummary::new("middle", "中间任务", 20),
            SessionSummary::new("selected", "选中任务", 10),
        ];
        let live = vec![SessionSummary::new("selected", "选中任务", 10)];

        let merged =
            merge_live_session_summaries(persisted, &live, "selected", &HashSet::new(), false);

        assert_eq!(
            vec!["newest", "middle", "selected"],
            merged
                .iter()
                .map(|summary| summary.id.as_str())
                .collect::<Vec<_>>(),
            "selecting a conversation should only change its selected state, not its list position"
        );
    }

    /// 点开会话只是选中它，不能顺手改写它的工作区归属。
    ///
    /// 实时摘要（live）只负责「名字 / 时间」这类还在内存里变动的字段；工作区
    /// 归属的真源是落盘快照。实时摘要里这条如果缺归属（例如进程启动时建的
    /// 初始会话从没进过 `session_roots`），整体覆盖会把侧栏那一行从它的工作区
    /// 分组里打到「未分组」——用户看到的就是「点一下它就换组了」。
    #[test]
    fn selected_live_summary_must_not_erase_workspace_root() {
        let persisted = vec![
            SessionSummary::new("selected", "选中任务", 10)
                .with_workspace_root(Some("/w/project".into())),
        ];
        let live = vec![SessionSummary::new("selected", "选中任务", 10)];

        let merged =
            merge_live_session_summaries(persisted, &live, "selected", &HashSet::new(), false);

        assert_eq!(
            Some("/w/project"),
            merged[0].workspace_root.as_deref(),
            "选中会话不应把它从所属工作区分组里打出去"
        );
    }

    /// 反过来也一样：未分组的会话不能被实时摘要凭空塞进某个分组。
    #[test]
    fn selected_live_summary_must_not_invent_workspace_root() {
        let persisted = vec![SessionSummary::new("legacy", "旧会话", 10)];
        let live = vec![
            SessionSummary::new("legacy", "旧会话", 10)
                .with_workspace_root(Some("/w/project".into())),
        ];

        let merged =
            merge_live_session_summaries(persisted, &live, "legacy", &HashSet::new(), false);

        assert_eq!(
            None,
            merged[0].workspace_root.as_deref(),
            "未分组的会话不应因为被选中就挂到某个工作区下"
        );
    }

    #[test]
    fn archived_sidebar_does_not_mix_in_live_workbench_tasks() {
        let archived = vec![SessionSummary::new("archived", "归档任务", 10)];
        let live = vec![SessionSummary::new("current", "当前任务", 30)];
        let running = HashSet::from(["current".to_string()]);

        let merged = merge_live_session_summaries(archived, &live, "current", &running, true);

        assert_eq!(
            vec!["archived"],
            merged.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn local_workbench_can_switch_away_from_a_running_session() {
        // 后台轮次方案：两侧切换都不再强停；ACP 的新建/切换也不取消在飞轮次。
        assert!(!should_stop_task_before_session_switch(Backend::Local));
        assert!(!should_stop_task_before_session_switch(Backend::Acp));
    }

    /// 启动时那张初始会话也要记住自己的工作区。
    ///
    /// `session_roots` 是侧栏归属在内存里的依据。漏了这条会话，它第一次落盘
    /// 之前实时摘要就报不出归属，侧栏先把它摆进「未分组」，落盘后才跳回分组。
    #[gpui::test]
    fn the_initial_session_knows_its_workspace(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .with_workspace_root(std::path::PathBuf::from("/tmp/navop-test-ws-a"));
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, _cx| {
            assert_eq!(
                view.session_roots
                    .get(&view.current_session)
                    .map(String::as_str),
                Some("/tmp/navop-test-ws-a"),
                "初始会话必须记住它的工作区，否则它在侧栏里会先显示成「未分组」"
            );
        });
    }

    /// 一条只带会话内容的存储后端；用来造「快照里没有工作区归属」的旧会话。
    fn test_session_storage() -> one_core::storage::StorageManager {
        use one_core::llm::chat_history::{
            AgentSessionRepository, MessageRepository, SessionRepository,
        };
        use one_core::storage::StorageManager;
        use one_core::storage::connection::SqliteConnection;
        use one_core::storage::migration::run_migrations;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "navop-ai-chat-view-sessions-{}-{unique}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(run_migrations)
            .expect("run migrations");

        let storage = StorageManager::new_with_connection(conn.clone());
        storage.register(AgentSessionRepository::new(conn.clone()));
        storage.register(SessionRepository::new(conn.clone()));
        storage.register(MessageRepository::new(conn));
        storage
    }

    /// 点开一条「没有归属」的旧会话，不能顺手把它钉到当前工作区。
    ///
    /// 侧栏把它摆在「未分组」，就是因为快照里没有归属。归属的定格只发生在
    /// **真正落盘**那一刻（见 `persist_session` 里的兜底）；只是点开看一眼
    /// 就改写它，用户看到的就是「无分组的自己跑到分组下面去了」。
    #[gpui::test]
    fn opening_a_legacy_session_without_a_workspace_does_not_assign_one(cx: &mut TestAppContext) {
        use agent_runtime::{HistoryItem, SessionId, SessionSnapshot};
        use one_core::llm::chat_history::AgentSessionRepository;
        use one_core::storage::GlobalStorageState;

        init_test_ui(cx);

        let legacy_uid = "legacy-without-workspace";
        let snapshot = SessionSnapshot {
            id: SessionId::from_string(legacy_uid),
            resources: ResourceContext::new(),
            history: vec![
                HistoryItem::User {
                    text: "升级前的老会话".into(),
                    images: Vec::new(),
                },
                HistoryItem::Assistant("老回答".into()),
            ],
            plan: None,
            system_instruction: None,
            skills: agent_runtime::SkillContext::new(),
            workspace_root: None,
            draft: None,
            context_tokens: None,
            acp: None,
        };
        let snapshot_json = serde_json::to_string(&snapshot).expect("序列化快照");

        let storage = test_session_storage();
        storage
            .get::<AgentSessionRepository>()
            .expect("会话仓储")
            .save_snapshot(legacy_uid, "升级前的老会话", &snapshot_json)
            .expect("写入一条无归属旧会话");
        cx.update(|cx| cx.set_global(GlobalStorageState { storage }));

        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .with_workspace_root(std::path::PathBuf::from("/tmp/navop-test-ws-a"));
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update_in(cx, |view, _window, cx| {
            view.switch_session(legacy_uid, cx);

            assert_eq!(
                legacy_uid, view.current_session,
                "前置：确实切到了那条旧会话（快照读回来了）"
            );
            assert_eq!(
                None,
                view.session_roots.get(legacy_uid).map(String::as_str),
                "点开旧会话只是看一眼，不该凭空给它定一个工作区归属"
            );
        });
    }

    #[gpui::test]
    fn new_local_session_keeps_previous_running_task_in_sidebar(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            let previous_session = view.current_session.clone();
            view.set_running(true, cx);
            view.new_session(cx);

            assert_ne!(previous_session, view.current_session);
            assert!(view.running_sessions.contains(&previous_session));
            assert!(!view.is_running);
            assert_eq!(
                Some(view.current_session.as_str()),
                view.sessions.first().map(|session| session.id.as_str())
            );
            assert!(
                view.sessions
                    .iter()
                    .any(|session| session.id == previous_session)
            );

            view.switch_session(&previous_session, cx);
            assert_eq!(previous_session, view.current_session);
            assert!(view.is_running);
            assert!(view.running_sessions.contains(&previous_session));
        });
    }

    #[gpui::test]
    fn running_session_shows_loading_spinner_in_sidebar(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let running_session = view.update(cx, |view, cx| {
            let running_session = view.current_session.clone();
            view.set_running(true, cx);
            running_session
        });
        let cx: &mut VisualTestContext = cx;
        let spinner_id: &'static str =
            Box::leak(format!("agent-session-running-spinner-{running_session}").into_boxed_str());

        cx.debug_bounds(spinner_id)
            .expect("running conversation should show an animated loading spinner in the sidebar");
    }

    #[gpui::test]
    fn all_parallel_running_sessions_show_loading_in_sidebar(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (background_session, current_session) = view.update(cx, |view, cx| {
            let background_session = view.current_session.clone();
            view.set_running(true, cx);
            view.new_session(cx);
            let current_session = view.current_session.clone();
            view.set_running(true, cx);
            (background_session, current_session)
        });
        let cx: &mut VisualTestContext = cx;
        let background_spinner_id: &'static str = Box::leak(
            format!("agent-session-running-spinner-{background_session}").into_boxed_str(),
        );
        let current_spinner_id: &'static str =
            Box::leak(format!("agent-session-running-spinner-{current_session}").into_boxed_str());

        cx.debug_bounds(background_spinner_id)
            .expect("background running conversation should show loading in the sidebar");
        cx.debug_bounds(current_spinner_id)
            .expect("current running conversation should show loading in the sidebar");
    }

    #[gpui::test]
    fn running_chat_shows_activity_indicator_in_message_area(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.set_running(true, cx);
        });
        let cx: &mut VisualTestContext = cx;

        cx.debug_bounds("ai-chat-activity")
            .expect("running conversation should show the activity strip in the message area");
    }

    #[gpui::test]
    fn idle_chat_hides_activity_indicator(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, _| {
            assert!(!view.is_running);
        });
        let cx: &mut VisualTestContext = cx;

        assert!(
            cx.debug_bounds("ai-chat-activity").is_none(),
            "idle conversation must not show the running activity strip"
        );
    }

    #[test]
    fn background_running_session_uses_readable_foreground_color() {
        let foreground = gpui::rgb(0xf8fafc).into();
        let selected_foreground = gpui::rgb(0xe2e8f0).into();
        let style = SessionRowStyle {
            foreground,
            muted_foreground: gpui::rgb(0x64748b).into(),
            selected_background: gpui::rgb(0x1e293b).into(),
            selected_foreground,
            hover_background: gpui::rgb(0x0f172a).into(),
        };

        assert_eq!(
            foreground,
            running_session_indicator_color(false, style),
            "background running tasks must remain clearly visible on the sidebar background"
        );
        assert_eq!(
            selected_foreground,
            running_session_indicator_color(true, style),
            "the selected running task should use the selected row foreground"
        );
    }

    #[test]
    fn parallel_running_sessions_use_independent_animation_ids() {
        assert_ne!(
            running_session_animation_id("session-a"),
            running_session_animation_id("session-b")
        );
    }

    #[gpui::test]
    fn sidebar_mode_input_is_edge_to_edge(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (_, cx) = cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let area = cx
            .debug_bounds("agent-input-area")
            .expect("input area should render");
        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");

        assert_eq!(
            area.size.width, input.size.width,
            "sidebar input should fill the bottom area: area={area:?}, input={input:?}"
        );
        assert_eq!(
            area.origin.x, input.origin.x,
            "sidebar input should not be inset: area={area:?}, input={input:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_user_message_row_fills_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::user("帮我看看内存占用"));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let scroll = cx
            .debug_bounds("ai-chat-messages-scroll")
            .expect("message scroll area should render");
        let user_row = cx
            .debug_bounds("ai-chat-user-row")
            .expect("user row should render");
        let user_bubble = cx
            .debug_bounds("ai-chat-user-bubble")
            .expect("user bubble should render");

        let expected_column_width = scroll.size.width - px(32.0);
        assert_eq!(
            expected_column_width, column.size.width,
            "sidebar message column should fill the padded scroll area: scroll={scroll:?}, column={column:?}"
        );
        assert_eq!(
            column.size.width, user_row.size.width,
            "user message row should fill the message column: column={column:?}, row={user_row:?}"
        );
        assert_eq!(
            column.origin.x, user_row.origin.x,
            "user message row should not drift horizontally: column={column:?}, row={user_row:?}"
        );
        assert!(
            user_bubble.size.width < px(240.0),
            "short user message bubble should fit its content instead of filling the row: row={user_row:?}, bubble={user_bubble:?}"
        );
        assert_eq!(
            user_row.origin.x + user_row.size.width,
            user_bubble.origin.x + user_bubble.size.width,
            "user message bubble should align to the right edge: row={user_row:?}, bubble={user_bubble:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_user_message_uses_plain_text_and_readable_width(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::user("**保持**"));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let bubble = cx
            .debug_bounds("ai-chat-user-bubble")
            .expect("user bubble should render");
        let plain_text = cx
            .debug_bounds("ai-chat-user-plain-text")
            .expect("user content should use the plain-text renderer");

        assert!(
            bubble.size.width >= px(128.0),
            "short user messages should have a readable minimum width: bubble={bubble:?}"
        );
        assert_eq!(
            bubble.size.width - px(26.0),
            plain_text.size.width,
            "plain user text should use the bubble width inside its padding and border: bubble={bubble:?}, text={plain_text:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_long_user_message_bubble_uses_available_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::user(
                "帮我看看这台服务器当前还有多少内存，并且顺便判断一下是否需要扩容或者清理缓存",
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let bubble = cx
            .debug_bounds("ai-chat-user-bubble")
            .expect("user bubble should render");

        assert!(
            bubble.size.width > column.size.width * 0.7,
            "long user bubble should use the available sidebar column width: column={column:?}, bubble={bubble:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_fills_fixed_host_frame(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::user("帮我检查终端侧边栏布局"));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let slot = cx
            .debug_bounds("fixed-sidebar-content-slot")
            .expect("fixed sidebar content slot should render");
        let root = cx
            .debug_bounds("agent-sidebar-root")
            .expect("sidebar root should render");
        let stack = cx
            .debug_bounds("agent-sidebar-stack")
            .expect("sidebar stack should render");
        let messages = cx
            .debug_bounds("ai-chat-messages")
            .expect("messages area should render");
        let input_area = cx
            .debug_bounds("agent-input-area")
            .expect("input area should render");
        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");

        assert_eq!(slot.origin.x, root.origin.x);
        assert_eq!(slot.size.width, root.size.width);
        assert_eq!(root.origin.x, stack.origin.x);
        assert_eq!(root.size.width, stack.size.width);
        assert_eq!(root.origin.x, messages.origin.x);
        assert_eq!(root.size.width, messages.size.width);
        assert_eq!(root.origin.x, input_area.origin.x);
        assert_eq!(root.size.width, input_area.size.width);
        assert_eq!(input_area.origin.x, input.origin.x);
        assert_eq!(input_area.size.width, input.size.width);
        assert!(
            input_area.size.height > px(0.0),
            "sidebar input area must keep a visible height: area={input_area:?}, input={input:?}"
        );
        assert!(
            input.size.height > px(0.0),
            "sidebar input root must keep a visible height: area={input_area:?}, input={input:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_keeps_input_visible_after_long_agent_reply(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::short);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::assistant(
                    std::iter::repeat_n(
                        "这是 Agent 返回的一段较长回复，用于验证消息内容只能在消息区域内部滚动。",
                        40,
                    )
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let slot = cx
            .debug_bounds("fixed-sidebar-content-slot")
            .expect("fixed sidebar content slot should render");
        let messages = cx
            .debug_bounds("ai-chat-messages")
            .expect("messages area should render");
        let input_area = cx
            .debug_bounds("agent-input-area")
            .expect("input area should remain rendered");
        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should remain rendered");

        assert!(
            input_area.size.height > px(0.0),
            "input area must not collapse in a short sidebar: slot={slot:?}, input={input_area:?}"
        );
        assert!(
            input.size.height > px(0.0),
            "input root must not collapse in a short sidebar: slot={slot:?}, input={input:?}"
        );
        assert!(
            messages.bottom() <= input_area.origin.y,
            "messages must end before the input area: messages={messages:?}, input={input_area:?}"
        );
        assert!(
            input_area.bottom() <= slot.bottom(),
            "input area must stay inside the sidebar viewport: slot={slot:?}, input={input_area:?}"
        );
        assert!(
            input.origin.y < slot.bottom(),
            "the scrollable input root must start inside the sidebar viewport: slot={slot:?}, input={input:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_tool_card_fills_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CARD,
                ToolCardData {
                    call_id: "call-layout".to_string(),
                    tool_name: "terminal.exec".to_string(),
                    action: agent_runtime::ToolAction::Execute,
                    target_id: Some("ssh-prod-with-a-very-long-target-id".to_string()),
                    target_label: Some("生产终端节点-很长的展示名称".to_string()),
                    input_summary: "ps aux | sort -nrk 3,3 | head -20".to_string(),
                    input_json: r#"{"command":"ps aux | sort -nrk 3,3 | head -20"}"#.to_string(),
                    running: true,
                    success: None,
                    summary: String::new(),
                    data_text: String::new(),
                    file_changes: Vec::new(),
                    duration_ms: None,
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let card = cx
            .debug_bounds("agent-tool-card")
            .expect("tool card should render");

        // 工具行是活动块的子行,缩进一级(块头占最左边);除了这一级缩进,它仍
        // 该铺满剩余宽度。
        assert_eq!(column.origin.x + px(16.0), card.origin.x);
        assert_eq!(
            column.size.width - px(16.0),
            card.size.width,
            "tool card should fill the rest of the sidebar message column: column={column:?}, card={card:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_tool_confirm_actions_align_to_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CONFIRM_CARD,
                ToolConfirmCardData {
                    call_id: "call-confirm-layout".to_string(),
                    tool_name: "terminal_exec".to_string(),
                    items: Vec::new(),
                    input_summary: "free -h".to_string(),
                    input_json: r#"{"command":"free -h"}"#.to_string(),
                    question: "确认执行工具 terminal_exec 吗？".to_string(),
                    status: "pending".to_string(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render");

        assert!(
            approve.right() > column.right() - px(96.0),
            "approval button should align near the message column right edge: column={column:?}, approve={approve:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_tool_confirm_json_block_uses_available_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CONFIRM_CARD,
                ToolConfirmCardData {
                    call_id: "call-confirm-json-layout".to_string(),
                    tool_name: "terminal_exec".to_string(),
                    items: Vec::new(),
                    input_summary: "free -h".to_string(),
                    input_json: r#"{
  "target": "ssh-prod",
  "command": "free -h",
  "subprocess": true
}"#
                    .to_string(),
                    question: "确认执行工具 terminal_exec 吗？".to_string(),
                    status: "pending".to_string(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let json = cx
            .debug_bounds("agent-tool-json-block")
            .expect("tool json block should render");
        let frame = cx
            .debug_bounds("agent-tool-json-frame")
            .expect("tool json frame should render");
        let input = cx
            .debug_bounds("agent-tool-json-input-slot")
            .expect("tool json input slot should render");

        assert!(
            json.size.width > column.size.width * 0.75,
            "tool confirm json block should use the available sidebar column width: column={column:?}, json={json:?}"
        );
        assert!(
            frame.size.width > column.size.width * 0.75,
            "tool confirm json frame should use the available sidebar column width: column={column:?}, frame={frame:?}"
        );
        assert!(frame.right() <= json.right());
        assert!(
            input.size.width > column.size.width * 0.75,
            "tool confirm json input should use the available sidebar column width: column={column:?}, input={input:?}"
        );
    }

    #[test]
    fn runtime_binding_switches_runtime_from_structured_model_option() {
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = calls.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |option| {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            Ok(test_runtime(option.model.as_ref()))
        });

        let resources = ResourceContext::new();
        let initial_runtime = test_runtime(first.model.as_ref());
        let mut binding = RuntimeBinding::new(
            initial_runtime,
            resources.clone(),
            Some(first),
            Some(factory),
        );
        let old_session = binding.session_id.clone();

        assert!(
            binding
                .switch_model(&second, &resources)
                .expect("runtime switch should succeed")
        );
        // 空会话也一起搬过去：切模型不该凭空多出一个 session id。
        assert_eq!(binding.session_id, old_session);
        assert!(binding.runtime.session(&old_session).is_some());
        assert_eq!(binding.runtime.services().model.model_name(), "qwen3:14b");
        assert_eq!(
            binding
                .selected_model
                .as_ref()
                .unwrap()
                .provider_id
                .as_ref(),
            "ollama"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 切模型必须带上当前会话：本地会话的历史只活在 Runtime 内存里，换 Runtime
    /// 若新开空会话，用户看到的就是"切完模型它不记得刚才聊过什么"。
    #[test]
    fn runtime_binding_switch_model_carries_the_current_session() {
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let factory: AgentRuntimeFactory =
            Arc::new(|option| Ok(test_runtime(option.model.as_ref())));
        let resources = ResourceContext::new();
        let mut binding = RuntimeBinding::new(
            test_runtime(first.model.as_ref()),
            resources.clone(),
            Some(first),
            Some(factory),
        );
        let session_id = binding.session_id.clone();
        binding
            .runtime
            .session(&session_id)
            .expect("initial session should exist")
            .record_user_input("查询当前连接数");

        assert!(
            binding
                .switch_model(&second, &resources)
                .expect("runtime switch should succeed")
        );

        assert_eq!(binding.session_id, session_id);
        assert_eq!(binding.runtime.services().model.model_name(), "qwen3:14b");
        let carried = binding
            .runtime
            .session(&session_id)
            .expect("session should be restored into the new runtime");
        assert_eq!(carried.id(), &session_id);
        assert_eq!(
            carried.history_snapshot().len(),
            1,
            "对话历史应随会话延续到新模型"
        );
    }

    #[test]
    fn runtime_binding_preserves_state_when_runtime_factory_fails() {
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let factory: AgentRuntimeFactory =
            Arc::new(|_| Err(anyhow::anyhow!("duplicate agent tool names: load_skill")));
        let resources = ResourceContext::new();
        let initial_runtime = test_runtime(first.model.as_ref());
        let expected_runtime = initial_runtime.clone();
        let mut binding = RuntimeBinding::new(
            initial_runtime,
            resources.clone(),
            Some(first.clone()),
            Some(factory),
        );
        let old_session = binding.session_id.clone();

        let error = binding
            .switch_model(&second, &resources)
            .expect_err("runtime factory failure should be propagated");

        assert_eq!(error.to_string(), "duplicate agent tool names: load_skill");
        assert!(Arc::ptr_eq(&binding.runtime, &expected_runtime));
        assert_eq!(binding.session_id, old_session);
        assert_eq!(binding.selected_model, Some(first));
    }

    #[test]
    fn provider_config_models_expand_to_structured_options() {
        let config = ProviderConfig {
            id: 7,
            name: "Local Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            model: "qwen3:14b".to_string(),
            models: vec!["qwen3:14b".to_string(), "llama3.1".to_string()],
            is_default: true,
            ..Default::default()
        };

        let specs = runtime_specs_from_provider_configs(vec![config], ToolRegistry::new())
            .expect("ollama provider config should build without network");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].option.provider_id.as_ref(), "7");
        assert_eq!(specs[0].option.provider_label.as_ref(), "Local Ollama");
        assert_eq!(specs[0].option.model.as_ref(), "qwen3:14b");
        assert_eq!(specs[1].option.model.as_ref(), "llama3.1");
        assert_eq!(
            selected_provider_model_id(&specs),
            Some(SharedString::from("provider:7:qwen3:14b"))
        );
    }

    #[test]
    fn provider_config_initial_runtime_uses_default_model() {
        let first = ProviderConfig {
            id: 7,
            name: "First".to_string(),
            provider_type: ProviderType::Ollama,
            model: "first-model".to_string(),
            is_default: false,
            ..Default::default()
        };
        let second = ProviderConfig {
            id: 8,
            name: "Default".to_string(),
            provider_type: ProviderType::Ollama,
            model: "default-model".to_string(),
            is_default: true,
            ..Default::default()
        };

        let config = AgentChatViewConfig::from_provider_configs(
            ResourceContext::new(),
            vec![],
            vec![first, second],
            ToolRegistry::new(),
        )
        .expect("provider configs should build");

        assert_eq!(
            config.selected_model_id,
            Some(SharedString::from("provider:8:default-model"))
        );
        assert_eq!(
            config.runtime.services().model.model_name(),
            "default-model"
        );
    }

    #[test]
    fn runtime_specs_factory_rejects_unknown_model_option() {
        let provider = ProviderConfig {
            id: 7,
            name: "Local Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            model: "qwen3:14b".to_string(),
            is_default: true,
            ..Default::default()
        };
        let config = AgentChatViewConfig::from_provider_configs(
            ResourceContext::new(),
            vec![],
            vec![provider],
            ToolRegistry::new(),
        )
        .expect("provider config should build");
        let factory = config
            .runtime_factory
            .expect("provider-backed config should expose a runtime factory");
        let unknown =
            ComposerModelOption::new("unknown:model", "unknown", "Unknown", "unknown-model");

        let error = factory(&unknown)
            .err()
            .expect("unknown model option must fail closed");

        assert_eq!(
            error.to_string(),
            "unknown agent model option: unknown:model"
        );
    }

    #[test]
    fn provider_config_uses_type_label_when_name_is_empty() {
        let config = ProviderConfig {
            id: 8,
            provider_type: ProviderType::Ollama,
            model: "mistral".to_string(),
            ..Default::default()
        };

        let specs = runtime_specs_from_provider_configs(vec![config], ToolRegistry::new())
            .expect("ollama provider config should build without network");

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].option.provider_label.as_ref(), "Ollama");
    }

    #[test]
    fn refreshed_models_keep_current_selection_when_it_still_exists() {
        let current = ComposerModelOption::new("p:old", "p", "Provider", "old");
        let added = ComposerModelOption::new("p:new", "p", "Provider", "new");
        let previous_id = current.id.clone();
        let default_id = added.id.clone();

        let (selected, retained) =
            refreshed_model_selection(Some(&previous_id), Some(&default_id), &[current, added]);

        assert_eq!(
            selected.as_ref().map(|model| model.id.as_ref()),
            Some("p:old")
        );
        assert!(retained.is_some());
    }

    #[test]
    fn refreshed_models_fall_back_when_current_selection_was_removed() {
        let fallback = ComposerModelOption::new("p:new", "p", "Provider", "new");
        let removed_id = SharedString::from("p:removed");
        let default_id = fallback.id.clone();

        let (selected, retained) =
            refreshed_model_selection(Some(&removed_id), Some(&default_id), &[fallback]);

        assert_eq!(
            selected.as_ref().map(|model| model.id.as_ref()),
            Some("p:new")
        );
        assert!(retained.is_none());
    }

    /// 视图级测试里标记「本地那段对话」的哨兵文本。
    const LOCAL_TRANSCRIPT_MARK: &str = "本地这段不能丢";

    /// 视图级测试里冒充「ACP agent 那段对话」的哨兵文本。
    const ACP_TRANSCRIPT_MARK: &str = "agent 那边说的话";

    fn test_runtime(model_name: &str) -> Arc<Runtime> {
        let model = Arc::new(NamedModelClient(model_name.to_string()));
        let tools = Arc::new(ToolRouter::new(ToolRegistry::new()));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn test_runtime_with_model(model: Arc<MockModelClient>) -> Arc<Runtime> {
        let tools = Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        ));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn test_runtime_with_model_and_write_tool(model: Arc<MockModelClient>) -> Arc<Runtime> {
        let tools = Arc::new(ToolRouter::new(
            ToolRegistry::new()
                .with_tool(Arc::new(EchoTool))
                .with_tool(Arc::new(WriteTool)),
        ));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn init_test_ui(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
    }

    fn run_gpui_until(cx: &mut VisualTestContext, condition: impl Fn() -> bool) {
        for _ in 0..20 {
            if condition() {
                return;
            }
            cx.run_until_parked();
        }
        assert!(condition(), "GPUI test condition was not reached");
    }

    // ===== 决策栏（DecisionDock）=====

    /// 决策栏与时间线卡片是**同一个决策**：点决策栏的按钮，走的就是内联卡片用的那个 action。
    #[gpui::test]
    fn gpui_decision_dock_mirrors_the_inline_confirm_card(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.tool_execution_mode = ToolExecutionMode::Manual;
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        // 决策栏出现，且与内联确认卡是同一条决策。
        cx.debug_bounds("ai-chat-decision-dock")
            .expect("decision dock should render while the tool awaits approval");
        let pending = view.read_with(cx, |view, _| view.transcript.pending_decisions());
        assert_eq!(1, pending.len());
        assert_eq!("c_write", pending[0].id);
        assert_eq!(crate::DecisionAuthority::LocalTool, pending[0].authority);

        let allow = cx
            .debug_bounds("decision-option-c_write-allow")
            .expect("dock allow button should render");
        let deny = cx
            .debug_bounds("decision-option-c_write-deny")
            .expect("dock deny button should render");
        assert_eq!(allow.origin.y, deny.origin.y, "options share one row");
        // 详情默认收起；展开是显式动作，不抢焦点。
        assert!(cx.debug_bounds("ai-chat-decision-details").is_none());

        // 真点决策栏的按钮 —— 与点内联卡片走同一个命令入口。
        cx.simulate_click(allow.center(), Modifiers::default());
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
        // 决策落地后决策栏整条消失，不留空条。
        assert!(cx.debug_bounds("ai-chat-decision-dock").is_none());
    }

    /// ACP 权限：决策栏只做来源标注与转发，落地仍是 `SelectAcpPermissionOption`，
    /// 且 provider 下发的 `option_id` 原样回传（不虚构、不本地化改写）。
    #[gpui::test]
    fn gpui_decision_dock_forwards_the_provider_option_id_for_acp(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| view.receive_acp_permission(envelope, cx));
        let cx: &mut VisualTestContext = cx;

        cx.debug_bounds("ai-chat-decision-dock")
            .expect("decision dock should render for a pending ACP permission");

        // 内联卡片的选项 id 之一；决策栏必须把同一个 id 转发出去。
        let option = view.read_with(cx, |view, _| {
            view.transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .and_then(|message| AcpPermissionCardData::from_json(&message.content))
                .expect("ACP permission card")
                .options
                .first()
                .cloned()
                .expect("provider option")
        });
        cx.dispatch_action(SelectAcpPermissionOption {
            request_id: "session:call".into(),
            option_id: option.option_id.clone(),
        });
        cx.run_until_parked();

        assert!(matches!(
            outcome_rx.try_recv(),
            Ok(AcpPermissionOutcome::Selected { option_id }) if option_id == option.option_id
        ));
        assert!(cx.debug_bounds("ai-chat-decision-dock").is_none());
    }

    /// 详情展开是纯展示动作：显式点开才有内容，且不影响决策本身。
    #[gpui::test]
    fn gpui_decision_dock_details_expand_only_on_request(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CONFIRM_CARD,
                ToolConfirmCardData {
                    call_id: "call_details".into(),
                    tool_name: "fs.write".into(),
                    items: Vec::new(),
                    input_summary: "navop/.env".into(),
                    input_json: r#"{"path":".env","token":"••••"}"#.into(),
                    question: "需要确认".into(),
                    status: "pending".into(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        assert!(cx.debug_bounds("ai-chat-decision-details").is_none());
        view.update(cx, |view, cx| {
            view.decision_details = Some("call_details".into());
            cx.notify();
        });
        assert!(
            cx.debug_bounds("ai-chat-decision-details").is_some(),
            "explicitly opening details should render them"
        );
        // 展开详情不解算决策：卡片仍是 pending。
        assert!(view.read_with(cx, |view, _| {
            view.transcript.has_pending_tool_confirm("call_details")
        }));
    }

    /// Public MCP 与本地工具共用 `agent.confirm` 卡，但**授权域不合并**：
    /// 同一个按钮只作用于它自己那一域，落地路径也各走各的。
    #[gpui::test]
    fn gpui_decision_dock_keeps_public_mcp_in_its_own_authority(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let request_id = "public-mcp:approval-dock";
        let (envelope, mut outcome_rx) =
            AcpPublicMcpApprovalEnvelope::new(AcpPublicMcpApprovalRequest {
                request_id: request_id.into(),
                tool_name: "terminal.exec".into(),
                summary: "Call Execute in terminal".into(),
                details: json!({"requestArguments": {"command": "du -xhd1 /"}}),
            });
        view.update(cx, |view, cx| {
            view.receive_public_mcp_approval(envelope, cx)
        });
        cx.run_until_parked();

        let authority = view.read_with(cx, |view, _| {
            view.transcript
                .pending_decisions()
                .into_iter()
                .find(|decision| decision.id == request_id)
                .map(|decision| decision.authority)
        });
        assert_eq!(Some(crate::DecisionAuthority::PublicMcp), authority);

        cx.dispatch_action(ApproveToolCall {
            call_id: request_id.into(),
        });
        cx.run_until_parked();

        assert_eq!(
            AcpPublicMcpApprovalOutcome::Approved,
            outcome_rx.try_recv().expect("Public MCP approval response")
        );
    }

    // ===== 会话内搜索（findbar）=====

    /// 生成足够长的回复，让消息列表真的产生滚动区——否则「跳转」无从验证。
    fn long_reply(tag: &str) -> String {
        (0..12)
            .map(|index| {
                format!(
                    "{tag} 第 {index} 段：{}",
                    "让这一轮长得足以撑出滚动区".repeat(6)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 铺一段多轮对话；`needle_turns` 里的轮次额外带上 `needle`。
    fn push_conversation(
        view: &mut AgentChatView,
        turns: usize,
        needle_turns: &[usize],
        needle: &str,
    ) {
        for index in 0..turns {
            view.transcript
                .push_message(crate::ChatMessageUI::user(format!("问题 {index}")));
            let body = if needle_turns.contains(&index) {
                format!("{needle}\n\n{}", long_reply(&format!("回答 {index}")))
            } else {
                long_reply(&format!("回答 {index}"))
            };
            view.transcript
                .push_message(crate::ChatMessageUI::assistant(body));
        }
    }

    /// 强制跑一帧。`run_until_parked` 只跑后台任务，不落帧；
    /// 需要把焦点 / 滚动这类「下一帧才生效」的状态推进时，用一次 `debug_bounds` 触发绘制。
    fn draw_frame(cx: &mut VisualTestContext) {
        let _ = cx.debug_bounds("ai-chat-messages-scroll");
        cx.run_until_parked();
    }

    fn open_find_keystroke() -> &'static str {
        crate::find_shortcut::find_defaults_for_platform(cfg!(target_os = "macos")).0[0]
    }

    fn focus_composer(view: &Entity<AgentChatView>, cx: &mut VisualTestContext) {
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            input.update(cx, |input, cx| input.focus_input(window, cx));
        });
    }

    /// 快捷键打开 → 输入即搜 → Escape 关闭并清空命中。
    #[gpui::test]
    fn gpui_findbar_opens_with_shortcut_searches_and_closes_with_escape(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            push_conversation(view, 3, &[2], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("ai-chat-findbar").is_none(),
            "findbar must stay hidden until asked for"
        );

        // 真实姿势：用户正在输入框里打字，然后按 Cmd/Ctrl+F。
        // 焦点要等一帧才算落到已渲染帧上，否则按键派发的起点是空的。
        focus_composer(&view, cx);
        draw_frame(cx);

        // 先单独确认 action 接线本身是通的（与按键绑定解耦，失败时好定位）。
        cx.dispatch_action(ToggleTranscriptFind);
        draw_frame(cx);
        assert!(
            view.read_with(cx, |view, _| view.findbar_open),
            "the toggle action itself must open the findbar"
        );
        cx.dispatch_action(CloseTranscriptFind);
        draw_frame(cx);
        assert!(!view.read_with(cx, |view, _| view.findbar_open));

        cx.simulate_keystrokes(open_find_keystroke());
        draw_frame(cx);
        assert!(
            cx.debug_bounds("ai-chat-findbar").is_some(),
            "the find shortcut should open the findbar"
        );

        // 打开时焦点已在搜索框上，直接输入即可（不再额外点击）。
        cx.simulate_input("预算审批");
        cx.run_until_parked();
        assert_eq!(
            Some(2),
            view.read_with(cx, |view, _| view.search.current_turn_index()),
            "the only hit lives in the last turn"
        );
        assert_eq!(1, view.read_with(cx, |view, _| view.search.total()));

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("ai-chat-findbar").is_none(),
            "escape should close the findbar"
        );
        assert!(!view.read_with(cx, |view, _| view.findbar_open));
        assert!(
            !view.read_with(cx, |view, _| view.search.is_active()),
            "closing must drop the hit set, not leave stale highlights"
        );
    }

    /// 跳转要真的把目标轮次滚到视口顶部。
    ///
    /// 这条同时钉住一个**结构性不变量**：滚动容器的第 N 个直接子元素就是第 N 轮
    /// （`ScrollHandle::scroll_to_item` 以直接子元素为索引）。一旦有人再往里塞一层
    /// 包装，跳转会静默失效，这里会先炸。
    #[gpui::test]
    fn gpui_findbar_jump_scrolls_the_target_turn_to_the_top(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            // needle 放在中间轮：跳它不会被「滚到底」夹住，断言才是确定的。
            push_conversation(view, 12, &[5], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        assert_eq!(
            px(0.0),
            view.read_with(cx, |view, _| view.scroll_handle.offset().y),
            "the list should start at the top"
        );

        view.update(cx, |view, cx| {
            view.findbar_open = true;
            view.search.set_query("预算审批", &view.transcript.messages);
            view.jump_to_current_search_hit();
            cx.notify();
        });
        // 滚动是在 prepaint 里落地的：至少跑一帧，再读偏移。
        draw_frame(cx);
        draw_frame(cx);

        assert_eq!(
            Some(5),
            view.read_with(cx, |view, _| view.search.current_turn_index()),
            "the query must resolve to the middle turn"
        );
        assert!(
            view.read_with(cx, |view, _| view.scroll_handle.bounds_for_item(5))
                .is_some(),
            "the scroll container must index its turns directly, otherwise \
             `scroll_to_item` silently does nothing"
        );

        let (top_item, offset_y) = view.read_with(cx, |view, _| {
            (view.scroll_handle.top_item(), view.scroll_handle.offset().y)
        });
        // GPUI 把滚动偏移夹在 `[-max_offset, 0]`（见 `gpui` 的 `div.rs::compute_scroll_offset`），
        // 向下滚动时它是**负数**，所以这里不能断言「大于 0」。真正判方向的是下面的 `top_item`。
        assert_ne!(px(0.0), offset_y, "jumping must actually scroll");
        assert_eq!(
            5, top_item,
            "the hit turn should sit at the top of the viewport"
        );
    }

    /// 前后跳转按轮次推进，且**不碰 composer**。
    #[gpui::test]
    fn gpui_findbar_stepping_advances_hits_without_touching_the_composer(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            push_conversation(view, 4, &[0, 3], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        view.update(cx, |view, cx| {
            view.findbar_open = true;
            view.search.set_query("预算审批", &view.transcript.messages);
            cx.notify();
        });
        cx.run_until_parked();

        assert_eq!(2, view.read_with(cx, |view, _| view.search.total()));
        let first = view.read_with(cx, |view, _| view.search.current_turn_index());
        assert_eq!(Some(0), first);

        let next = cx
            .debug_bounds("ai-chat-findbar-next")
            .expect("next button should render");
        cx.simulate_click(next.center(), Modifiers::default());
        cx.run_until_parked();

        let second = view.read_with(cx, |view, _| view.search.current_turn_index());
        assert_ne!(first, second, "next must move to a different turn");
        assert_eq!(Some(3), second, "the second hit lives in the last turn");

        // composer 完全没被动过。
        assert!(
            view.read_with(cx, |view, cx| view.input.read(cx).composer_text(cx))
                .is_empty(),
            "searching must not write into the composer"
        );

        // 反向要能绕回第一条。
        let previous = cx
            .debug_bounds("ai-chat-findbar-prev")
            .expect("prev button should render");
        cx.simulate_click(previous.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            Some(0),
            view.read_with(cx, |view, _| view.search.current_turn_index())
        );
    }

    /// 正文变化（流式追加）后命中要重算，而不是停在旧结果上。
    #[gpui::test]
    fn gpui_findbar_recounts_hits_when_the_transcript_grows(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            push_conversation(view, 2, &[0], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        view.update(cx, |view, cx| {
            view.findbar_open = true;
            view.search.set_query("预算审批", &view.transcript.messages);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(1, view.read_with(cx, |view, _| view.search.total()));
        // 追加一轮含命中的对话：不重新 set_query，也要靠 revision 变更自动重算。
        view.update(cx, |view, cx| {
            view.transcript
                .push_message(crate::ChatMessageUI::user("再问一句".to_string()));
            view.transcript
                .push_message(crate::ChatMessageUI::assistant("预算审批 追加".to_string()));
            cx.notify();
        });
        // 命中重算发生在渲染里（按修订号判新），必须真落一帧。
        draw_frame(cx);

        assert_eq!(2, view.read_with(cx, |view, _| view.search.total()));
        assert!(
            view.read_with(cx, |view, _| view.search.hits_turn(2)),
            "the appended message belongs to a new turn and must be marked as a hit"
        );
    }

    /// 工具栏入口与「关闭」按钮走鼠标路径，同样可用。
    #[gpui::test]
    fn gpui_findbar_toolbar_button_toggles_and_close_button_closes(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            push_conversation(view, 2, &[1], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        let toggle = cx
            .debug_bounds("agent-chat-find")
            .expect("toolbar search entry should render");
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.findbar_open));
        assert!(cx.debug_bounds("ai-chat-findbar").is_some());

        let close = cx
            .debug_bounds("ai-chat-findbar-close")
            .expect("close button should render");
        cx.simulate_click(close.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.findbar_open));
        assert!(cx.debug_bounds("ai-chat-findbar").is_none());
    }

    /// 快捷键打开的 findbar，再按一次同样的键**不能关掉它**。
    ///
    /// `Cmd/Ctrl+F` 的语义是「找东西」而不是「别找了」：用户连按两下是常见姿势，
    /// 把面板和命中一起收掉属于误操作。关闭只走 `escape` / 关闭按钮 / 工具栏按钮。
    #[gpui::test]
    fn gpui_findbar_shortcut_refocuses_instead_of_closing(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            push_conversation(view, 3, &[2], "预算审批");
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();

        focus_composer(&view, cx);
        draw_frame(cx);
        cx.simulate_keystrokes(open_find_keystroke());
        draw_frame(cx);
        assert!(view.read_with(cx, |view, _| view.findbar_open));

        // 焦点此时已经在 findbar 的搜索框里，再按一次同样的键。
        cx.simulate_keystrokes(open_find_keystroke());
        draw_frame(cx);
        assert!(
            view.read_with(cx, |view, _| view.findbar_open),
            "pressing the find shortcut again must not close the findbar"
        );
        assert!(cx.debug_bounds("ai-chat-findbar").is_some());

        // 真正关掉之后，快捷键要能再打开：它是「打开/聚焦」，不是单向开关。
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.findbar_open));
        cx.simulate_keystrokes(open_find_keystroke());
        draw_frame(cx);
        assert!(
            cx.debug_bounds("ai-chat-findbar").is_some(),
            "the shortcut must still open the findbar after it was closed"
        );
    }

    struct NamedModelClient(String);
    #[async_trait]
    impl ModelClient for NamedModelClient {
        async fn complete(
            &self,
            _request: ModelRequest,
        ) -> Result<ModelResponse, agent_runtime::RuntimeError> {
            Ok(ModelResponse::text("ok"))
        }

        async fn complete_stream(
            &self,
            _request: ModelRequest,
        ) -> Result<ModelStream, agent_runtime::RuntimeError> {
            Ok(Box::pin(futures::stream::empty()))
        }

        fn model_name(&self) -> &str {
            &self.0
        }
    }

    /// 工作台把聊天面板当普通子视图塞进内容区。聊天框与聊天记录都必须在里面真正排布出来，
    /// 否则「工作台里没有输入框 / 看不到聊天记录」这类问题不会在面板自身的测试里暴露。
    #[gpui::test]
    fn gpui_workbench_content_hosts_the_chat_composer_and_transcript(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config =
            AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]).sidebar_mode(false);

        struct NavStub;

        impl Render for NavStub {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full()
            }
        }

        let (shell, cx) = cx.add_window_view(move |window, cx| {
            let chat = AgentChatView::view_with_config(config, window, cx);
            // 宿主会压掉内建侧栏（会话列表由外壳统一渲染），测试必须跟宿主同形。
            chat.update(cx, |chat, cx| chat.set_sidebar_suppressed(true, cx));
            let nav: gpui::AnyView = cx.new(|_| NavStub).into();
            crate::workbench::WorkbenchShell::new(
                crate::workbench::WorkbenchShellConfig {
                    panels: vec![crate::workbench::WorkbenchPanelEntry::new(
                        crate::workbench::WorkbenchPanelKind::Chat,
                        chat,
                    )],
                    // 宿主给的是内建会话列表；这里用等宽占位视图走同一条布局分支。
                    session_nav: Some(nav),
                    session_source: None,
                    initial_state: crate::workbench::WorkbenchState::new(
                        crate::workbench::WorkbenchPanelKind::Chat,
                    ),
                    theme: None,
                    subscriptions: Vec::new(),
                    workspace_root: None,
                },
                window,
                cx,
            )
        });

        let cx: &mut VisualTestContext = cx;
        cx.run_until_parked();
        let _ = shell;

        // 内容区必须自己拿到高度：外壳的行容器默认 items_center，不写 h_full 的高
        // 会在真机上塔成 0（子元素还在，但全被 overflow_hidden 裁掉 = 一片空白）。
        let content = cx
            .debug_bounds("workbench-content")
            .expect("the workbench content area must be laid out");
        assert!(
            content.size.height > px(0.0) && content.size.width > px(0.0),
            "the workbench content area must get a real size, got {:?}",
            content.size
        );

        let composer = cx
            .debug_bounds("agent-input-area")
            .expect("the workbench content area must lay out the chat composer");
        assert!(
            composer.size.height > px(0.0),
            "the chat composer must have a real height inside the workbench, got {:?}",
            composer.size
        );
        assert!(
            cx.debug_bounds("ai-chat-messages").is_some(),
            "the workbench content area must lay out the chat transcript"
        );
    }

    /// 底栏 chip 的动作必须延后到本视图的租借释放之后才执行。
    ///
    /// 触发链路（与线上逐字同形）：`AgentInput` emit → 本视图的
    /// `cx.subscribe_in` → GPUI 先 `subscriber.update(...)` 再回调，回调期间
    /// `AgentChatView` 已被租借 → `run_composer_action` 把动作交给宿主 →
    /// 宿主末了 `refresh_composer_context()` 回头 `update` 本视图。
    /// 原地同步调用就是 GPUI 的双重租借 panic
    /// （`cannot update ... while it is already being updated`）；而 macOS 的
    /// 事件回调是 `extern "C"`，panic 无法 unwind，会升级成
    /// `fatal runtime error` 直接 abort 整个进程 —— 点一下分支 / Worktree /
    /// 工作区就崩一次。延后（`Window::defer`）后动作照旧执行，只是落在租借
    /// 释放之后。
    #[gpui::test]
    fn gpui_composer_action_re_entering_the_view_does_not_double_lease(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_branch = calls.clone();

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let view_for_action = view.clone();
        let source = ComposerContextSource {
            snapshot: Arc::new(|_: &mut gpui::App| ComposerContextSnapshot::default()),
            select_workspace: Arc::new(
                |_: &std::path::Path, _: &mut gpui::Window, _: &mut gpui::App| {},
            ),
            browse_workspace: Arc::new(|_: &mut gpui::Window, _: &mut gpui::App| {}),
            // 宿主真实做法：切完分支再让视图重取快照 —— 也就是 update 本视图。
            select_branch: Arc::new(move |_: &SharedString, cx: &mut gpui::App| {
                calls_for_branch.fetch_add(1, Ordering::SeqCst);
                view_for_action.update(cx, |view, cx| view.refresh_composer_context(cx));
            }),
            toggle_worktree: Arc::new(|_: bool, _: &mut gpui::App| {}),
            // 这条用例只关心 chip 动作的重入，不涉及 worktree 创建。
            prepare_worktree: Arc::new(|_: &mut gpui::App| Task::ready(Ok(()))),
            report_worktree_failure: Arc::new(
                |_: &SharedString, _: &mut gpui::Window, _: &mut gpui::App| {},
            ),
        };
        view.update_in(cx, |view, _window, cx| {
            view.set_composer_context_source(source, cx)
        });

        let input = view.update_in(cx, |view, _window, _cx| view.input.clone());
        input.update(cx, |_, cx| {
            cx.emit(AgentInputEvent::SelectBranch { name: "dev".into() });
        });
        cx.run_until_parked();

        assert_eq!(
            1,
            calls.load(Ordering::SeqCst),
            "宿主动作必须照旧执行，只是延后到租借释放之后"
        );
    }

    /// 造一个「Worktree 已勾选、待创建」的上下文源。
    ///
    /// `prepare` 是「第一次发送时才创建」那一步的替身，测试用它分别走成功与失败两条路。
    /// 返回的第二个值是宿主侧「上报失败」的记录 —— 线上那一步会弹通知，这里只记下来
    /// 供断言。
    fn source_with_pending_worktree(
        prepare: Arc<dyn Fn(&mut gpui::App) -> Task<Result<(), SharedString>>>,
    ) -> (ComposerContextSource, Arc<Mutex<Vec<SharedString>>>) {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let reported_for_source = reported.clone();
        let source = ComposerContextSource {
            snapshot: Arc::new(|_: &mut gpui::App| ComposerContextSnapshot {
                worktree: ComposerWorktreeState::pending(),
                ..Default::default()
            }),
            select_workspace: Arc::new(
                |_: &std::path::Path, _: &mut gpui::Window, _: &mut gpui::App| {},
            ),
            browse_workspace: Arc::new(|_: &mut gpui::Window, _: &mut gpui::App| {}),
            select_branch: Arc::new(|_: &SharedString, _: &mut gpui::App| {}),
            toggle_worktree: Arc::new(|_: bool, _: &mut gpui::App| {}),
            prepare_worktree: prepare,
            report_worktree_failure: Arc::new(
                move |message: &SharedString, _: &mut gpui::Window, _: &mut gpui::App| {
                    reported_for_source.lock().unwrap().push(message.clone());
                },
            ),
        };
        (source, reported)
    }

    /// 走输入框的真实提交路径（与线上同一条：`AgentInput` emit → 本视图订阅）。
    ///
    /// 提交后必须跟着清空输入框，和 `AgentInput::submit` 同序：emit 是**延后派发**的
    /// （视图在 `flush_effects` 时才收到），清空紧接其后立刻做。少了这一步，
    /// 「失败时消息回到输入框」（`restore_to_composer`）的断言会因为输入框压根没被
    /// 清过而恒真 —— 那是条假绿。
    fn submit_from_composer(view: &Entity<AgentChatView>, text: &str, cx: &mut VisualTestContext) {
        let input = view.update_in(cx, |view, _window, _cx| view.input.clone());
        input.update_in(cx, |input, window, cx| {
            input.set_composer_text(text, window, cx);
        });
        input.update(cx, |_, cx| {
            cx.emit(AgentInputEvent::Submit {
                text: text.to_string(),
                mentions: Vec::new(),
                images: Vec::new(),
            });
        });
        input.update_in(cx, |input, window, cx| {
            input.set_composer_text("", window, cx);
        });
    }

    /// 勾选 Worktree 后第一次发送要先建 worktree，建好之前消息不能出去。
    ///
    /// 理由不是「省点资源」而是正确性：ACP 会话绑的是工作区根，根还没换到新
    /// worktree 就发出去，等于照旧跑在主工作区上，用户却以为自己在隔离环境里。
    #[gpui::test]
    fn pending_worktree_defers_the_first_submission(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let prepared = Arc::new(AtomicUsize::new(0));
        let prepared_for_source = prepared.clone();
        let (source, _reported) =
            source_with_pending_worktree(Arc::new(move |_: &mut gpui::App| {
                prepared_for_source.fetch_add(1, Ordering::SeqCst);
                // `git worktree add` 跑完了，但换根是**延后派发**的：
                // 视图这边的根这时还没落地。
                Task::ready(Ok(()))
            }));
        view.update_in(cx, |view, _window, cx| {
            view.set_composer_context_source(source, cx)
        });

        submit_from_composer(&view, "在 worktree 里跑", cx);
        cx.run_until_parked();

        assert_eq!(1, prepared.load(Ordering::SeqCst), "拦下提交时要发起创建");
        view.read_with(cx, |view, _| {
            assert!(
                !view.gated_submissions.is_empty(),
                "待创建的 worktree 必须拦下这次提交，等根切过去再发"
            );
            assert!(
                !view.current_session_has_messages(),
                "worktree 还没落地，消息不能先进了会话"
            );
        });
    }

    /// 创建失败：中止发送并把内容还回输入框。
    ///
    /// **不**悄悄降到主工作区继续发 —— 那种「静默换个地方跑」会让人以为改动落在
    /// 隔离的 worktree 里，实际全在主工作区上。勾选态保留，用户可以直接重试。
    #[gpui::test]
    fn worktree_creation_failure_restores_the_composer(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let (source, reported) = source_with_pending_worktree(Arc::new(|_: &mut gpui::App| {
            Task::ready(Err(SharedString::from("fatal: 分支已存在")))
        }));
        view.update_in(cx, |view, _window, cx| {
            view.set_composer_context_source(source, cx)
        });

        submit_from_composer(&view, "别把我的话弄丢了", cx);
        cx.run_until_parked();

        view.read_with(cx, |view, cx| {
            assert!(
                view.gated_submissions.is_empty(),
                "失败之后不能还挂着待发提交"
            );
            assert!(
                !view.current_session_has_messages(),
                "创建失败就不能把消息发出去"
            );
            assert_eq!(
                "别把我的话弄丢了",
                view.input.read(cx).composer_text(cx),
                "失败时消息必须回到输入框，否则用户白打一遍"
            );
        });
        assert_eq!(
            vec![SharedString::from("fatal: 分支已存在")],
            *reported.lock().unwrap(),
            "失败原因要交给宿主去说（线上是弹通知），视图不能默默吞掉"
        );
    }

    /// 根切到新 worktree 之后，被拦下的那条提交要接着发出去。
    ///
    /// 走 `apply_workspace_root`（`set_workspace_root` 的后半程，线上由 `RootChanged`
    /// 的订阅者触发），而不是直接调 `resume_gated_submission`：直接调后者的话，
    /// 「换根之后要把提交接上」这件事本身没有测试盯着 —— 把接线的那个调用删掉，
    /// 测试照样全绿。
    ///
    /// 不直接调 `set_workspace_root`：它会 `AppSettings::update_and_save`，在测试里
    /// 会拿假路径覆盖用户真实的 `last_workspace_root`。
    #[gpui::test]
    fn the_gated_submission_resumes_once_the_root_switches(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let (source, _reported) =
            source_with_pending_worktree(Arc::new(|_: &mut gpui::App| Task::ready(Ok(()))));
        view.update_in(cx, |view, _window, cx| {
            view.set_composer_context_source(source, cx)
        });

        submit_from_composer(&view, "在 worktree 里跑", cx);
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.gated_submissions.is_empty(), "前置：提交应当被拦下");
            assert!(!view.current_session_has_messages(), "前置：还没发出去");
        });

        view.update(cx, |view, cx| {
            view.apply_workspace_root(std::path::PathBuf::from("/tmp/navop-wt-under-test"), cx)
        });
        cx.run_until_parked();

        view.read_with(cx, |view, _| {
            assert!(view.gated_submissions.is_empty(), "接手之后不该再挂着");
            assert!(
                view.current_session_has_messages(),
                "根切过去之后必须接着把这条消息发出去"
            );
        });
    }

    /// 创建期间又发一条：只排队，**不能**再建第二个 worktree。
    ///
    /// 创建要秒级，这期间用户完全可能再发一条；单槽位实现会把上一条静默覆盖，
    /// 所以这里同时钉住「第二条被拦下」和「`prepare_worktree` 只调一次」。
    #[gpui::test]
    fn a_submission_during_creation_only_queues(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let created = Arc::new(AtomicUsize::new(0));
        let created_for_source = created.clone();
        let (source, _reported) =
            source_with_pending_worktree(Arc::new(move |_: &mut gpui::App| {
                created_for_source.fetch_add(1, Ordering::SeqCst);
                Task::ready(Ok(()))
            }));
        view.update_in(cx, |view, _window, cx| {
            view.set_composer_context_source(source, cx)
        });

        submit_from_composer(&view, "第一条", cx);
        cx.run_until_parked();
        // 创建还没落地（线上要等 `RootChanged` 级联），用户又发了一条。
        submit_from_composer(&view, "第二条", cx);
        cx.run_until_parked();

        assert_eq!(
            1,
            created.load(Ordering::SeqCst),
            "第二条只是排队，不能再建一个 worktree"
        );
        view.read_with(cx, |view, _| {
            assert_eq!(
                vec!["第一条", "第二条"],
                view.gated_submissions
                    .iter()
                    .map(|submission| submission.text.as_str())
                    .collect::<Vec<_>>(),
                "两条都要留着（旧的单槽位会覆盖掉第一条），且先来先发"
            );
            assert!(
                !view.current_session_has_messages(),
                "根还没切过去，两条都不许发"
            );
        });
    }
}

#[cfg(test)]
mod acp_probe_status_tests {
    use super::{AcpAgentProbe, probe_status_label, truncate_probe_error};
    use crate::acp::AcpModelInfo;

    fn model(id: &str) -> AcpModelInfo {
        AcpModelInfo {
            id: id.to_string(),
            label: id.to_string(),
        }
    }

    #[test]
    fn identified_agent_reports_name_version_and_model_count() {
        let probe = AcpAgentProbe {
            name: Some("Codex".to_string()),
            version: Some("1.2.3".to_string()),
            models: vec![model("gpt-5"), model("gpt-5-mini")],
            ..Default::default()
        };

        let label = probe_status_label(&probe).to_string();

        assert!(label.contains("Codex"), "{label}");
        assert!(label.contains("1.2.3"), "{label}");
        assert!(label.contains('2'), "{label}");
    }

    #[test]
    fn login_only_probe_marks_login_required() {
        let probe = AcpAgentProbe {
            name: Some("Kimi".to_string()),
            auth_methods: vec!["oauth".to_string()],
            ..Default::default()
        };

        let label = probe_status_label(&probe).to_string();

        assert!(label.contains("Kimi"), "{label}");
        assert_ne!("ACP Agent", label);
    }

    #[test]
    fn failed_probe_keeps_the_error_text() {
        let probe = AcpAgentProbe {
            error: Some("spawn failed: no such file".to_string()),
            ..Default::default()
        };

        let label = probe_status_label(&probe).to_string();

        assert!(label.contains("spawn failed"), "{label}");
    }

    #[test]
    fn empty_probe_falls_back_to_generic_label() {
        assert_eq!(
            "ACP Agent",
            probe_status_label(&AcpAgentProbe::default()).as_ref()
        );
    }

    #[test]
    fn probe_error_is_first_line_only_and_bounded() {
        let error = format!("{}\nsecond line", "x".repeat(200));
        let label = truncate_probe_error(&error);

        assert!(label.starts_with("xxx"), "{label}");
        assert!(!label.contains("second line"));
        assert_eq!(81, label.chars().count());
    }
}

#[cfg(test)]
mod acp_preconnect_model_tests {
    use super::acp_model_options_from_probe;
    use crate::acp::AcpModelInfo;
    use gpui::SharedString;

    fn model(id: &str, label: &str) -> AcpModelInfo {
        AcpModelInfo {
            id: id.to_string(),
            label: label.to_string(),
        }
    }

    #[test]
    fn probe_models_become_preconnect_options_with_distinct_ids() {
        let agent = SharedString::from("builtin.codex");
        let models = vec![model("gpt-5", "GPT-5"), model("gpt-5-mini", "GPT-5 mini")];

        let options = acp_model_options_from_probe(&agent, &models);

        assert_eq!(2, options.len());
        assert_eq!("gpt-5", options[0].model.as_ref());
        assert_eq!("builtin.codex", options[0].provider_id.as_ref());
        assert_eq!(
            "GPT-5",
            options[0].hint.as_ref().map(AsRef::as_ref).unwrap_or("")
        );
        assert_ne!(options[0].id, options[1].id, "选项 id 必须唯一");
        // ACP 标签只显示模型名，agent 名不进标签。
        assert!(options[0].model_only);
        assert_eq!("gpt-5", options[0].display_label().as_ref());
    }

    #[test]
    fn probe_options_are_scoped_to_their_agent() {
        let options = acp_model_options_from_probe(
            &SharedString::from("builtin.gemini"),
            &[model("gemini-2.5-pro", "Gemini 2.5 Pro")],
        );

        assert!(options[0].id.starts_with("acp:builtin.gemini:"));
    }

    #[test]
    fn empty_probe_models_produce_no_options() {
        assert!(acp_model_options_from_probe(&SharedString::from("a"), &[]).is_empty());
    }
}

#[cfg(test)]
mod acp_reconnect_tests {
    use super::{
        ACP_RECONNECT_MAX_ATTEMPTS, AcpReconnectDecision, acp_reconnect_decision,
        acp_reconnect_delay,
    };
    use std::time::Duration;

    #[test]
    fn reconnects_only_for_an_unavailable_owned_acp_target() {
        assert_eq!(
            AcpReconnectDecision::Reconnect,
            acp_reconnect_decision(true, true, false, true, 0)
        );
    }

    #[test]
    fn never_reconnects_without_a_target_or_when_not_acp() {
        // 用户切到内置 Agent：has_target 为 false，永不自动重连。
        assert_eq!(
            AcpReconnectDecision::Idle,
            acp_reconnect_decision(true, true, false, false, 0)
        );
        assert_eq!(
            AcpReconnectDecision::Idle,
            acp_reconnect_decision(false, true, false, true, 0)
        );
    }

    #[test]
    fn healthy_or_busy_connections_are_left_alone() {
        // 连接仍可用：不重连。
        assert_eq!(
            AcpReconnectDecision::Idle,
            acp_reconnect_decision(true, false, false, true, 0)
        );
        // 已有连接动作在飞：不并发拉起第二个进程。
        assert_eq!(
            AcpReconnectDecision::Idle,
            acp_reconnect_decision(true, true, true, true, 0)
        );
    }

    #[test]
    fn gives_up_after_the_attempt_budget() {
        let below = ACP_RECONNECT_MAX_ATTEMPTS - 1;
        assert_eq!(
            AcpReconnectDecision::Reconnect,
            acp_reconnect_decision(true, true, false, true, below)
        );
        assert_eq!(
            AcpReconnectDecision::GiveUp,
            acp_reconnect_decision(true, true, false, true, ACP_RECONNECT_MAX_ATTEMPTS)
        );
        assert_eq!(
            AcpReconnectDecision::GiveUp,
            acp_reconnect_decision(true, true, false, true, ACP_RECONNECT_MAX_ATTEMPTS + 5)
        );
    }

    #[test]
    fn backoff_doubles_and_is_bounded() {
        assert_eq!(Duration::from_millis(600), acp_reconnect_delay(0));
        assert_eq!(Duration::from_millis(1200), acp_reconnect_delay(1));
        assert_eq!(Duration::from_millis(2400), acp_reconnect_delay(2));
        // 上限：继续翻倍没有意义，且会撑大延迟。
        assert_eq!(Duration::from_millis(4800), acp_reconnect_delay(3));
        assert_eq!(Duration::from_millis(4800), acp_reconnect_delay(99));
    }

    #[test]
    fn in_flight_turns_block_reconnect_and_resume_after_turn_owners_clear() {
        // 连接断在轮次进行中：agent 那边还挂着我们的 turn，此刻拉新进程只会
        // 得到一个「同样的对话开第二个进程」。有 owner 在就让路，等轮次收尾。
        assert_eq!(
            AcpReconnectDecision::Idle,
            acp_reconnect_decision(true, true, true, true, 0)
        );
        // 轮次收尾（owner 清空）后：恢复重连。
        assert_eq!(
            AcpReconnectDecision::Reconnect,
            acp_reconnect_decision(true, true, false, true, 0)
        );
    }
}

#[cfg(test)]
mod acp_probe_hosting_tests {
    /// 回归：探测曾经直接在 GPUI foreground future 里 await，导致
    /// ① 在非 Tokio 线程创建 timer → `there is no reactor running`；
    /// ② 多个探测在同一线程交错持有 `EnterGuard` →
    ///    `EnterGuard values dropped out of order`。
    ///
    /// 因此探测必须放到 GPUI 后台线程，并用 `Handle::block_on` 进入 runtime。
    #[test]
    fn probes_run_off_the_gpui_thread() {
        let source = include_str!("agent_view.rs").replace("\r\n", "\n");
        let start = source
            .find("fn spawn_acp_probes")
            .expect("spawn_acp_probes should exist");
        let rest = &source[start + 1..];
        let end = rest
            .find("\n    fn ")
            .map(|offset| start + 1 + offset)
            .unwrap_or(source.len());
        let region = &source[start..end];

        assert!(
            region.contains("background_spawn"),
            "探测必须跑在 GPUI 后台线程上"
        );
        assert!(
            region.contains("probe_agent_blocking"),
            "必须用 block_on 进入 runtime，避免 EnterGuard 在同线程交错"
        );
        assert!(
            !region.contains("crate::acp::probe_agent("),
            "不要在 GPUI foreground future 里直接 await 异步探测"
        );
    }
}
