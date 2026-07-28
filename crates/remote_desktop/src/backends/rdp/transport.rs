use std::io::{BufReader, Write};
use std::process::{Command, Stdio};

use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BackendSignal {
    Connected,
    Disconnected(HelperDisconnect),
    OutputEnded,
}

pub(super) fn start_helper_session(
    helper: &HelperProcessConfig,
    connect: &HelperRequest,
    latest_clipboard_text: &Option<String>,
    latest_clipboard_files: &Option<ClipboardFilesSnapshot>,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> Result<
    (
        std::process::Child,
        std::process::ChildStdin,
        std::sync::mpsc::Receiver<BackendSignal>,
    ),
    (),
> {
    send_status(output_tx, &format!("starting {} helper", protocol.label()));
    let Some(mut helper) = spawn_helper(helper, protocol) else {
        return Err(());
    };
    let Some(stdout) = helper.stdout.take() else {
        tracing::warn!(
            protocol = protocol.label(),
            "remote desktop helper stdout unavailable"
        );
        return Err(());
    };
    let Some(mut stdin) = helper.stdin.take() else {
        tracing::warn!(
            protocol = protocol.label(),
            "remote desktop helper stdin unavailable"
        );
        return Err(());
    };
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    spawn_output_reader(stdout, output_tx.clone(), signal_tx, protocol);
    write_request(&mut stdin, connect, output_tx, protocol).map_err(|_| ())?;
    for request in
        reconnect_replay_requests(latest_clipboard_text, latest_clipboard_files, protocol)
    {
        let _ = write_request(&mut stdin, &request, output_tx, protocol);
    }
    Ok((helper, stdin, signal_rx))
}

pub(super) fn reconnect_replay_requests(
    latest_clipboard_text: &Option<String>,
    latest_clipboard_files: &Option<ClipboardFilesSnapshot>,
    protocol: RemoteDesktopProtocol,
) -> Vec<HelperRequest> {
    let mut requests = Vec::with_capacity(2);
    if let Some(text) = latest_clipboard_text.clone() {
        requests.push(HelperRequest::ClipboardText { text });
    }
    if protocol == RemoteDesktopProtocol::Rdp
        && let Some(snapshot) = latest_clipboard_files.clone()
    {
        requests.push(HelperRequest::ClipboardFiles {
            transfer_id: snapshot.transfer_id,
            paths: snapshot.paths,
        });
    }
    requests
}

fn spawn_helper(
    helper: &HelperProcessConfig,
    protocol: RemoteDesktopProtocol,
) -> Option<std::process::Child> {
    let mut command = Command::new(&helper.command);
    process_util::configure_background_child(&mut command);
    match command
        .args(&helper.args)
        .current_dir(&helper.working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            tracing::warn!(
                protocol = protocol.label(),
                command = %helper.command.display(),
                ?error,
                "failed to start remote desktop helper"
            );
            None
        }
    }
}

fn spawn_output_reader(
    stdout: std::process::ChildStdout,
    output_tx: OutputMailboxSender,
    signal_tx: std::sync::mpsc::Sender<BackendSignal>,
    protocol: RemoteDesktopProtocol,
) {
    let _ = std::thread::Builder::new()
        .name(format!("remote-desktop-{}-output", protocol.provider_id()))
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match super::transport_frames::read_helper_output(&mut reader, protocol) {
                    Ok(Some(output)) => forward_helper_output(output, &output_tx, &signal_tx),
                    Ok(None) => break,
                    Err(error) => {
                        let reason = format!("failed to read {} helper: {error}", protocol.label());
                        tracing::warn!(
                            protocol = protocol.label(),
                            ?error,
                            "failed to read remote desktop helper output"
                        );
                        let _ = signal_tx.send(BackendSignal::Disconnected(HelperDisconnect {
                            kind: None,
                            reason,
                        }));
                        break;
                    }
                }
            }
            let _ = signal_tx.send(BackendSignal::OutputEnded);
        });
}

pub(super) struct HelperOutput {
    pub(super) output: Option<RemoteDesktopOutput>,
    pub(super) connected: bool,
    pub(super) disconnect: Option<HelperDisconnect>,
}

pub(super) fn forward_helper_output(
    helper_output: HelperOutput,
    output_tx: &OutputMailboxSender,
    signal_tx: &std::sync::mpsc::Sender<BackendSignal>,
) {
    // Publish a connected output before notifying the backend loop so the
    // session is marked ready before any subsequent signal is handled.
    // Terminal helper diagnostics deliberately have no user-visible output;
    // only the backend receives them for structured classification.
    let HelperOutput {
        output,
        connected,
        disconnect,
    } = helper_output;
    if let Some(output) = output {
        let _ = output_tx.send(output);
    }
    if connected {
        let _ = signal_tx.send(BackendSignal::Connected);
    }
    if let Some(disconnect) = disconnect {
        let _ = signal_tx.send(BackendSignal::Disconnected(disconnect));
    }
}

pub(super) fn write_request(
    stdin: &mut std::process::ChildStdin,
    request: &HelperRequest,
    _output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<()> {
    let line = encode_request_line(request)?;
    if let Err(error) = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()) {
        tracing::warn!(
            protocol = protocol.label(),
            ?error,
            "failed to write remote desktop helper request"
        );
        anyhow::bail!(error);
    }
    Ok(())
}

pub(super) fn send_status(output_tx: &OutputMailboxSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

pub(super) fn send_reconnecting(
    output_tx: &OutputMailboxSender,
    reconnect: RemoteDesktopReconnect,
) {
    let _ = output_tx.send(RemoteDesktopOutput::Reconnecting(reconnect));
}

pub(super) fn send_failure(output_tx: &OutputMailboxSender, failure: RemoteDesktopFailure) {
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(failure));
}

pub(super) fn send_terminated(output_tx: &OutputMailboxSender, failure: RemoteDesktopFailure) {
    let _ = output_tx.send(RemoteDesktopOutput::Terminated(failure));
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
