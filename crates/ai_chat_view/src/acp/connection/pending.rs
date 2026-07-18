use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{AuthMethodId, SessionId as AcpSessionId};
use agent_client_protocol::{Agent, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId};
use rust_i18n::t;
use tokio::sync::broadcast;

use crate::acp::config::AcpAgentConfig;
use crate::acp::state::AcpSessionState;
use crate::acp::turn::AcpTurnTracker;
use crate::acp::{AcpError, AcpErrorKind};

use super::AcpConnection;
use super::lifecycle::AcpConnectionLifecycle;
use super::setup::complete_authentication;

pub struct AcpPendingConnection {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) conn: ConnectionTo<Agent>,
    pub(super) session_id: SessionId,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    pub(super) active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    pub(super) workspace_root: PathBuf,
    pub(super) config: AcpAgentConfig,
    pub(super) methods: Vec<AuthMethodId>,
    pub(super) lifecycle: AcpConnectionLifecycle,
}

impl AcpPendingConnection {
    pub fn methods(&self) -> Vec<String> {
        self.methods
            .iter()
            .map(|method| method.0.to_string())
            .collect()
    }

    pub async fn authenticate(self, method_id: String) -> Result<AcpConnection, AcpError> {
        let method = self
            .methods
            .iter()
            .find(|method| method.0.as_ref() == method_id)
            .cloned()
            .ok_or_else(|| unsupported_method(&self.config, &method_id))?;
        let acp_session_id = complete_authentication(
            &self.conn,
            &self.config,
            &self.state,
            self.workspace_root.clone(),
            method,
        )
        .await?;
        Ok(self.into_ready(acp_session_id))
    }

    fn into_ready(self, acp_session_id: AcpSessionId) -> AcpConnection {
        AcpConnection {
            handle: self.handle,
            conn: self.conn,
            acp_session_id,
            session_id: self.session_id,
            events_tx: self.events_tx,
            state: self.state,
            active_turn: self.active_turn,
            prompt_timeout: self.config.timeouts.prompt,
            agent_id: self.config.id.to_string(),
            agent_name: self.config.name.to_string(),
            _lifecycle: self.lifecycle,
        }
    }
}

fn unsupported_method(config: &AcpAgentConfig, method_id: &str) -> AcpError {
    AcpError::new(
        AcpErrorKind::UnsupportedAuthMethod,
        config.id.to_string(),
        config.name.to_string(),
        t!("AgentUi.acp_unsupported_auth_method", method = method_id).to_string(),
    )
}
