pub(crate) mod exec_capture;
pub(crate) mod exec_supervisor;
pub mod history;
pub mod ingress_queue;
mod local_shell;
pub mod osc;
pub mod performance_metrics;
pub mod pty_backend;
pub mod recording;
pub mod serial_backend;
mod serial_ingress;
pub mod shell_integration;
pub mod ssh_backend;
mod ssh_ingress;
mod ssh_session_identity;
mod ssh_session_installation;
pub mod terminal;
pub mod types;
#[cfg(any(test, target_os = "windows"))]
mod windows_shell_integration;

pub use exec_supervisor::TerminalExecError;
pub use local_shell::{
    local_config_from_custom_profile, local_config_from_settings,
    local_config_from_settings_with_profile,
};
pub use performance_metrics::{
    TerminalActivity, TerminalInputMetricSource, TerminalPerformanceMetrics,
    TerminalPerformanceSnapshot, TerminalPerformanceWindow,
};
pub use pty_backend::{GpuiEventProxy, TerminalEvent};
pub use serial_backend::SerialBackend;
pub use ssh_backend::SshBackend;
pub use ssh_session_identity::{
    PersistedSshSessionIdentity, PersistedSshSessionIdentityError, SshSessionIdentityTransition,
};
pub use terminal::{TerminalScrollProxy, TerminalTextSnapshot};
pub use types::{
    LocalConfig, TerminalBackend, TerminalControlAction, TerminalControlError,
    TerminalControlHandle, TerminalControlOutput, TerminalControlReadiness, TerminalControlRequest,
    TerminalExecCompletion, TerminalExecHandle, TerminalExecObserver, TerminalExecOutput,
    TerminalExecProgress, TerminalExecRequest, TerminalInputHandle, TerminalSize,
};

#[cfg(test)]
mod ingress_queue_tests;
#[cfg(test)]
mod performance_metrics_tests;
#[cfg(test)]
mod serial_ingress_tests;
#[cfg(test)]
mod ssh_ingress_tests;
