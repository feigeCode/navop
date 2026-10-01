//! ACP 连接句柄与生命周期入口。

mod lifecycle;
mod notifications;
mod outcome;
mod pending;
mod prompt;
mod runner;
mod session;
mod setup;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::SessionId as AcpSessionId;
use agent_client_protocol::{Agent, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use gpui::AsyncApp;
use tokio::sync::broadcast;

use crate::acp::config::AcpAgentConfig;
use crate::acp::elicitation::AcpElicitationProvider;
use crate::acp::permission::AcpPermissionProvider;
use crate::acp::state::{AcpConnectionPhase, AcpSessionContinuity, AcpSessionState};
use crate::acp::turn::AcpTurnTracker;
use crate::acp::{AcpError, AcpErrorKind};

use lifecycle::AcpConnectionLifecycle;
pub use pending::AcpPendingConnection;
pub use prompt::AcpPromptStartError;

/// 连接期间由视图提供的交互通道：agent 反过来问用户时的两条回程。
///
/// 两者都可以缺省（例如只在测试或纯后端场景里连一次）。缺省时对应请求立刻按「取消」
/// 回给 agent——宁可让它走降级路径，也不能让它在等一个永远不会有人回答的问题。
#[derive(Default, Clone)]
pub struct AcpClientProviders {
    /// 工具调用前的权限确认。
    pub permission: Option<AcpPermissionProvider>,
    /// agent 主动向用户提问（表单 / URL）。
    pub elicitation: Option<AcpElicitationProvider>,
}

impl AcpClientProviders {
    /// 同时提供权限确认与提问两条通道。
    pub fn new(
        permission: AcpPermissionProvider,
        elicitation: AcpElicitationProvider,
    ) -> Self {
        Self {
            permission: Some(permission),
            elicitation: Some(elicitation),
        }
    }
}

pub enum AcpConnectOutcome {
    Ready(Box<AcpConnection>),
    AuthenticationRequired(Box<AcpPendingConnection>),
}

/// 子代理**详情会话**注册表：子会话的协议 id → 内置事件流里的详情会话 id。
///
/// # 为什么需要一张共享表
///
/// `session/load` 一条子会话，作用是让 agent 把它登记进自己的会话表 —— OpenCode
/// 的 `ACPSession` 正因如此才会转发它的 `session/update`（未登记的子会话通知会被
/// `tryGet` 丢掉）。登记必须发生在请求**之前**，而通知处理跑在 ACP 的 dispatch
/// loop 上、拿不到 `AcpConnection`，只能靠这块共享表判断「这条通知不是主会话的」。
///
/// 表的内容只活在本次连接内：连接被替换/收掉时随 `Arc` 一起消失。
pub(crate) type AcpDetailSessions = Arc<Mutex<HashMap<String, SessionId>>>;

pub struct AcpConnection {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) conn: ConnectionTo<Agent>,
    pub(super) acp_session_id: AcpSessionId,
    pub(super) session_id: SessionId,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    pub(super) active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    /// 历史回放窗口：`session/load` 期间收到的 `session/update` 归属哪个**回放轮次**。
    ///
    /// 详见 [`AcpConnection::begin_history_replay`]。
    pub(super) history_replay: Arc<Mutex<Option<TurnId>>>,
    /// 已被登记为「子代理详情会话」的子会话；见 [`AcpDetailSessions`]。
    pub(super) detail_sessions: AcpDetailSessions,
    pub(super) prompt_timeout: std::time::Duration,
    pub(super) agent_id: String,
    pub(super) agent_name: String,
    _lifecycle: AcpConnectionLifecycle,
}

impl AcpConnection {
    pub async fn connect(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect(config, workspace_root, cx).await
    }

    /// 带上视图提供的交互通道连接。
    pub async fn connect_with_providers(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        providers: AcpClientProviders,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_providers(config, workspace_root, providers, cx).await
    }

