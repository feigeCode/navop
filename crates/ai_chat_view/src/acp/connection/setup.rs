use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{AuthMethodId, NewSessionRequest, SessionId};
use agent_client_protocol::{Agent, ConnectionTo};

use crate::acp::auth::{AuthDecision, authenticate, select_auth};
use crate::acp::client::build_initialize_request;
use crate::acp::config::{AcpAgentConfig, AcpTransport};
use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
use crate::acp::{AcpError, AcpErrorKind, AcpRecoveryAction};

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
) -> Result<SetupOutcome, agent_client_protocol::Error> {
    let init = connection
        .send_request(build_initialize_request())
        .block_task()
        .await?;
    if let Ok(mut state) = state.lock() {
        state.set_agent_capabilities(init.agent_capabilities.clone());
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
    create_session(connection, state, workspace_root)
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
    create_session(connection, state, workspace_root)
        .await
        .map_err(|error| session_error(config, error))
}

async fn create_session(
    connection: &ConnectionTo<Agent>,
    state: &Arc<Mutex<AcpSessionState>>,
    workspace_root: PathBuf,
) -> Result<SessionId, agent_client_protocol::Error> {
    transition_state(state, AcpConnectionPhase::CreatingSession);
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
        "ACP 会话创建失败",
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
    if let AcpTransport::Stdio { env, .. } = &config.transport {
        names.extend(
            env.iter()
                .filter(|(_, value)| !value.is_empty())
                .map(|(name, _)| name.clone()),
        );
    }
    names
}

fn acp_error_to_protocol(error: crate::acp::AcpError) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}
