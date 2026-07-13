use std::sync::Arc;

use one_core::storage::{PortForwardingKind, StoredConnection};
use port_forwarding::PortForwardingRuntime;
use rust_i18n::t;

pub struct PortForwardingTabConfig {
    pub(crate) connection: StoredConnection,
    pub(crate) ssh_connection: StoredConnection,
    pub(crate) runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    pub(crate) connection_id: i64,
    pub(crate) kind: PortForwardingKind,
    pub(crate) bind_label: String,
    pub(crate) target_label: String,
    pub(crate) ssh_label: String,
}

impl PortForwardingTabConfig {
    pub fn new(
        connection: StoredConnection,
        ssh_connection: StoredConnection,
        runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    ) -> anyhow::Result<Self> {
        let connection_id = connection
            .id
            .ok_or_else(|| anyhow::anyhow!("missing connection id"))?;
        let params = connection.to_port_forwarding_params()?;
        let ssh = ssh_connection.to_ssh_params()?;
        Ok(Self {
            connection,
            ssh_connection,
            runtime,
            connection_id,
            kind: params.kind,
            bind_label: format!("{}:{}", params.bind_host, params.bind_port),
            target_label: target_label(params.kind, &params.target_host, params.target_port),
            ssh_label: format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
        })
    }
}

fn target_label(kind: PortForwardingKind, host: &str, port: u16) -> String {
    match kind {
        PortForwardingKind::Local => format!("{host}:{port}"),
        PortForwardingKind::Dynamic => t!("PortForwardingTab.dynamic_target").to_string(),
    }
}