    /// 连接并优先复用指定的 ACP 协议会话（拿不到就新建）。
    ///
    /// 重连 / 应用重启后走这条：不传 `resume` 的话每次都 `session/new`，
    /// agent 那边会一路堆空会话，用户看到的是「上次说的话全没了」。
    pub async fn connect_with_providers_and_resume(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        providers: AcpClientProviders,
        resume: Option<AcpSessionId>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_providers_and_resume(config, workspace_root, providers, resume, cx)
            .await
    }

    /// 只带权限确认通道（旧签名，保留给还不需要提问回程的调用方）。
    pub async fn connect_with_permission_provider(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        permission_provider: AcpPermissionProvider,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        Self::connect_with_providers(
            config,
            workspace_root,
            AcpClientProviders {
                permission: Some(permission_provider),
                elicitation: None,
            },
            cx,
        )
        .await
    }

    #[doc(hidden)]
    pub async fn connect_with_runtime(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        handle: tokio::runtime::Handle,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_runtime(config, workspace_root, handle).await
    }

    /// 只给运行时、只给复用目标的变体。
    #[doc(hidden)]
    pub async fn connect_with_runtime_and_resume(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        handle: tokio::runtime::Handle,
        resume: Option<AcpSessionId>,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_runtime_and_resume(config, workspace_root, handle, resume).await
    }

    #[doc(hidden)]
    pub async fn connect_with_runtime_and_providers(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        handle: tokio::runtime::Handle,
        providers: AcpClientProviders,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_runtime_and_providers(config, workspace_root, handle, providers).await
    }

    /// 只带权限确认通道的运行时变体（旧签名）。
    #[doc(hidden)]
    pub async fn connect_with_runtime_and_permission_provider(
        config: &AcpAgentConfig,
        workspace_root: std::path::PathBuf,
        handle: tokio::runtime::Handle,
        permission_provider: AcpPermissionProvider,
    ) -> anyhow::Result<AcpConnectOutcome> {
        Self::connect_with_runtime_and_providers(
            config,
            workspace_root,
            handle,
            AcpClientProviders {
                permission: Some(permission_provider),
                elicitation: None,
            },
        )
        .await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id.clone()
    }

    /// 当前指向的 ACP **协议**会话 id。
    ///
    /// 与 [`Self::session_id`] 不是一回事：那个是内置事件流的会话 id（`acp:<uuid>`），
    /// 这个才是能拿去 `session/load`、`session/resume` 的地址，重连时要记住它。
    pub fn protocol_session_id(&self) -> String {
        self.acp_session_id.0.to_string()
    }

