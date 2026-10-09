use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, ReadTextFileRequest, RequestPermissionRequest,
    SessionId as AcpSessionId, SessionNotification, WriteTextFileRequest,
};
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use gpui::AsyncApp;
use one_core::gpui_tokio::Tokio;
use tokio::sync::{broadcast, oneshot};

use crate::acp::client::{handle_read_text_file_request, handle_write_text_file_request};
use crate::acp::config::AcpAgentConfig;
use crate::acp::elicitation::resolve_acp_elicitation_request;
use crate::acp::permission::resolve_acp_permission_request;
use crate::acp::state::{AcpConnectionPhase, AcpSessionState};

use super::notifications::{NotificationContext, handle_notification};
use super::outcome::finish_connect;
use super::setup::{SetupOutcome, setup_connection};
use super::{
    AcpActiveTurns, AcpClientProviders, AcpConnectOutcome, AcpDetailSessions,
    AcpInteractiveSession, connection_closed_error, fail_connection_and_take_active_turn,
    transition_state,
};

pub(super) type ReadyMessage = Result<(ConnectionTo<Agent>, SetupOutcome), String>;
pub(super) type ReadyWait =
    Result<Result<ReadyMessage, oneshot::error::RecvError>, tokio::time::error::Elapsed>;

#[derive(Clone)]
pub(super) struct ConnectShared {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) session_id: SessionId,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    /// 在飞轮次表；与 [`AcpConnection::active_turn`] 是同一个 `Arc`。
    pub(super) active_turn: AcpActiveTurns,
    /// 交互会话指针；与 [`AcpConnection::interactive_session`] 是同一个 `Arc`。
    ///
    /// 通知处理跑在另一条任务上（ACP 的 dispatch loop），拿不到 `AcpConnection`，
    /// 只能靠这个共享指针知道「一条 `session/update` 要不要应用到交互状态」。
    pub(super) interactive_session: AcpInteractiveSession,
    /// 历史回放窗口；与 [`AcpConnection::history_replay`] 是同一个 `Arc`。
    ///
    /// 通知处理跑在另一条任务上（ACP 的 dispatch loop），拿不到 `AcpConnection`，
    /// 只能靠这个共享槽位知道「现在收到的是 `session/load` 重放出来的历史」。
    pub(super) history_replay: Arc<Mutex<Option<TurnId>>>,
    /// 子代理详情会话注册表；与 [`AcpConnection::detail_sessions`] 是同一个 `Arc`。
    ///
    /// 通知处理拿不到 `AcpConnection`（跑在 ACP 的 dispatch loop 上），只能靠这块
    /// 共享表判断一条 `session/update` 是不是子代理详情会话的。
    pub(super) detail_sessions: AcpDetailSessions,
    pub(super) workspace_root: PathBuf,
    pub(super) config: AcpAgentConfig,
    /// 调用方希望复用的 ACP 协议会话（上次这个内置会话用的那个）。
    ///
    /// `None` 或复用失败都退回 `session/new`；见 `setup::open_session`。
    pub(super) resume: Option<AcpSessionId>,
}

pub(super) struct SpawnedConnection {
    pub(super) join: tokio::task::JoinHandle<()>,
    pub(super) shutdown_tx: oneshot::Sender<()>,
    ready_rx: Option<oneshot::Receiver<ReadyMessage>>,
}

struct ClientTaskContext {
    agent: AcpAgent,
    providers: AcpClientProviders,
    shared: ConnectShared,
    ready_tx: oneshot::Sender<ReadyMessage>,
    shutdown_rx: oneshot::Receiver<()>,
}

pub(super) async fn connect(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    cx: &mut AsyncApp,
) -> anyhow::Result<AcpConnectOutcome> {
    let handle = cx.update(|cx| Tokio::handle(cx));
    connect_with_parts(
        config,
        workspace_root,
        handle,
        AcpClientProviders::default(),
        None,
    )
    .await
}

pub(super) async fn connect_with_providers(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    providers: AcpClientProviders,
    cx: &mut AsyncApp,
) -> anyhow::Result<AcpConnectOutcome> {
    let handle = cx.update(|cx| Tokio::handle(cx));
    connect_with_parts(config, workspace_root, handle, providers, None).await
}

/// 带「沿用哪个 ACP 会话」的连接入口：重连 / 重启时优先 `load` 或 `resume`。
pub(super) async fn connect_with_providers_and_resume(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    providers: AcpClientProviders,
    resume: Option<AcpSessionId>,
    cx: &mut AsyncApp,
) -> anyhow::Result<AcpConnectOutcome> {
    let handle = cx.update(|cx| Tokio::handle(cx));
    connect_with_parts(config, workspace_root, handle, providers, resume).await
}

