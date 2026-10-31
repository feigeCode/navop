use public_mcp::discovery::PublicMcpMode;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicMcpRuntimeStatus {
    Disabled,
    Starting {
        generation: u64,
    },
    Running {
        generation: u64,
        bind_addr: SocketAddr,
        mode: PublicMcpMode,
        discovery_path: PathBuf,
        client_count: usize,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

impl Default for PublicMcpRuntimeStatus {
    fn default() -> Self {
        Self::Disabled
    }
}

impl PublicMcpRuntimeStatus {
    pub fn disabled(self) -> Self {
        Self::Disabled
    }

    pub fn starting(self, generation: u64) -> Self {
        Self::Starting { generation }
    }

    pub fn running(
        self,
        generation: u64,
        bind_addr: SocketAddr,
        mode: PublicMcpMode,
        discovery_path: PathBuf,
        client_count: usize,
    ) -> Self {
        Self::Running {
            generation,
            bind_addr,
            mode,
            discovery_path,
            client_count,
        }
    }

    pub fn with_client_count(self, client_count: usize) -> Self {
        match self {
            Self::Running {
                generation,
                bind_addr,
                mode,
                discovery_path,
                ..
            } => Self::Running {
                generation,
                bind_addr,
                mode,
                discovery_path,
                client_count,
            },
            status => status,
        }
    }

    #[cfg(test)]
    pub fn failed(self, generation: u64, message: impl Into<String>) -> Self {
        Self::Failed {
            generation,
            message: message.into(),
        }
    }

    pub fn try_set_failed(&mut self, generation: u64, message: impl Into<String>) {
        if self.generation() == Some(generation) {
            *self = Self::Failed {
                generation,
                message: message.into(),
            };
        }
    }

    #[cfg(test)]
    pub fn state_id(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Starting { .. } => "starting",
            Self::Running { .. } => "running",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::Disabled => None,
            Self::Starting { generation }
            | Self::Running { generation, .. }
            | Self::Failed { generation, .. } => Some(*generation),
        }
    }

    #[cfg(test)]
    pub fn summary(&self) -> String {
        match self {
            Self::Disabled => "MCP server is disabled".to_string(),
            Self::Starting { generation } => format!("MCP server is starting ({generation})"),
            Self::Running {
                bind_addr,
                mode,
                discovery_path,
                client_count,
                ..
            } => format!(
                "MCP server is running at {bind_addr}; mode={mode:?}; clients={client_count}; discovery={}",
                discovery_path.display()
            ),
            Self::Failed { message, .. } => format!("MCP server failed: {message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::discovery::PublicMcpMode;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;

    #[test]
    fn runtime_status_moves_through_starting_running_failed_and_disabled() {
        let mut status = PublicMcpRuntimeStatus::Disabled;

        status = status.starting(7);
        assert_eq!("starting", status.state_id());
        assert_eq!(Some(7), status.generation());

        status = status.running(
            7,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 38291),
            PublicMcpMode::Persistent,
            PathBuf::from("/tmp/public-mcp.json"),
            2,
        );
        assert_eq!("running", status.state_id());
        assert_eq!(Some(7), status.generation());
        assert!(status.summary().contains("127.0.0.1:38291"));
        assert!(status.summary().contains("clients=2"));
        assert!(status.summary().contains("/tmp/public-mcp.json"));

        status = status.failed(8, "bind failed");
        assert_eq!("failed", status.state_id());
        assert_eq!(Some(8), status.generation());
        assert!(status.summary().contains("bind failed"));

        status = status.disabled();
        assert_eq!("disabled", status.state_id());
        assert_eq!(None, status.generation());
    }

    #[test]
    fn stale_runtime_status_events_do_not_replace_newer_generation() {
        let mut status = PublicMcpRuntimeStatus::Disabled.starting(10);

        status.try_set_failed(9, "old task failed");

        assert_eq!("starting", status.state_id());
        assert_eq!(Some(10), status.generation());
    }
}