    /// 这次连接实际怎么打开会话的（复用成功 / 没记忆新建 / 复用失败降级）。
    ///
    /// 视图据此决定要不要告诉用户「上一轮的上下文没接上」：复用失败只写日志的话，
    /// 用户会以为对话还在，继续追问才发现 agent 什么都不记得。
    pub fn session_continuity(&self) -> Option<AcpSessionContinuity> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.session_continuity())
    }

    pub fn phase(&self) -> AcpConnectionPhase {
        self.state
            .lock()
            .map(|state| state.phase().clone())
            .unwrap_or(AcpConnectionPhase::Closed)
    }

    /// 连接此刻认领的轮次；`None` 表示这一侧没有任何 prompt 在飞。
    ///
    /// 这是「这一轮到底结束没有」的**权威答案**——事件流是广播通道，会被挤掉，
    /// 视图侧靠事件推出来的 running 因此可能是过期的；`active_turn` 就在产出事件
    /// 的地方，不会落后。视图用它做丢事件之后的重同步判据，见
    /// [`AgentChatView::driver_reports_idle`](crate::agent_view)。
    ///
    /// `try_prompt` / `abandon_active_turn` / `claim_prompt_completion` 分别
    /// 在发起、本地放弃、正常结束时增删它，所以「`None`」只可能是这三种情况之一。
    pub fn active_turn_id(&self) -> Option<TurnId> {
        self.active_turn
            .lock()
            .ok()
            .and_then(|active| active.as_ref().map(|tracker| tracker.turn_id().clone()))
    }

    /// 本地放弃这一轮：agent 没有回终态，用户选择不再等。
    ///
    /// 与 [`claim_prompt_completion`] 的关键差别是**不产出任何终态事件**——迟到的
    /// 输出一律丢弃（视图那边已经没有对应的 owner 了）。必须做这一步，否则连接会
    /// 一直记着「这一轮还在跑」：用户下一条消息会被 [`AcpPromptStartError::AlreadyRunning`]
    /// 挡回来排进队列，而那一轮永远不会结束。
    ///
    /// 返回是否真的放弃了；轮次对不上说明这一轮已经结束或已被换掉。
    pub fn abandon_active_turn(&self, turn_id: &TurnId) -> bool {
        // 与 `claim_prompt_completion` 保持同一把锁的顺序：先 tracker,再 phase。
        let Ok(mut active) = self.active_turn.lock() else {
            return false;
        };
        if !active
            .as_ref()
            .is_some_and(|tracker| tracker.turn_id() == turn_id)
        {
            return false;
        }
        active.take();
        // 锁要分开取:`transition_state` 自己会再锁一次,持着锁调它会自锁。
        let running = self.state.lock().is_ok_and(|state| {
            matches!(
                state.phase(),
                AcpConnectionPhase::RunningTurn { turn_id: running } if running == turn_id
            )
        });
        if running {
            transition_state(&self.state, AcpConnectionPhase::Ready);
        }
        true
    }

    /// 开始把接下来收到的 `session/update` 当成**历史回放**收。
    ///
    /// `session/load` 会把整段历史重放成一批 `session/update`。那不是任何一轮的输出：
    /// 此刻 `active_turn` 是空的，通知层手里没有轮次 id，只能把它们全丢掉 —— 用户点开
    /// 一条 ACP 历史会话，屏幕上就什么都没有。
    ///
    /// 这个窗口给出的就是这个缺失的 id：窗口开着时，没有活动轮次的通知一律归属到它。
    /// 调用方（视图）拿同一个 id 去放行事件，见 `AgentChatView::acp_history_replay`。
    ///
    /// 窗口由 [`Self::end_history_replay`] 关闭，且**必须成对**：落单的回放轮次会让
    /// 之后所有无主通知都被算成历史。
    pub fn begin_history_replay(&self) -> TurnId {
        let turn_id = new_acp_replay_turn_id();
        if let Ok(mut replay) = self.history_replay.lock() {
            *replay = Some(turn_id.clone());
        }
        turn_id
    }

    /// 关闭历史回放窗口。
    ///
    /// 关得掉不代表回放"少一段"：协议保证 `session/load` 的响应一定在它引发的**所有**
    /// 通知之后派发（dispatch loop 逐条处理完才轮到下一条），所以响应回来的那一刻，
    /// 回放通知已经全部翻译成事件发出去了。
    pub fn end_history_replay(&self) {
        if let Ok(mut replay) = self.history_replay.lock() {
            *replay = None;
        }
    }

    /// 这条子会话是否已经被登记成详情会话。
    ///
    /// 视图用它避免对同一张卡片重复 `session/load` —— 每 load 一次，agent 就会把
    /// 整段子代理历史重放一遍，重复触发既浪费也刷屏。
    pub fn is_detail_session_registered(&self, acp_session_id: &str) -> bool {
        self.detail_sessions
            .lock()
            .is_ok_and(|sessions| sessions.contains_key(acp_session_id))
    }


    pub async fn set_model(
        &self,
        config_id: agent_client_protocol::schema::v1::SessionConfigId,
        value: agent_client_protocol::schema::v1::SessionConfigValueId,
    ) -> anyhow::Result<()> {
        self.set_config_option(config_id, value).await.map(|_| ())
    }

    pub(crate) fn state(&self) -> AcpSessionState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }
}

