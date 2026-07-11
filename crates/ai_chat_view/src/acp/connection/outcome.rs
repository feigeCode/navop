use agent_client_protocol::{Agent, ConnectionTo};

use crate::acp::state::AcpConnectionPhase;
use crate::acp::{AcpError, AcpErrorKind};

use super::lifecycle::AcpConnectionLifecycle;
use super::pending::AcpPendingConnection;
use super::runner::{ConnectShared, ReadyWait, SpawnedConnection};
use super::setup::SetupOutcome;
use super::{AcpConnectOutcome, AcpConnection, transition_state};

pub(super) fn finish_connect(
    shared: ConnectShared,
    spawned: SpawnedConnection,
    ready: ReadyWait,
) -> anyhow::Result<AcpConnectOutcome> {
    match ready {
        Ok(Ok(Ok((connection, SetupOutcome::Ready(session_id))))) => Ok(AcpConnectOutcome::Ready(
            Box::new(build_ready(shared, spawned, connection, session_id)),
        )),
        Ok(Ok(Ok((connection, SetupOutcome::AuthenticationRequired(methods))))) => {
            Ok(AcpConnectOutcome::AuthenticationRequired(Box::new(
                build_pending(shared, spawned, connection, methods),
            )))
        }
        Ok(Ok(Err(error))) => {
            abort_with_error(shared, spawned, format!("ACP agent 初始化失败:{error}"))
        }
        Ok(Err(_)) => abort_with_error(
            shared,
            spawned,
            "ACP agent 未就绪(进程可能已退出)".to_string(),
        ),
        Err(_) => abort_with_error(shared, spawned, "ACP agent 连接超时".to_string()),
    }
}

fn build_ready(
    shared: ConnectShared,
    spawned: SpawnedConnection,
    conn: ConnectionTo<Agent>,
    acp_session_id: agent_client_protocol::schema::SessionId,
) -> AcpConnection {
    let lifecycle = build_lifecycle(&shared, spawned);
    AcpConnection {
        handle: shared.handle,
        conn,
        acp_session_id,
        session_id: shared.session_id,
        events_tx: shared.events_tx,
        state: shared.state,
        active_turn: shared.active_turn,
        prompt_timeout: shared.config.timeouts.prompt,
        agent_id: shared.config.id.to_string(),
        agent_name: shared.config.name.to_string(),
        _lifecycle: lifecycle,
    }
}

fn build_pending(
    shared: ConnectShared,
    spawned: SpawnedConnection,
    conn: ConnectionTo<Agent>,
    methods: Vec<agent_client_protocol::schema::AuthMethodId>,
) -> AcpPendingConnection {
    let lifecycle = build_lifecycle(&shared, spawned);
    AcpPendingConnection {
        handle: shared.handle,
        conn,
        session_id: shared.session_id,
        events_tx: shared.events_tx,
        state: shared.state,
        active_turn: shared.active_turn,
        workspace_root: shared.workspace_root,
        config: shared.config,
        methods,
        lifecycle,
    }
}

fn build_lifecycle(shared: &ConnectShared, spawned: SpawnedConnection) -> AcpConnectionLifecycle {
    AcpConnectionLifecycle {
        handle: shared.handle.clone(),
        join: spawned.join,
        shutdown: Some(spawned.shutdown_tx),
        state: shared.state.clone(),
    }
}

fn abort_with_error(
    shared: ConnectShared,
    spawned: SpawnedConnection,
    message: String,
) -> anyhow::Result<AcpConnectOutcome> {
    spawned.join.abort();
    let error = AcpError::new(
        AcpErrorKind::InitializeFailed,
        shared.config.id.to_string(),
        shared.config.name.to_string(),
        message.clone(),
    );
    transition_state(&shared.state, AcpConnectionPhase::Failed { error });
    anyhow::bail!(message)
}
