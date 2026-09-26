use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AuthMethodId, LoadSessionRequest, NewSessionRequest, ResumeSessionRequest, SessionId,
};
use agent_client_protocol::{Agent, ConnectionTo};
use rust_i18n::t;

use crate::acp::auth::{AuthDecision, authenticate, select_auth};
use crate::acp::client::build_initialize_request;
use crate::acp::config::{AcpAgentConfig, AcpTransport};
use crate::acp::state::{AcpConnectionPhase, AcpSessionContinuity, AcpSessionState};
use crate::acp::{
    AcpError, AcpErrorKind, AcpRecoveryAction, AcpSessionOpen, acp_session_open_kind,
};

use super::transition_state;

pub(super) enum SetupOutcome {
    Ready(SessionId),
    AuthenticationRequired(Vec<AuthMethodId>),
}

pub(super) async fn setup_connection(
    connection: &ConnectionTo<Agent>,
    config: &AcpAgentConfig,
    state: &Arc<Mutex<AcpSessionState>>,
    workspace_root: PathBuf,
    resume: Option<SessionId>,
) -> Result<SetupOutcome, agent_client_protocol::Error> {
    let init = connection
        .send_request(build_initialize_request())
        .block_task()
        .await?;
    if let Ok(mut state) = state.lock() {
        state.set_agent_capabilities(init.agent_capabilities.clone());
        state.set_agent_info(init.agent_info.clone());
    }
    let advertised = init
        .auth_methods
        .iter()
        .map(|method| method.id().clone())
        .collect::<Vec<_>>();
    tracing::info!(agent_info = ?init.agent_info, auth_methods = ?advertised, "ACP initialized");
    if let Some(methods) = apply_auth(connection, config, state, &advertised).await? {
        return Ok(SetupOutcome::AuthenticationRequired(methods));
    }
    open_session(connection, state, workspace_root, resume)
        .await
        .map(SetupOutcome::Ready)
}

async fn apply_auth(
    connection: &ConnectionTo<Agent>,
    config: &AcpAgentConfig,
    state: &Arc<Mutex<AcpSessionState>>,
    advertised: &[AuthMethodId],
) -> Result<Option<Vec<AuthMethodId>>, agent_client_protocol::Error> {
    let decision = select_auth(
        advertised,
        &config.auth,
        &available_env(config),
        config.id.as_ref(),
        config.name.as_ref(),
    )
    .map_err(acp_error_to_protocol)?;
    match decision {
        AuthDecision::Authenticate(method_id) => {
            transition_state(
                state,
                AcpConnectionPhase::Authenticating {
                    method_id: method_id.0.to_string(),
                },
            );
            authenticate(
                connection,
                method_id,
                config.timeouts.authenticate,
                config.id.as_ref(),
                config.name.as_ref(),
            )
            .await
            .map_err(acp_error_to_protocol)?;
            Ok(None)
        }
        AuthDecision::RequireInteraction { methods } => {
            Ok(Some(authentication_required(state, methods)))
        }
        AuthDecision::SkipNoMethods | AuthDecision::UseLocalFallback => Ok(None),
    }
}

fn authentication_required(
    state: &Arc<Mutex<AcpSessionState>>,
    methods: Vec<AuthMethodId>,
) -> Vec<AuthMethodId> {
    let labels = methods
        .iter()
        .map(|method| method.0.to_string())
        .collect::<Vec<_>>();
    transition_state(
        state,
        AcpConnectionPhase::AuthenticationRequired { methods: labels },
    );
    methods
}

pub(super) async fn complete_authentication(
    connection: &ConnectionTo<Agent>,
    config: &AcpAgentConfig,
    state: &Arc<Mutex<AcpSessionState>>,
    workspace_root: PathBuf,
    method_id: AuthMethodId,
    resume: Option<SessionId>,
) -> Result<SessionId, AcpError> {
    transition_state(
        state,
        AcpConnectionPhase::Authenticating {
            method_id: method_id.0.to_string(),
        },
    );
    authenticate(
        connection,
        method_id,
        config.timeouts.authenticate,
        config.id.as_ref(),
        config.name.as_ref(),
    )
    .await?;
    open_session(connection, state, workspace_root, resume)
        .await
        .map_err(|error| session_error(config, error))
}

/// 打开会话：能复用上次那个就复用，否则新建。
///
/// 复用走 agent 声明的能力（`session/load` 能回放历史，优先；否则 `session/resume`）。
/// **复用失败必须降级而不是把整次连接判死**：那个会话可能已被 agent 删掉、或不再属于
/// 当前工作区，为此报连接失败比多开一个空会话糟得多。
///
/// 无论走哪条路，都把 [`AcpSessionContinuity`] 记进状态：视图据此决定要不要告诉用户
/// 「上一轮的上下文没接上」。只写日志的话，用户会以为对话还在。
async fn open_session(
    connection: &ConnectionTo<Agent>,
    state: &Arc<Mutex<AcpSessionState>>,
    workspace_root: PathBuf,
    resume: Option<SessionId>,
) -> Result<SessionId, agent_client_protocol::Error> {
    if let Some(target) = resume {
        let open_kind = state
            .lock()
            .ok()
            .and_then(|state| acp_session_open_kind(state.agent_capabilities()));
        if let Some(kind) = reopen_kind(open_kind) {
            if reopen_session(connection, state, &target, &workspace_root, kind)
                .await
                .is_ok()
            {
                record_session_continuity(state, continuity_for_reopen(kind));
                return Ok(target);
            }
            tracing::warn!(
                session = %target.0,
                open = ?kind,
                "failed to reopen the remembered ACP session; falling back to a new one"
            );
            let created = create_session(connection, state, workspace_root).await?;
            record_session_continuity(state, AcpSessionContinuity::RestartedAfterReuseFailure);
            return Ok(created);
        }
        tracing::debug!(
            session = %target.0,
            "agent advertises neither session/load nor session/resume; creating a new session"
        );
    }
    let created = create_session(connection, state, workspace_root).await?;
    record_session_continuity(state, AcpSessionContinuity::StartedFresh);
    Ok(created)
}

