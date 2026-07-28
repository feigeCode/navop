use std::path::PathBuf;
use std::time::Duration;

use crate::{
    RemoteDesktopBackend, RemoteDesktopCapabilities, RemoteDesktopConnectionOptions,
    RemoteDesktopFailure, RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol,
    RemoteDesktopReconnect, RemoteDesktopReconnectReason, RemoteDesktopRuntime, RemoteDesktopSize,
    helper_protocol::{HelperRequest, encode_request_line},
    output_mailbox::{OutputMailboxSender, output_mailbox},
};

mod helper_events;
mod input;
mod reconnect;
mod transport;
mod transport_frames;

const REMOTE_DESKTOP_BACKEND_POLL_INTERVAL: Duration = Duration::from_millis(8);
const MAX_INPUTS_PER_POLL: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClipboardFilesSnapshot {
    pub(super) transfer_id: u64,
    pub(super) paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HelperDisconnectKind {
    ConnectionFailure,
    Terminated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HelperDisconnect {
    pub(super) kind: Option<HelperDisconnectKind>,
    pub(super) reason: String,
}

pub struct RdpBackend {
    options: RemoteDesktopConnectionOptions,
    helper: HelperProcessConfig,
}

impl RdpBackend {
    pub fn new_with_helper(
        options: RemoteDesktopConnectionOptions,
        helper: HelperProcessConfig,
    ) -> Self {
        Self { options, helper }
    }
}

#[derive(Clone, Debug)]
pub struct HelperProcessConfig {
    command: PathBuf,
    args: Vec<String>,
    working_dir: PathBuf,
}

impl HelperProcessConfig {
    pub fn new(command: PathBuf, args: Vec<String>, working_dir: PathBuf) -> Self {
        Self {
            command,
            args,
            working_dir,
        }
    }
}

impl RemoteDesktopBackend for RdpBackend {
    fn name(&self) -> &'static str {
        "remote-desktop-helper"
    }

    fn start(
        self: Box<Self>,
        initial_size: RemoteDesktopSize,
    ) -> anyhow::Result<RemoteDesktopRuntime> {
        let (options, proxy_guard) = crate::backend::resolve_proxy_options(self.options.clone())?;
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (output_tx, output_rx) = output_mailbox();
        let helper = self.helper.clone();
        let mut connect = HelperRequest::connect_from_options(&options, initial_size);
        let protocol = options.protocol;

        std::thread::Builder::new()
            .name(format!("remote-desktop-{}", protocol.provider_id()))
            .spawn(move || {
                let _proxy_guard = proxy_guard;
                let mut latest_clipboard_text = None;
                let mut latest_clipboard_files = None;
                let mut reconnect_attempt = 0usize;
                loop {
                    let session_output_tx = output_tx.begin_session();
                    let result = run_helper_session(
                        &helper,
                        &mut connect,
                        &mut latest_clipboard_text,
                        &mut latest_clipboard_files,
                        &mut input_rx,
                        &session_output_tx,
                        protocol,
                    );
                    // The helper stdout reader is intentionally detached. Cut
                    // off its generation before publishing a reconnect barrier
                    // or starting the next process so late output cannot reset
                    // or resize the new session.
                    session_output_tx.end_session();
                    match result {
                        HelperRunResult::Closed | HelperRunResult::InputClosed => break,
                        HelperRunResult::Reconnect {
                            reason,
                            manual,
                            was_connected,
                            disconnect_kind,
                        } => {
                            if manual {
                                reconnect_attempt = 0;
                                transport::send_reconnecting(
                                    &output_tx,
                                    RemoteDesktopReconnect {
                                        reason: RemoteDesktopReconnectReason::Manual,
                                        delay_secs: None,
                                    },
                                );
                                continue;
                            }
                            match reconnect::reconnect_decision(
                                &reason,
                                was_connected,
                                disconnect_kind,
                            ) {
                                reconnect::ReconnectDecision::Retry(reconnect_reason) => {
                                    if was_connected {
                                        reconnect_attempt = 0;
                                    }
                                    let delay = reconnect::reconnect_delay(reconnect_attempt);
                                    reconnect_attempt = reconnect_attempt.saturating_add(1);
                                    transport::send_reconnecting(
                                        &output_tx,
                                        reconnect::reconnect_event(reconnect_reason, delay),
                                    );
                                    if !reconnect::wait_before_reconnect(
                                        &mut connect,
                                        &mut latest_clipboard_text,
                                        &mut latest_clipboard_files,
                                        &mut input_rx,
                                        delay,
                                        protocol,
                                    ) {
                                        break;
                                    }
                                }
                                reconnect::ReconnectDecision::ConnectionFailure(failure) => {
                                    tracing::warn!(
                                        error = %reason,
                                        ?failure,
                                        "remote desktop connection failed without reconnect"
                                    );
                                    transport::send_failure(&output_tx, failure);
                                    break;
                                }
                                reconnect::ReconnectDecision::Terminated(failure) => {
                                    tracing::warn!(
                                        error = %reason,
                                        ?failure,
                                        "remote desktop session terminated without reconnect"
                                    );
                                    transport::send_terminated(&output_tx, failure);
                                    break;
                                }
                            }
                        }
                    }
                }
            })?;

        Ok(RemoteDesktopRuntime {
            input_tx,
            output_rx,
        })
    }
}

pub(super) enum HelperRunResult {
    Closed,
    InputClosed,
    Reconnect {
        reason: String,
        manual: bool,
        was_connected: bool,
        disconnect_kind: Option<HelperDisconnectKind>,
    },
}

fn run_helper_session(
    helper: &HelperProcessConfig,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    latest_clipboard_files: &mut Option<ClipboardFilesSnapshot>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> HelperRunResult {
    let Ok((mut helper, mut stdin, signal_rx)) = transport::start_helper_session(
        helper,
        connect,
        latest_clipboard_text,
        latest_clipboard_files,
        output_tx,
        protocol,
    ) else {
        return HelperRunResult::Reconnect {
            reason: "failed to start remote desktop helper".to_string(),
            manual: false,
            was_connected: false,
            disconnect_kind: None,
        };
    };

    let mut was_connected = false;
    loop {
        if let Some(result) = input::handle_backend_signals(
            &signal_rx,
            &mut helper,
            &mut stdin,
            output_tx,
            &mut was_connected,
            protocol,
        ) {
            return result;
        }
        let mut input_context = input::RemoteInputContext {
            connect,
            latest_clipboard_text,
            latest_clipboard_files,
            helper: &mut helper,
            stdin: &mut stdin,
            output_tx,
            protocol,
        };
        if let Some(result) =
            input::handle_remote_input(input_rx, &mut input_context, was_connected)
        {
            return result;
        }
        if let Some(result) = input::poll_helper_exit(&mut helper, was_connected, protocol) {
            return result;
        }
        std::thread::sleep(REMOTE_DESKTOP_BACKEND_POLL_INTERVAL);
    }
}
