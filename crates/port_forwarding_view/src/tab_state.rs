use std::net::SocketAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopFailure {
    NotRunning,
    TunnelMayStillBeRunning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortForwardingTabState {
    Starting,
    Running {
        local_addr: SocketAddr,
    },
    Stopping,
    Failed {
        error: String,
        stop_failure: StopFailure,
    },
    Stopped,
}

impl PortForwardingTabState {
    pub fn starting() -> Self {
        Self::Starting
    }

    pub fn started(self, local_addr: SocketAddr) -> Self {
        Self::Running { local_addr }
    }

    pub fn start_failed(self, error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
            stop_failure: StopFailure::NotRunning,
        }
    }

    pub fn retry(self) -> Self {
        Self::Starting
    }

    pub fn begin_stop(self) -> Self {
        Self::Stopping
    }

    pub fn stop_failed(self, error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
            stop_failure: StopFailure::TunnelMayStillBeRunning,
        }
    }

    pub fn stop_succeeded(self) -> Self {
        Self::Stopped
    }

    pub fn can_close_without_prompt(&self) -> bool {
        matches!(
            self,
            Self::Stopped
                | Self::Failed {
                    stop_failure: StopFailure::NotRunning,
                    ..
                }
        )
    }

    pub fn can_retry_start(&self) -> bool {
        matches!(
            self,
            Self::Failed {
                stop_failure: StopFailure::NotRunning,
                ..
            }
        )
    }

    pub fn tunnel_may_be_running(&self) -> bool {
        matches!(
            self,
            Self::Failed {
                stop_failure: StopFailure::TunnelMayStillBeRunning,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{PortForwardingTabState, StopFailure};

    #[test]
    fn successful_start_records_running_address() {
        let state = PortForwardingTabState::starting().started("127.0.0.1:9000".parse().unwrap());

        assert_eq!(
            PortForwardingTabState::Running {
                local_addr: "127.0.0.1:9000".parse().unwrap()
            },
            state
        );
    }

    #[test]
    fn cancelling_close_keeps_running_state() {
        let state = PortForwardingTabState::Running {
            local_addr: "127.0.0.1:9000".parse().unwrap(),
        };

        let after_cancel = state.clone();

        assert_eq!(state, after_cancel);
    }

    #[test]
    fn stop_failure_keeps_tab_open_with_error() {
        let state = PortForwardingTabState::Running {
            local_addr: "127.0.0.1:9000".parse().unwrap(),
        }
        .begin_stop()
        .stop_failed("disconnect failed");

        assert_eq!(
            PortForwardingTabState::Failed {
                error: "disconnect failed".to_string(),
                stop_failure: StopFailure::TunnelMayStillBeRunning,
            },
            state
        );
        assert!(!state.can_close_without_prompt());
        assert!(!state.can_retry_start());
    }

    #[test]
    fn start_failure_can_retry() {
        let state = PortForwardingTabState::starting()
            .start_failed("connection refused")
            .retry();

        assert_eq!(PortForwardingTabState::Starting, state);
    }

    #[test]
    fn start_failure_allows_retry() {
        let state = PortForwardingTabState::starting().start_failed("connection refused");

        assert!(state.can_retry_start());
    }

    #[test]
    fn stopped_state_can_close_without_prompt() {
        let state = PortForwardingTabState::Stopping.stop_succeeded();

        assert_eq!(PortForwardingTabState::Stopped, state);
        assert!(state.can_close_without_prompt());
    }
}
