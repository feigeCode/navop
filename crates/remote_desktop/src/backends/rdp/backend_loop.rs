use std::sync::mpsc::Sender;

use tokio::sync::mpsc::UnboundedReceiver;

use super::{
    ClipboardFilesSnapshot, HelperDisconnectKind, HelperProcessConfig, HelperRunResult,
    REMOTE_DESKTOP_BACKEND_POLL_INTERVAL, input, reconnect, transport,
};
use crate::{
    RemoteDesktopFailure, RemoteDesktopInput, RemoteDesktopProtocol, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
    connection_test::{RemoteDesktopConnectionDiagnostic, send_connection_diagnostic},
    helper_protocol::HelperRequest,
    output_mailbox::OutputMailboxSender,
};

struct BackendLoop {
    helper: HelperProcessConfig,
    connect: HelperRequest,
    latest_clipboard_text: Option<String>,
    latest_clipboard_files: Option<ClipboardFilesSnapshot>,
    input_rx: UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
    diagnostic_tx: Option<Sender<RemoteDesktopConnectionDiagnostic>>,
    reconnect_attempt: usize,
}

pub(super) fn run(
    helper: HelperProcessConfig,
    connect: HelperRequest,
    input_rx: UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
    diagnostic_tx: Option<Sender<RemoteDesktopConnectionDiagnostic>>,
) {
    BackendLoop {
        helper,
        connect,
        latest_clipboard_text: None,
        latest_clipboard_files: None,
        input_rx,
        output_tx,
        protocol,
        diagnostic_tx,
        reconnect_attempt: 0,
    }
    .run();
}

impl BackendLoop {
    fn run(&mut self) {
        loop {
            let session_output_tx = self.output_tx.begin_session();
            let result = self.run_helper_session(&session_output_tx);
            // The stdout reader is detached, so end its generation before
            // reconnecting to prevent late output reaching the next session.
            session_output_tx.end_session();
            if !self.handle_result(result) {
                break;
            }
        }
    }

    fn run_helper_session(&mut self, output_tx: &OutputMailboxSender) -> HelperRunResult {
        let Ok((helper, stdin, signal_rx)) = transport::start_helper_session(
            &self.helper,
            &self.connect,
            &self.latest_clipboard_text,
            &self.latest_clipboard_files,
            output_tx,
            self.protocol,
        ) else {
            return helper_start_failure();
        };
        poll_helper_session(self, helper, stdin, signal_rx, output_tx)
    }

    fn handle_result(&mut self, result: HelperRunResult) -> bool {
        match result {
            HelperRunResult::Closed | HelperRunResult::InputClosed => false,
            HelperRunResult::Reconnect { manual: true, .. } => {
                self.reconnect_attempt = 0;
                transport::send_reconnecting(
                    &self.output_tx,
                    RemoteDesktopReconnect {
                        reason: RemoteDesktopReconnectReason::Manual,
                        delay_secs: None,
                    },
                );
                true
            }
            HelperRunResult::Reconnect {
                reason,
                was_connected,
                disconnect_kind,
                ..
            } => self.handle_disconnect(reason, was_connected, disconnect_kind),
        }
    }

    fn handle_disconnect(
        &mut self,
        reason: String,
        was_connected: bool,
        disconnect_kind: Option<HelperDisconnectKind>,
    ) -> bool {
        match reconnect::reconnect_decision(&reason, was_connected, disconnect_kind) {
            reconnect::ReconnectDecision::Retry(reconnect_reason) => {
                self.retry(was_connected, reconnect_reason)
            }
            reconnect::ReconnectDecision::ConnectionFailure(failure) => {
                self.finish(reason, failure, false);
                false
            }
            reconnect::ReconnectDecision::Terminated(failure) => {
                self.finish(reason, failure, true);
                false
            }
        }
    }

    fn retry(&mut self, was_connected: bool, reason: RemoteDesktopReconnectReason) -> bool {
        if was_connected {
            self.reconnect_attempt = 0;
        }
        let delay = reconnect::reconnect_delay(self.reconnect_attempt);
        self.reconnect_attempt = self.reconnect_attempt.saturating_add(1);
        transport::send_reconnecting(&self.output_tx, reconnect::reconnect_event(reason, delay));
        reconnect::wait_before_reconnect(
            &mut self.connect,
            &mut self.latest_clipboard_text,
            &mut self.latest_clipboard_files,
            &mut self.input_rx,
            delay,
            self.protocol,
        )
    }

    fn finish(&self, reason: String, failure: RemoteDesktopFailure, terminated: bool) {
        if terminated {
            tracing::warn!(
                error = %reason,
                ?failure,
                "remote desktop session terminated without reconnect"
            );
        } else {
            tracing::warn!(
                error = %reason,
                ?failure,
                "remote desktop connection failed without reconnect"
            );
        }
        send_connection_diagnostic(&self.diagnostic_tx, failure.clone(), reason);
        if terminated {
            transport::send_terminated(&self.output_tx, failure);
        } else {
            transport::send_failure(&self.output_tx, failure);
        }
    }
}

fn helper_start_failure() -> HelperRunResult {
    HelperRunResult::Reconnect {
        reason: "failed to start remote desktop helper".to_string(),
        manual: false,
        was_connected: false,
        disconnect_kind: None,
    }
}

fn poll_helper_session(
    backend: &mut BackendLoop,
    mut helper: std::process::Child,
    mut stdin: std::process::ChildStdin,
    signal_rx: std::sync::mpsc::Receiver<super::transport::BackendSignal>,
    output_tx: &OutputMailboxSender,
) -> HelperRunResult {
    let mut was_connected = false;
    loop {
        if let Some(result) = input::handle_backend_signals(
            &signal_rx,
            &mut helper,
            &mut stdin,
            output_tx,
            &mut was_connected,
            backend.protocol,
            &backend.diagnostic_tx,
        ) {
            return result;
        }
        let mut input_context = input::RemoteInputContext {
            connect: &mut backend.connect,
            latest_clipboard_text: &mut backend.latest_clipboard_text,
            latest_clipboard_files: &mut backend.latest_clipboard_files,
            helper: &mut helper,
            stdin: &mut stdin,
            output_tx,
            protocol: backend.protocol,
        };
        if let Some(result) =
            input::handle_remote_input(&mut backend.input_rx, &mut input_context, was_connected)
        {
            return result;
        }
        if let Some(result) = input::poll_helper_exit(&mut helper, was_connected, backend.protocol)
        {
            return result;
        }
        std::thread::sleep(REMOTE_DESKTOP_BACKEND_POLL_INTERVAL);
    }
}
