pub(crate) mod exec_capture;
pub(crate) mod exec_supervisor;
pub mod history;
mod local_shell;
pub mod osc;
pub mod pty_backend;
pub mod serial_backend;
pub mod shell_integration;
pub mod ssh_backend;
pub mod terminal;
pub mod types;

pub use exec_supervisor::TerminalExecError;
pub use local_shell::local_config_from_settings;
pub use pty_backend::{GpuiEventProxy, TerminalEvent};
pub use serial_backend::SerialBackend;
pub use ssh_backend::SshBackend;
pub use terminal::TerminalScrollProxy;
pub use types::{
    LocalConfig, TerminalBackend, TerminalExecCompletion, TerminalExecHandle, TerminalExecOutput,
    TerminalExecRequest, TerminalInputHandle, TerminalSize,
};
