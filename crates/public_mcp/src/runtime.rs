use crate::approval::PublicMcpApprovalManager;
use crate::discovery::{
    DiscoveryDocument, PublicMcpMode, public_mcp_discovery_path, remove_discovery, write_discovery,
};
use crate::permissions::PermissionMode;
use crate::protocol::PublicMcpServer;
use crate::registry::PublicMcpRegistry;
use crate::server::LoopbackMcpServer;
use crate::tools::PublicMcpToolRegistry;
use anyhow::Result;
use rand::RngCore;
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMcpState {
    Disabled,
    Starting,
    Running {
        bind_addr: SocketAddr,
        mode: PublicMcpMode,
        exposed_sessions: usize,
        client_count: usize,
        idle_timeout_secs: Option<u64>,
    },
    Stopping,
    Failed {
        message: String,
    },
}

pub struct PublicMcpRuntime {
    server: LoopbackMcpServer,
    discovery_path: PathBuf,
}

impl PublicMcpRuntime {
    pub async fn start_terminal_mcp(
        registry: PublicMcpRegistry,
        mode: PublicMcpMode,
        permission_mode: PermissionMode,
    ) -> Result<Self> {
        Self::start_with_discovery_path(
            registry,
            mode,
            permission_mode,
            public_mcp_discovery_path(),
        )
        .await
    }

    pub async fn start_with_discovery_path(
        registry: PublicMcpRegistry,
        mode: PublicMcpMode,
        permission_mode: PermissionMode,
        discovery_path: PathBuf,
    ) -> Result<Self> {
        let tool_registry = PublicMcpToolRegistry::terminal(registry);
        Self::start_with_tool_registry(tool_registry, mode, permission_mode, discovery_path).await
    }

    pub async fn start_with_tool_registry(
        tool_registry: PublicMcpToolRegistry,
        mode: PublicMcpMode,
        permission_mode: PermissionMode,
        discovery_path: PathBuf,
    ) -> Result<Self> {
        Self::start_with_tool_registry_and_approval(
            tool_registry,
            mode,
            permission_mode,
            discovery_path,
            PublicMcpApprovalManager::default(),
        )
        .await
    }

    pub async fn start_with_tool_registry_and_approval(
        tool_registry: PublicMcpToolRegistry,
        mode: PublicMcpMode,
        permission_mode: PermissionMode,
        discovery_path: PathBuf,
        approval_manager: PublicMcpApprovalManager,
    ) -> Result<Self> {
        let _ = remove_discovery(&discovery_path);
        let token = generate_token();
        let protocol = PublicMcpServer::with_tool_registry_and_approval(
            tool_registry,
            permission_mode,
            approval_manager,
        );
        let server = LoopbackMcpServer::bind(protocol, token.clone()).await?;
        let document = DiscoveryDocument::new(std::process::id(), server.bind_addr(), token, mode);
        write_discovery(&discovery_path, &document)?;
        Ok(Self {
            server,
            discovery_path,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.server.bind_addr()
    }

    pub fn discovery_path(&self) -> &PathBuf {
        &self.discovery_path
    }

    pub fn client_count(&self) -> usize {
        self.server.client_count()
    }
}

impl Drop for PublicMcpRuntime {
    fn drop(&mut self) {
        let _ = remove_discovery(&self.discovery_path);
    }
}

fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