/// agent 声明的复用能力 → 实际要发的复用请求。没有能力可依时返回 `None`（只能新建）。
fn reopen_kind(open: Option<AcpSessionOpen>) -> Option<Reopen> {
    match open {
        Some(AcpSessionOpen::Load) => Some(Reopen::Load),
        Some(AcpSessionOpen::Resume) => Some(Reopen::Resume),
        None => None,
    }
}

fn continuity_for_reopen(kind: Reopen) -> AcpSessionContinuity {
    match kind {
        Reopen::Load => AcpSessionContinuity::ReusedWithHistory,
        Reopen::Resume => AcpSessionContinuity::ReusedWithoutHistory,
    }
}

fn record_session_continuity(
    state: &Arc<Mutex<AcpSessionState>>,
    continuity: AcpSessionContinuity,
) {
    if let Ok(mut state) = state.lock() {
        state.set_session_continuity(continuity);
    }
}

#[derive(Clone, Copy, Debug)]
enum Reopen {
    Load,
    Resume,
}

/// 复用一条 ACP 会话。失败（含能力缺失）返回 `Err(())`，让调用方退回新建。
async fn reopen_session(
    connection: &ConnectionTo<Agent>,
    state: &Arc<Mutex<AcpSessionState>>,
    target: &SessionId,
    workspace_root: &PathBuf,
    kind: Reopen,
) -> Result<(), ()> {
    ensure_creating_session(state);
    let reopened = match kind {
        Reopen::Load => {
            let response = connection
                .send_request(LoadSessionRequest::new(
                    target.clone(),
                    workspace_root.clone(),
                ))
                .block_task()
                .await;
            response.map(|response| {
                if let Ok(mut state) = state.lock() {
                    state.apply_load_session_response(&response);
                }
                "loaded"
            })
        }
        Reopen::Resume => {
            let response = connection
                .send_request(ResumeSessionRequest::new(
                    target.clone(),
                    workspace_root.clone(),
                ))
                .block_task()
                .await;
            response.map(|response| {
                if let Ok(mut state) = state.lock() {
                    state.apply_resume_session_response(&response);
                }
                "resumed"
            })
        }
    };
    let Ok(how) = reopened else {
        return Err(());
    };
    transition_state(state, AcpConnectionPhase::Ready);
    tracing::info!(session = %target.0, how, "ACP session reused");
    Ok(())
}

/// 推进到 `CreatingSession`；已经是该相位时不再重复推进。
///
/// 复用失败后退回新建会走到这里第二次，重复 transition 只会打出一条没有意义的
/// 「非法相位」告警。
fn ensure_creating_session(state: &Arc<Mutex<AcpSessionState>>) {
    let already_there = state
        .lock()
        .map(|state| state.phase() == &AcpConnectionPhase::CreatingSession)
        .unwrap_or(false);
    if !already_there {
        transition_state(state, AcpConnectionPhase::CreatingSession);
    }
}

async fn create_session(
    connection: &ConnectionTo<Agent>,
    state: &Arc<Mutex<AcpSessionState>>,
    workspace_root: PathBuf,
) -> Result<SessionId, agent_client_protocol::Error> {
    ensure_creating_session(state);
    let response = connection
        .send_request(NewSessionRequest::new(workspace_root))
        .block_task()
        .await?;
    if let Ok(mut state) = state.lock() {
        state.apply_new_session_response(&response);
    }
    transition_state(state, AcpConnectionPhase::Ready);
    tracing::info!(session = %response.session_id.0, "ACP session created");
    Ok(response.session_id)
}

fn session_error(config: &AcpAgentConfig, error: agent_client_protocol::Error) -> AcpError {
    AcpError::new(
        AcpErrorKind::SessionCreationFailed,
        config.id.to_string(),
        config.name.to_string(),
        t!("AgentUi.acp_session_creation_failed").to_string(),
    )
    .with_detail(crate::acp::error::extract_rpc_error_detail(
        &error.message,
        error.data.as_ref(),
    ))
    .with_recovery(AcpRecoveryAction::Retry)
}

fn available_env(config: &AcpAgentConfig) -> BTreeSet<String> {
    let mut names = std::env::vars()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>();
    let AcpTransport::Stdio { env, .. } = &config.transport;
    names.extend(
        env.iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| name.clone()),
    );
    names
}

fn acp_error_to_protocol(error: crate::acp::AcpError) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}