pub(super) async fn connect_with_runtime(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<AcpConnectOutcome> {
    connect_with_parts(
        config,
        workspace_root,
        handle,
        AcpClientProviders::default(),
        None,
    )
    .await
}

/// 带复用目标的运行时变体（不给视图、只给测试与嵌入式调用方）。
pub(super) async fn connect_with_runtime_and_resume(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    handle: tokio::runtime::Handle,
    resume: Option<AcpSessionId>,
) -> anyhow::Result<AcpConnectOutcome> {
    connect_with_parts(
        config,
        workspace_root,
        handle,
        AcpClientProviders::default(),
        resume,
    )
    .await
}

pub(super) async fn connect_with_runtime_and_providers(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    handle: tokio::runtime::Handle,
    providers: AcpClientProviders,
) -> anyhow::Result<AcpConnectOutcome> {
    connect_with_parts(config, workspace_root, handle, providers, None).await
}

async fn connect_with_parts(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    handle: tokio::runtime::Handle,
    providers: AcpClientProviders,
    resume: Option<AcpSessionId>,
) -> anyhow::Result<AcpConnectOutcome> {
    let shared = prepare_shared(config, workspace_root, handle, resume);
    let mut spawned = spawn_client(shared.clone(), providers);
    let ready_rx = spawned.ready_rx.take().expect("ready receiver must exist");
    let ready = wait_for_ready(&shared.handle, config.timeouts.connect, ready_rx).await;
    finish_connect(shared, spawned, ready)
}

async fn wait_for_ready(
    handle: &tokio::runtime::Handle,
    timeout: std::time::Duration,
    ready_rx: oneshot::Receiver<ReadyMessage>,
) -> ReadyWait {
    let _runtime = handle.enter();
    tokio::time::timeout(timeout, ready_rx).await
}

fn prepare_shared(
    config: &AcpAgentConfig,
    workspace_root: PathBuf,
    handle: tokio::runtime::Handle,
    resume: Option<AcpSessionId>,
) -> ConnectShared {
    // 4096：ACP 一轮会把每个工具调用展开成几十上百条 `session/update`，512 在
    // 长任务（子代理实测连续 182 分钟）里会被冲掉，订阅端 lagged 丢事件。
    // 丢了还有 `on_runtime_events_dropped` 的自愈兜底，但容量先把常见场景挡住。
    let (events_tx, _keep) = broadcast::channel(4096);
    let state = Arc::new(Mutex::new(AcpSessionState::default()));
    transition_state(&state, AcpConnectionPhase::Initializing);
    ConnectShared {
        handle,
        events_tx,
        session_id: SessionId::from_string(format!("acp:{}", uuid::Uuid::new_v4())),
        state,
        active_turn: Arc::new(Mutex::new(HashMap::new())),
        interactive_session: Arc::new(Mutex::new(String::new())),
        history_replay: Arc::new(Mutex::new(None)),
        detail_sessions: Arc::new(Mutex::new(HashMap::new())),
        workspace_root,
        config: config.clone(),
        resume,
    }
}

fn spawn_client(shared: ConnectShared, providers: AcpClientProviders) -> SpawnedConnection {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let context = ClientTaskContext {
        agent: shared.config.to_acp_agent(),
        providers,
        shared: shared.clone(),
        ready_tx,
        shutdown_rx,
    };
    let join = shared.handle.spawn(run_client(context));
    SpawnedConnection {
        join,
        shutdown_tx,
        ready_rx: Some(ready_rx),
    }
}

async fn run_client(context: ClientTaskContext) {
    let agent = context.agent;
    let permission_provider = context.providers.permission;
    let elicitation_provider = context.providers.elicitation;
    let shared = context.shared;
    let ready_tx = context.ready_tx;
    let shutdown_rx = context.shutdown_rx;
    let notification = NotificationContext::new(&shared);
    let read_root = shared.workspace_root.clone();
    let write_root = shared.workspace_root.clone();
    let setup_shared = shared.clone();
    let result = Client
        .builder()
        .on_receive_notification(
            async move |value: SessionNotification, _cx| handle_notification(&notification, value),
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                responder.respond(
                    resolve_acp_permission_request(permission_provider.clone(), request).await,
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ReadTextFileRequest, responder, _connection| {
                match handle_read_text_file_request(&request, &read_root) {
                    Ok(response) => responder.respond(response),
                    Err(error) => responder.respond_with_error(error),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: WriteTextFileRequest, responder, _connection| {
                match handle_write_text_file_request(&request, &write_root) {
                    Ok(response) => responder.respond(response),
                    Err(error) => responder.respond_with_error(error),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        // agent 主动提问：交给视图渲染成聊天里的问答卡片，结果原样送回。
        .on_receive_request(
            async move |request: CreateElicitationRequest, responder, _connection| {
                responder.respond(
                    resolve_acp_elicitation_request(elicitation_provider.clone(), request).await,
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async move |connection| {
            setup_and_park(connection, setup_shared, ready_tx, shutdown_rx).await
        })
        .await;
    if let Err(error) = result {
        handle_client_error(&shared, error);
    }
}

async fn setup_and_park(
    connection: ConnectionTo<Agent>,
    shared: ConnectShared,
    ready_tx: oneshot::Sender<ReadyMessage>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), agent_client_protocol::Error> {
    let setup = setup_connection(
        &connection,
        &shared.config,
        &shared.state,
        shared.workspace_root,
        shared.resume.clone(),
    )
    .await;
    match setup {
        Ok(outcome) => {
            let _ = ready_tx.send(Ok((connection, outcome)));
            let _ = shutdown_rx.await;
            Ok(())
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error.to_string()));
            Err(error)
        }
    }
}

fn handle_client_error(shared: &ConnectShared, protocol: agent_client_protocol::Error) {
    let protocol_detail = protocol.to_string();
    let error = connection_closed_error(
        shared.config.id.as_ref(),
        shared.config.name.as_ref(),
        Some(&protocol_detail),
    );
    if let Some(turn_id) =
        fail_connection_and_take_active_turn(&shared.active_turn, &shared.state, error.clone())
    {
        let _ = shared.events_tx.send(RuntimeEvent::TurnFailed {
            session_id: shared.session_id.clone(),
            turn_id,
            reason: error.to_string(),
        });
    }
}
