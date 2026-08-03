use std::path::PathBuf;
use std::time::Duration;

use crate::connection_test::{RemoteDesktopConnectionDiagnostic, send_connection_ready};
use crate::{
    RemoteDesktopBackend, RemoteDesktopCapabilities, RemoteDesktopConnectionOptions,
    RemoteDesktopFailure, RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol,
    RemoteDesktopReconnect, RemoteDesktopReconnectReason, RemoteDesktopRuntime, RemoteDesktopSize,
    helper_protocol::{HelperRequest, encode_request_line},
    output_mailbox::{OutputMailboxSender, output_mailbox},
};

mod backend_loop;
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
    diagnostic_tx: Option<std::sync::mpsc::Sender<RemoteDesktopConnectionDiagnostic>>,
}

impl RdpBackend {
    pub fn new_with_helper(
        options: RemoteDesktopConnectionOptions,
        helper: HelperProcessConfig,
    ) -> Self {
        Self::new_with_helper_and_diagnostics(options, helper, None)
    }

    pub(crate) fn new_with_helper_and_diagnostics(
        options: RemoteDesktopConnectionOptions,
        helper: HelperProcessConfig,
        diagnostic_tx: Option<std::sync::mpsc::Sender<RemoteDesktopConnectionDiagnostic>>,
    ) -> Self {
        Self {
            options,
            helper,
            diagnostic_tx,
        }
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
        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (output_tx, output_rx) = output_mailbox();
        let helper = self.helper.clone();
        let diagnostic_tx = self.diagnostic_tx.clone();
        let connect = HelperRequest::connect_from_options(&options, initial_size);
        let protocol = options.protocol;

        std::thread::Builder::new()
            .name(format!("remote-desktop-{}", protocol.provider_id()))
            .spawn(move || {
                let _proxy_guard = proxy_guard;
                backend_loop::run(
                    helper,
                    connect,
                    input_rx,
                    output_tx,
                    protocol,
                    diagnostic_tx,
                );
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
