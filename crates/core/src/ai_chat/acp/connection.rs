use agent_client_protocol::schema::{
    AuthMethod, AuthMethodId, AuthenticateRequest, CancelNotification, ContentBlock,
    InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion, RequestPermissionRequest,
    SessionId as AcpSessionId, SessionNotification, TextContent,
};
use agent_client_protocol::{Agent, Client, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use gpui::AsyncApp;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

use crate::ai_chat::acp::config::{AcpAgentConfig, AcpTransport};
use crate::ai_chat::acp::permission::{acp_permission_provider, resolve_acp_permission_request};
use crate::ai_chat::acp::translate::session_update_to_events;
use crate::gpui_tokio::Tokio;

const AUTHENTICATE_TIMEOUT: Duration = Duration::from_secs(15);

pub struct AcpConnection {
    handle: tokio::runtime::Handle,
    conn: ConnectionTo<Agent>,
    acp_session_id: AcpSessionId,
    session_id: SessionId,
    turn_id: TurnId,
    events_tx: broadcast::Sender<RuntimeEvent>,
    join: tokio::task::JoinHandle<()>,
    _shutdown: oneshot::Sender<()>,
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        self.join.abort();
    }
}

impl AcpConnection {
    pub async fn connect(config: &AcpAgentConfig, cx: &mut AsyncApp) -> anyhow::Result<Self> {
        let handle = cx.update(|cx| Tokio::handle(cx));
        let permission_provider = cx.update(|cx| acp_permission_provider(cx));
        let agent = config.to_acp_agent();
        let (events_tx, _keep) = broadcast::channel(512);
        let session_id = SessionId::from_string(format!("acp:{}", uuid::Uuid::new_v4()));
        let turn_id = TurnId::from_string(format!("acp-turn:{}", uuid::Uuid::new_v4()));
        let (ready_tx, ready_rx) =
            oneshot::channel::<Result<(ConnectionTo<Agent>, AcpSessionId), String>>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let config_for_auth = config.clone();
        let ev_notif = events_tx.clone();
        let ev_err = events_tx.clone();
        let sid_notif = session_id.clone();
        let tid_notif = turn_id.clone();
        let sid_err = session_id.clone();
        let tid_err = turn_id.clone();
        let permission_provider_for_request = permission_provider.clone();

        let join = handle.spawn(async move {
            let result = Client
                .builder()
                .on_receive_notification(
                    async move |notification: SessionNotification, _cx| {
                        for event in
                            session_update_to_events(&notification.update, &sid_notif, &tid_notif)
                        {
                            let _ = ev_notif.send(event);
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        responder.respond(
                            resolve_acp_permission_request(
                                permission_provider_for_request.clone(),
                                request,
                            )
                            .await,
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
                    let setup = setup_session(&connection, &config_for_auth).await;
                    match setup {
                        Ok(acp_session_id) => {
                            let _ = ready_tx.send(Ok((connection.clone(), acp_session_id)));
                            let _ = shutdown_rx.await;
                            Ok(())
                        }
                        Err(error) => {
                            let _ = ready_tx.send(Err(error.to_string()));
                            Err(error)
                        }
                    }
                })
                .await;

            if let Err(error) = result {
                let _ = ev_err.send(RuntimeEvent::TurnFailed {
                    session_id: sid_err,
                    turn_id: tid_err,
                    reason: format!("ACP connection ended: {error}"),
                });
            }
        });

        match ready_rx.await {
            Ok(Ok((conn, acp_session_id))) => Ok(Self {
                handle,
                conn,
                acp_session_id,
                session_id,
                turn_id,
                events_tx,
                join,
                _shutdown: shutdown_tx,
            }),
            Ok(Err(error)) => {
                join.abort();
                anyhow::bail!("ACP agent initialization failed: {error}")
            }
            Err(_) => {
                join.abort();
                anyhow::bail!("ACP agent did not become ready")
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id.clone()
    }

    pub fn prompt(&self, text: String) {
        let _ = self.events_tx.send(RuntimeEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
        });
        let conn = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        let events = self.events_tx.clone();
        let sid = self.session_id.clone();
        let tid = self.turn_id.clone();
        self.handle.spawn(async move {
            let request = PromptRequest::new(
                acp_session_id,
                vec![ContentBlock::Text(TextContent::new(text))],
            );
            match conn.send_request(request).block_task().await {
                Ok(response) => {
                    tracing::info!(stop_reason = ?response.stop_reason, "ACP prompt completed");
                    let _ = events.send(RuntimeEvent::TurnCompleted {
                        session_id: sid,
                        turn_id: tid,
                        answer: None,
                    });
                }
                Err(error) => {
                    let _ = events.send(RuntimeEvent::TurnFailed {
                        session_id: sid,
                        turn_id: tid,
                        reason: error.to_string(),
                    });
                }
            }
        });
    }

    pub fn cancel(&self) {
        let conn = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        self.handle.spawn(async move {
            let _ = conn.send_notification(CancelNotification::new(acp_session_id));
        });
    }
}

async fn setup_session(
    connection: &ConnectionTo<Agent>,
    config: &AcpAgentConfig,
) -> Result<AcpSessionId, agent_client_protocol::Error> {
    let init = connection
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    if let Some(method_id) = select_non_interactive_auth_method(&init.auth_methods, config) {
        authenticate(connection, method_id).await;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let response = connection
        .send_request(NewSessionRequest::new(cwd))
        .block_task()
        .await?;
    Ok(response.session_id)
}

async fn authenticate(connection: &ConnectionTo<Agent>, method_id: AuthMethodId) {
    let request = connection
        .send_request(AuthenticateRequest::new(method_id))
        .block_task();
    match tokio::time::timeout(AUTHENTICATE_TIMEOUT, request).await {
        Ok(Ok(_)) => tracing::info!("ACP authenticate completed"),
        Ok(Err(error)) => tracing::warn!("ACP authenticate failed, continuing: {error}"),
        Err(_) => tracing::warn!("ACP authenticate timed out, continuing"),
    }
}

fn select_non_interactive_auth_method(
    methods: &[AuthMethod],
    config: &AcpAgentConfig,
) -> Option<AuthMethodId> {
    methods
        .iter()
        .find(|method| auth_env_available(method.id(), config))
        .map(|method| method.id().clone())
}

fn auth_env_available(method_id: &AuthMethodId, config: &AcpAgentConfig) -> bool {
    let method_key = method_id.0.to_ascii_lowercase();
    let config_env = match &config.transport {
        AcpTransport::Stdio { env, .. } => env.as_slice(),
        AcpTransport::Http { .. } => &[],
    };
    config_env
        .iter()
        .any(|(name, value)| !value.is_empty() && normalize_env_name(name) == method_key)
        || std::env::vars()
            .any(|(name, value)| !value.is_empty() && normalize_env_name(&name) == method_key)
}

fn normalize_env_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::normalize_env_name;

    #[test]
    fn normalizes_auth_env_names_to_acp_method_keys() {
        assert_eq!("api-key", normalize_env_name("API_KEY"));
        assert_eq!("oauth-token", normalize_env_name("oauth-token"));
    }
}
