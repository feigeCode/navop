use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::TerminalExecError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalControlReadiness {
    SubmissionPending,
    CommandRunning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalControlAction {
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalControlRequest {
    pub action: TerminalControlAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalControlOutput {
    pub action: TerminalControlAction,
    pub sent: bool,
    pub readiness_before: TerminalControlReadiness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalControlError {
    NotRunning,
    Busy,
    ReadinessUnknown,
    Disconnected,
    Cancelled,
}

impl std::fmt::Display for TerminalControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotRunning => "terminal_not_running",
            Self::Busy => "terminal_busy",
            Self::ReadinessUnknown => "readiness_unknown",
            Self::Disconnected => "terminal_disconnected",
            Self::Cancelled => "cancelled",
        })
    }
}

impl std::error::Error for TerminalControlError {}

#[derive(Clone)]
pub struct TerminalControlHandle {
    control_fn: Arc<
        dyn Fn(TerminalControlRequest, CancellationToken) -> TerminalControlFuture + Send + Sync,
    >,
}

pub type TerminalControlFuture = Pin<
    Box<dyn Future<Output = Result<TerminalControlOutput, TerminalControlError>> + Send + 'static>,
>;

impl TerminalControlHandle {
    pub fn new(
        control_fn: impl Fn(TerminalControlRequest, CancellationToken) -> TerminalControlFuture
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            control_fn: Arc::new(control_fn),
        }
    }

    pub async fn control(
        &self,
        request: TerminalControlRequest,
        cancellation: CancellationToken,
    ) -> Result<TerminalControlOutput, TerminalControlError> {
        (self.control_fn)(request, cancellation).await
    }
}

#[derive(Clone)]
pub struct TerminalInputHandle {
    write_fn: Arc<dyn Fn(Vec<u8>) + Send + Sync>,
}

impl TerminalInputHandle {
    pub fn new(write_fn: impl Fn(Vec<u8>) + Send + Sync + 'static) -> Self {
        Self {
            write_fn: Arc::new(write_fn),
        }
    }

    pub fn write(&self, data: impl Into<Vec<u8>>) {
        (self.write_fn)(data.into());
    }
}

#[derive(Clone)]
pub struct TerminalExecHandle {
    exec_fn:
        Arc<dyn Fn(TerminalExecRequest, CancellationToken) -> TerminalExecFuture + Send + Sync>,
}

pub type TerminalExecFuture =
    Pin<Box<dyn Future<Output = Result<TerminalExecOutput, TerminalExecError>> + Send + 'static>>;

impl TerminalExecHandle {
    pub fn new(
        exec_fn: impl Fn(TerminalExecRequest, CancellationToken) -> TerminalExecFuture
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            exec_fn: Arc::new(exec_fn),
        }
    }

    pub async fn exec(
        &self,
        request: TerminalExecRequest,
        cancellation: CancellationToken,
    ) -> Result<TerminalExecOutput, TerminalExecError> {
        (self.exec_fn)(request, cancellation).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalExecRequest {
    pub command: String,
    pub submit: bool,
    pub wait_for_output: bool,
    pub ready_timeout: Duration,
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalExecCompletion {
    ObservedOutput,
    ShellIntegrationExit,
    SubmittedOnly,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalExecOutput {
    pub completion: TerminalExecCompletion,
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
}

/// Terminal backend trait - abstracts local PTY and SSH backends
pub trait TerminalBackend: Send {
    fn write(&self, data: Vec<u8>);
    fn resize(&self, size: TerminalSize);
    fn shutdown(&self);

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        None
    }

    fn exec_handle(&self) -> Option<TerminalExecHandle> {
        None
    }

    fn control_handle(&self) -> Option<TerminalControlHandle> {
        None
    }
}

/// Local terminal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Shell command (default: system default shell)
    pub shell: Option<String>,
    /// Arguments passed directly to the shell process.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory
    pub working_dir: Option<String>,
    /// Environment variables
    pub env: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalControlAction, TerminalControlHandle, TerminalControlOutput,
        TerminalControlReadiness, TerminalControlRequest, TerminalExecCompletion,
        TerminalExecHandle, TerminalExecOutput, TerminalExecRequest, TerminalInputHandle,
    };
    use crate::TerminalExecError;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn terminal_input_handle_forwards_bytes_to_writer() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        let handle = TerminalInputHandle::new(move |bytes| {
            sink.lock().expect("written lock").push(bytes);
        });

        handle.write(b"df -h\n".to_vec());

        assert_eq!(vec![b"df -h\n".to_vec()], *written.lock().unwrap());
    }

    #[tokio::test]
    async fn terminal_exec_handle_delegates_to_executor() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let sink = requests.clone();
        let handle = TerminalExecHandle::new(move |request, _cancellation| {
            let sink = sink.clone();
            Box::pin(async move {
                sink.lock().expect("requests lock").push(request.command);
                Ok(TerminalExecOutput {
                    completion: TerminalExecCompletion::SubmittedOnly,
                    exit_code: None,
                    output: String::new(),
                    duration_ms: 0,
                })
            })
        });

        let result = handle
            .exec(
                TerminalExecRequest {
                    command: "df -h".to_string(),
                    submit: true,
                    wait_for_output: false,
                    ready_timeout: Duration::ZERO,
                    timeout: Duration::from_millis(1),
                },
                CancellationToken::new(),
            )
            .await
            .expect("exec handle should delegate");

        assert_eq!(TerminalExecCompletion::SubmittedOnly, result.completion);
        assert_eq!(vec!["df -h".to_string()], *requests.lock().unwrap());
    }

    #[tokio::test]
    async fn terminal_exec_handle_forwards_cancellation() {
        let handle = TerminalExecHandle::new(|_request, cancellation| {
            Box::pin(async move {
                cancellation.cancelled().await;
                Err(TerminalExecError::CancelledBeforeSubmit)
            })
        });
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = handle
            .exec(
                TerminalExecRequest {
                    command: "sleep 300".to_string(),
                    submit: true,
                    wait_for_output: true,
                    ready_timeout: Duration::ZERO,
                    timeout: Duration::from_secs(30),
                },
                cancellation,
            )
            .await
            .expect_err("cancelled execution should fail before submission");

        assert_eq!(TerminalExecError::CancelledBeforeSubmit, error);
    }

    #[tokio::test]
    async fn terminal_control_handle_delegates_to_controller() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let sink = requests.clone();
        let handle = TerminalControlHandle::new(move |request, _cancellation| {
            let sink = sink.clone();
            Box::pin(async move {
                sink.lock().expect("requests lock").push(request.action);
                Ok(TerminalControlOutput {
                    action: request.action,
                    sent: true,
                    readiness_before: TerminalControlReadiness::CommandRunning,
                })
            })
        });

        let result = handle
            .control(
                TerminalControlRequest {
                    action: TerminalControlAction::Interrupt,
                },
                CancellationToken::new(),
            )
            .await
            .expect("control handle should delegate");

        assert!(result.sent);
        assert_eq!(
            vec![TerminalControlAction::Interrupt],
            *requests.lock().unwrap()
        );
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            working_dir: None,
            env: vec![
                ("TERM".to_string(), "xterm-256color".to_string()),
                ("COLORTERM".to_string(), "truecolor".to_string()),
                ("CLICOLOR".to_string(), "1".to_string()),
                ("CLICOLOR_FORCE".to_string(), "1".to_string()),
            ],
        }
    }
}

/// Terminal dimensions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}
