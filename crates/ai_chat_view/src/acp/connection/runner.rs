use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
    ReadTextFileRequest, RequestPermissionRequest, SessionNotification, WriteTextFileRequest,
};
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId};
use gpui::AsyncApp;
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use tokio::sync::{broadcast, oneshot};

use crate::acp::client::{handle_read_text_file_request, handle_write_text_file_request};
use crate::acp::config::AcpAgentConfig;
use crate::acp::permission::{AcpPermissionProvider, resolve_acp_permission_request};
use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
use crate::acp::turn::AcpTurnTracker;
use crate::acp::{AcpError, AcpErrorKind};

use super::notifications::{NotificationContext, handle_notification};
use super::outcome::finish_connect;
use super::setup::{SetupOutcome, setup_connection};
use super::{AcpConnectOutcome, take_active_turn_id, transition_state};

pub(super) type ReadyMessage = Result<(ConnectionTo<Agent>, SetupOutcome), String>;
pub(super) type ReadyWait =
    Result<Result<ReadyMessage, oneshot::error::RecvError>, tokio::time::error::Elapsed>;

#[derive(Clone)]
pub(super) struct ConnectShared {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) session_id: SessionId,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    pub(super) active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    pub(super) workspace_root: PathBuf,
    pub(super) config: AcpAgentConfig,
}

pub(super) struct SpawnedConnection {
    pub(super) join: tokio::task::JoinHandle<()>,
    pub(super) shutdown_tx: oneshot::Sender<()>,
    ready_rx: Option<oneshot::Receiver<ReadyMessage>>,
}

struct ClientTaskContext {
    agent: AcpAgent,
    permission_provider: Option<AcpPermissionProvider>,
    shared: ConnectShared,
    ready_tx: oneshot::Sender<ReadyMessage>,
    shutdown_rx: oneshot::Receiver<()>,
}

pub(super) async fn connect(
    config: &AcpAgentConfig,
    cx: &mut AsyncApp,
) -> anyhow::Result<AcpConnectOutcome> {
    let handle = cx.update(|cx| Tokio::handle(cx));
    connect_with_parts(config, handle, None).await
}

pub(super) async fn connect_with_permission_provider(
    config: &AcpAgentConfig,
    permission_provider: AcpPermissionProvider,
    cx: &mut AsyncApp,
) -> anyhow::Result<AcpConnectOutcome> {
    let handle = cx.update(|cx| Tokio::handle(cx));
    connect_with_parts(config, handle, Some(permission_provider)).await
}

pub(super) async fn connect_with_runtime(
    config: &AcpAgentConfig,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<AcpConnectOutcome> {
    connect_with_parts(config, handle, None).await
}

pub(super) async fn connect_with_runtime_and_permission_provider(
    config: &AcpAgentConfig,
    handle: tokio::runtime::Handle,
    permission_provider: AcpPermissionProvider,
) -> anyhow::Result<AcpConnectOutcome> {
    connect_with_parts(config, handle, Some(permission_provider)).await
}

async fn connect_with_parts(
    config: &AcpAgentConfig,
    handle: tokio::runtime::Handle,
    permission_provider: Option<AcpPermissionProvider>,
) -> anyhow::Result<AcpConnectOutcome> {
    let shared = prepare_shared(config, handle);
    let mut spawned = spawn_client(shared.clone(), permission_provider);
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

fn prepare_shared(config: &AcpAgentConfig, handle: tokio::runtime::Handle) -> ConnectShared {
    let (events_tx, _keep) = broadcast::channel(512);
    let state = Arc::new(Mutex::new(AcpSessionState::default()));
    transition_state(&state, AcpConnectionPhase::Initializing);
    ConnectShared {
        handle,
        events_tx,
        session_id: SessionId::from_string(format!("acp:{}", uuid::Uuid::new_v4())),
        state,
        active_turn: Arc::new(Mutex::new(None)),
        workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        config: config.clone(),
    }
}

fn spawn_client(
    shared: ConnectShared,
    permission_provider: Option<AcpPermissionProvider>,
) -> SpawnedConnection {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let context = ClientTaskContext {
        agent: shared.config.to_acp_agent(),
        permission_provider,
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
    let permission_provider = context.permission_provider;
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
    let error = AcpError::new(
        AcpErrorKind::ConnectionClosed,
        shared.config.id.to_string(),
        shared.config.name.to_string(),
        t!("AgentUi.acp_connection_closed").to_string(),
    )
    .with_detail(protocol.to_string());
    transition_state(
        &shared.state,
        AcpConnectionPhase::Failed {
            error: error.clone(),
        },
    );
    if let Some(turn_id) = take_active_turn_id(&shared.active_turn) {
        let _ = shared.events_tx.send(RuntimeEvent::TurnFailed {
            session_id: shared.session_id.clone(),
            turn_id,
            reason: error.to_string(),
        });
    }
}
