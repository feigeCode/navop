use std::fmt;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopFailure, RemoteDesktopInput,
    RemoteDesktopProviderRegistry, RemoteDesktopRuntime, RemoteDesktopSize,
};

const CONNECTION_TEST_POLL_INTERVAL: Duration = Duration::from_millis(8);
const CONNECTION_TEST_SIZE: RemoteDesktopSize = RemoteDesktopSize {
    width: 1024,
    height: 768,
    scale_factor: 100,
};

#[derive(Clone, PartialEq, Eq)]
pub struct RemoteDesktopConnectionTestFailure {
    pub failure: RemoteDesktopFailure,
    pub reason: String,
}

impl fmt::Debug for RemoteDesktopConnectionTestFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteDesktopConnectionTestFailure")
            .field("failure", &self.failure)
            .field("reason_present", &!self.reason.is_empty())
            .field("reason_len", &self.reason.len())
            .finish()
    }
}

impl fmt::Display for RemoteDesktopConnectionTestFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for RemoteDesktopConnectionTestFailure {}

#[derive(Clone)]
pub(crate) enum RemoteDesktopConnectionDiagnostic {
    Connected,
    Failure {
        failure: RemoteDesktopFailure,
        reason: String,
    },
}

pub fn test_connection(
    options: RemoteDesktopConnectionOptions,
    timeout: Duration,
) -> Result<(), RemoteDesktopConnectionTestFailure> {
    let (diagnostic_tx, diagnostic_rx) = std::sync::mpsc::channel();
    let registry = RemoteDesktopProviderRegistry::load_default();
    let backend = crate::backend::create_backend_with_registry_and_diagnostics(
        options,
        &registry,
        diagnostic_tx,
    )
    .map_err(start_failure)?;
    let runtime = backend.start(CONNECTION_TEST_SIZE).map_err(start_failure)?;
    wait_for_connection(runtime, diagnostic_rx, timeout)
}

pub(crate) fn send_connection_diagnostic(
    sender: &Option<Sender<RemoteDesktopConnectionDiagnostic>>,
    failure: RemoteDesktopFailure,
    reason: String,
) {
    if let Some(sender) = sender {
        let _ = sender.send(RemoteDesktopConnectionDiagnostic::Failure { failure, reason });
    }
}

pub(crate) fn send_connection_ready(sender: &Option<Sender<RemoteDesktopConnectionDiagnostic>>) {
    if let Some(sender) = sender {
        let _ = sender.send(RemoteDesktopConnectionDiagnostic::Connected);
    }
}

fn wait_for_connection(
    runtime: RemoteDesktopRuntime,
    diagnostic_rx: Receiver<RemoteDesktopConnectionDiagnostic>,
    timeout: Duration,
) -> Result<(), RemoteDesktopConnectionTestFailure> {
    let _close = CloseRuntime(runtime.input_tx.clone());
    let started = Instant::now();
    loop {
        if let Some(result) = poll_connection_test(&diagnostic_rx) {
            return result;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Err(timeout_failure(timeout));
        }
        std::thread::sleep(CONNECTION_TEST_POLL_INTERVAL.min(timeout - elapsed));
    }
}

fn poll_connection_test(
    diagnostic_rx: &Receiver<RemoteDesktopConnectionDiagnostic>,
) -> Option<Result<(), RemoteDesktopConnectionTestFailure>> {
    match diagnostic_rx.try_recv() {
        Ok(RemoteDesktopConnectionDiagnostic::Connected) => Some(Ok(())),
        Ok(RemoteDesktopConnectionDiagnostic::Failure { failure, reason }) => {
            Some(Err(RemoteDesktopConnectionTestFailure { failure, reason }))
        }
        Err(TryRecvError::Disconnected | TryRecvError::Empty) => None,
    }
}

fn start_failure(error: anyhow::Error) -> RemoteDesktopConnectionTestFailure {
    let failure = error
        .downcast_ref::<crate::RemoteDesktopProviderVersionError>()
        .map(|error| RemoteDesktopFailure::ProviderVersion {
            protocol: error.protocol,
            installed: error.installed.clone(),
            required: error.required.clone(),
            invalid: error.invalid,
        })
        .unwrap_or(RemoteDesktopFailure::ConnectionFailed);
    RemoteDesktopConnectionTestFailure {
        failure,
        reason: format!("{error:#}"),
    }
}

