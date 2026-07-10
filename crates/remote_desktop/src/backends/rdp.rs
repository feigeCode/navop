use std::path::PathBuf;
use std::time::Duration;

use crate::{
    RemoteDesktopBackend, RemoteDesktopCapabilities, RemoteDesktopConnectionOptions,
    RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol, RemoteDesktopRuntime,
    RemoteDesktopSize,
    helper_protocol::{HelperEvent, HelperRequest, decode_event_line, encode_request_line},
    output_mailbox::{OutputMailboxSender, output_mailbox},
};

mod input;
#[cfg(test)]
mod tests;
mod transport;

const RDP_BACKEND_POLL_INTERVAL: Duration = Duration::from_millis(8);
const MAX_INPUTS_PER_POLL: usize = 256;

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
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (output_tx, output_rx) = output_mailbox();
        let helper = self.helper.clone();
        let mut connect = HelperRequest::connect_from_options(&self.options, initial_size);
        let protocol = self.options.protocol;

        std::thread::Builder::new()
            .name("remote-desktop-rdp".to_string())
            .spawn(move || {
                let mut latest_clipboard_text = None;
                let mut reconnect_attempt = 0usize;
                loop {
                    match run_helper_session(
                        &helper,
                        &mut connect,
                        &mut latest_clipboard_text,
                        &mut input_rx,
                        &output_tx,
                        protocol,
                    ) {
                        HelperRunResult::Closed | HelperRunResult::InputClosed => break,
                        HelperRunResult::Reconnect {
                            reason,
                            manual,
                            was_connected,
                        } => {
                            if was_connected || manual {
                                reconnect_attempt = 0;
                            }
                            if manual {
                                transport::send_status(
                                    &output_tx,
                                    "reconnecting remote desktop session",
                                );
                                continue;
                            }
                            let delay = input::reconnect_delay(reconnect_attempt);
                            reconnect_attempt = reconnect_attempt.saturating_add(1);
                            transport::send_status(
                                &output_tx,
                                &input::reconnect_status_message(&reason, delay),
                            );
                            if !input::wait_before_reconnect(
                                &mut connect,
                                &mut latest_clipboard_text,
                                &mut input_rx,
                                delay,
                            ) {
                                break;
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
    },
}

fn run_helper_session(
    helper: &HelperProcessConfig,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> HelperRunResult {
    let Ok((mut helper, mut stdin, signal_rx)) =
        transport::start_helper_session(helper, connect, latest_clipboard_text, output_tx)
    else {
        return HelperRunResult::Reconnect {
            reason: "failed to start remote desktop helper".to_string(),
            manual: false,
            was_connected: false,
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
        ) {
            return result;
        }
        let mut input_context = input::RemoteInputContext {
            connect,
            latest_clipboard_text,
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
        if let Some(result) = input::poll_helper_exit(&mut helper, was_connected) {
            return result;
        }
        std::thread::sleep(RDP_BACKEND_POLL_INTERVAL);
    }
}