fn new_acp_turn_id() -> TurnId {
    TurnId::from_string(format!("acp-turn:{}", uuid::Uuid::new_v4()))
}

/// 历史回放的合成轮次 id。
///
/// 与 [`new_acp_turn_id`] 分开命名，是为了让日志和转录里的轮次一眼能分出「这一轮是
/// agent 重放的历史」和「这一轮是真跑出来的」。
fn new_acp_replay_turn_id() -> TurnId {
    TurnId::from_string(format!("acp-replay:{}", uuid::Uuid::new_v4()))
}

fn transition_state(state: &Arc<Mutex<AcpSessionState>>, phase: AcpConnectionPhase) {
    if let Ok(mut state) = state.lock()
        && let Err(error) = state.transition(phase)
    {
        tracing::warn!(%error, "invalid ACP phase transition");
    }
}

pub(super) enum PromptCompletionClaim {
    Ready(AcpTurnTracker),
    Failed { turn_id: TurnId, error: AcpError },
}

pub(super) fn claim_prompt_completion(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
    expected_turn_id: &TurnId,
    closed_error: &AcpError,
) -> Option<PromptCompletionClaim> {
    // All paths that need both mutexes use this order. Keeping the tracker
    // claimed together with the phase transition prevents prompt completion,
    // connection failure, and lifecycle shutdown from publishing competing
    // terminal events for the same turn.
    let mut active = active_turn.lock().ok()?;
    if !active
        .as_ref()
        .is_some_and(|tracker| tracker.turn_id() == expected_turn_id)
    {
        return None;
    }
    let mut state = state.lock().ok()?;
    match state.phase() {
        AcpConnectionPhase::RunningTurn { turn_id } if turn_id == expected_turn_id => {
            if let Err(error) = state.transition(AcpConnectionPhase::Ready) {
                tracing::warn!(%error, "failed to finish ACP prompt phase");
                return None;
            }
            active.take().map(PromptCompletionClaim::Ready)
        }
        AcpConnectionPhase::Failed { error } => {
            let error = error.clone();
            active.take().map(|tracker| PromptCompletionClaim::Failed {
                turn_id: tracker.turn_id().clone(),
                error,
            })
        }
        AcpConnectionPhase::Closed => active.take().map(|tracker| PromptCompletionClaim::Failed {
            turn_id: tracker.turn_id().clone(),
            error: closed_error.clone(),
        }),
        phase => {
            tracing::warn!(
                ?phase,
                expected_turn_id = %expected_turn_id,
                "ACP prompt completed outside its running phase"
            );
            None
        }
    }
}

pub(super) fn fail_connection_and_take_active_turn(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
    error: AcpError,
) -> Option<TurnId> {
    let mut active = active_turn.lock().ok()?;
    let mut state = state.lock().ok()?;
    if let Err(transition_error) = state.transition(AcpConnectionPhase::Failed { error }) {
        tracing::warn!(%transition_error, "failed to mark ACP connection as failed");
        return None;
    }
    active.take().map(|tracker| tracker.turn_id().clone())
}

pub(super) fn close_connection_and_take_active_turn(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
) -> Option<TurnId> {
    let mut active = active_turn.lock().ok()?;
    let mut state = state.lock().ok()?;
    if !matches!(state.phase(), AcpConnectionPhase::Closed)
        && let Err(error) = state.transition(AcpConnectionPhase::Closed)
    {
        tracing::warn!(%error, "failed to close ACP connection phase");
        return None;
    }
    active.take().map(|tracker| tracker.turn_id().clone())
}

pub(super) fn connection_closed_error(
    agent_id: &str,
    agent_name: &str,
    detail: Option<&str>,
) -> AcpError {
    let error = AcpError::new(
        AcpErrorKind::ConnectionClosed,
        agent_id,
        agent_name,
        rust_i18n::t!("AgentUi.acp_connection_closed").to_string(),
    );
    match detail {
        Some(detail) => error.with_detail(detail),
        None => error,
    }
}