fn timeout_failure(timeout: Duration) -> RemoteDesktopConnectionTestFailure {
    RemoteDesktopConnectionTestFailure {
        failure: RemoteDesktopFailure::ConnectionFailed,
        reason: format!(
            "remote desktop connection test timed out after {:.1} seconds",
            timeout.as_secs_f64()
        ),
    }
}

struct CloseRuntime(tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>);

impl Drop for CloseRuntime {
    fn drop(&mut self) {
        let _ = self.0.send(RemoteDesktopInput::Close);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::{RemoteDesktopFailure, RemoteDesktopInput, RemoteDesktopRuntime};

    use super::{
        RemoteDesktopConnectionDiagnostic, RemoteDesktopConnectionTestFailure, wait_for_connection,
    };

    fn test_runtime() -> (
        RemoteDesktopRuntime,
        tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    ) {
        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_output_tx, output_rx) = crate::output_mailbox::output_mailbox();
        (
            RemoteDesktopRuntime {
                input_tx,
                output_rx,
            },
            input_rx,
        )
    }

    #[test]
    fn connected_diagnostic_completes_test_and_closes_runtime() {
        let (runtime, mut input_rx) = test_runtime();
        let (diagnostic_tx, diagnostic_rx) = mpsc::channel();
        diagnostic_tx
            .send(RemoteDesktopConnectionDiagnostic::Connected)
            .unwrap();

        assert!(wait_for_connection(runtime, diagnostic_rx, Duration::ZERO).is_ok());
        assert!(matches!(input_rx.try_recv(), Ok(RemoteDesktopInput::Close)));
    }

    #[test]
    fn diagnostic_preserves_server_reason_and_closes_runtime() {
        let (runtime, mut input_rx) = test_runtime();
        let (diagnostic_tx, diagnostic_rx) = mpsc::channel();
        diagnostic_tx
            .send(RemoteDesktopConnectionDiagnostic::Failure {
                failure: RemoteDesktopFailure::AuthenticationFailed,
                reason: "CredSSP rejected these credentials".to_string(),
            })
            .unwrap();

        let error = wait_for_connection(runtime, diagnostic_rx, Duration::ZERO).unwrap_err();

        assert!(error.failure == RemoteDesktopFailure::AuthenticationFailed);
        assert_eq!("CredSSP rejected these credentials", error.reason);
        assert!(matches!(input_rx.try_recv(), Ok(RemoteDesktopInput::Close)));
    }

    #[test]
    fn connected_event_wins_over_a_later_disconnect_diagnostic() {
        let (runtime, mut input_rx) = test_runtime();
        let (diagnostic_tx, diagnostic_rx) = mpsc::channel();
        diagnostic_tx
            .send(RemoteDesktopConnectionDiagnostic::Connected)
            .unwrap();
        diagnostic_tx
            .send(RemoteDesktopConnectionDiagnostic::Failure {
                failure: RemoteDesktopFailure::ConnectionFailed,
                reason: "server disconnected after the desktop became ready".to_string(),
            })
            .unwrap();

        assert!(wait_for_connection(runtime, diagnostic_rx, Duration::ZERO).is_ok());
        assert!(matches!(input_rx.try_recv(), Ok(RemoteDesktopInput::Close)));
    }

    #[test]
    fn timeout_closes_runtime() {
        let (runtime, mut input_rx) = test_runtime();
        let (_diagnostic_tx, diagnostic_rx) = mpsc::channel();

        let error = wait_for_connection(runtime, diagnostic_rx, Duration::ZERO).unwrap_err();

        assert!(error.failure == RemoteDesktopFailure::ConnectionFailed);
        assert!(error.reason.contains("timed out"));
        assert!(matches!(input_rx.try_recv(), Ok(RemoteDesktopInput::Close)));
    }

    #[test]
    fn failure_debug_redacts_raw_reason() {
        let failure = RemoteDesktopConnectionTestFailure {
            failure: RemoteDesktopFailure::AuthenticationFailed,
            reason: "private CredSSP server diagnostic".to_string(),
        };

        let debug = format!("{failure:?}");

        assert!(debug.contains("AuthenticationFailed"));
        assert!(!debug.contains("private"));
        assert!(!debug.contains("CredSSP"));
    }
}
