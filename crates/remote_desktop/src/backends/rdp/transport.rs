use std::io::{BufReader, Write};
use std::process::{Command, Stdio};

use super::*;

const MAX_REQUEST_BATCH_BYTES: usize = 4 * 1024 * 1024;
const HELPER_HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BackendSignal {
    ServerHello(crate::helper_protocol::HelperServerHello),
    Connected,
    Disconnected(HelperDisconnect),
    Pong(u64),
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
        crate::helper_protocol::HelperServerHello,
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
        terminate_failed_helper(&mut helper, protocol);
        return Err(());
    };
    let Some(mut stdin) = helper.stdin.take() else {
        tracing::warn!(
            protocol = protocol.label(),
            "remote desktop helper stdin unavailable"
        );
        terminate_failed_helper(&mut helper, protocol);
        return Err(());
    };
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    spawn_output_reader(stdout, output_tx.clone(), signal_tx, protocol);
    if write_request(
        &mut stdin,
        &HelperRequest::ClientHello {
            protocol_version: crate::helper_protocol::HELPER_PROTOCOL_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        output_tx,
        protocol,
    )
    .is_err()
    {
        terminate_failed_helper(&mut helper, protocol);
        return Err(());
    }
    let Some(server_hello) = await_server_hello(&signal_rx, output_tx, protocol) else {
        terminate_failed_helper(&mut helper, protocol);
        return Err(());
    };
    let replay_requests =
        reconnect_replay_requests(latest_clipboard_text, latest_clipboard_files, protocol);
    if write_requests(
        &mut stdin,
        std::iter::once(connect).chain(replay_requests.iter()),
        output_tx,
        protocol,
    )
    .is_err()
    {
        terminate_failed_helper(&mut helper, protocol);
        return Err(());
    }
    Ok((helper, stdin, signal_rx, server_hello))
}

fn await_server_hello(
    signal_rx: &std::sync::mpsc::Receiver<BackendSignal>,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> Option<crate::helper_protocol::HelperServerHello> {
    let deadline = std::time::Instant::now() + HELPER_HELLO_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            send_status(
                output_tx,
                &format!(
                    "{} helper did not complete the protocol handshake within {} seconds",
                    protocol.label(),
                    HELPER_HELLO_TIMEOUT.as_secs()
                ),
            );
            return None;
        }
        match signal_rx.recv_timeout(remaining) {
            Ok(BackendSignal::ServerHello(hello)) => {
                if hello.protocol_version != crate::helper_protocol::HELPER_PROTOCOL_VERSION {
                    send_status(
                        output_tx,
                        &format!(
                            "{} helper protocol {} does not match app protocol {}; update the provider",
                            protocol.label(),
                            hello.protocol_version,
                            crate::helper_protocol::HELPER_PROTOCOL_VERSION
                        ),
                    );
                    return None;
                }
                return Some(hello);
            }
            Ok(BackendSignal::Pong(_)) => {}
            Ok(BackendSignal::Connected)
            | Ok(BackendSignal::Disconnected(_))
            | Ok(BackendSignal::OutputEnded)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return None,
        }
    }
}

fn terminate_failed_helper(helper: &mut std::process::Child, protocol: RemoteDesktopProtocol) {
    if let Err(error) = helper.kill()
        && !matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
        )
    {
        tracing::warn!(
            protocol = protocol.label(),
            ?error,
            "failed to terminate remote desktop helper after startup failure"
        );
    }
    if let Err(error) = helper.wait() {
        tracing::warn!(
            protocol = protocol.label(),
            ?error,
            "failed to reap remote desktop helper after startup failure"
        );
    }
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
                            failure: None,
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
    pub(super) server_hello: Option<crate::helper_protocol::HelperServerHello>,
    pub(super) pong: Option<u64>,
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
        server_hello,
        pong,
    } = helper_output;
    if let Some(server_hello) = server_hello {
        let _ = signal_tx.send(BackendSignal::ServerHello(server_hello));
    }
    if let Some(output) = output {
        let _ = output_tx.send(output);
    }
    if connected {
        let _ = signal_tx.send(BackendSignal::Connected);
    }
    if let Some(disconnect) = disconnect {
        let _ = signal_tx.send(BackendSignal::Disconnected(disconnect));
    }
    if let Some(sequence) = pong {
        let _ = signal_tx.send(BackendSignal::Pong(sequence));
    }
}

pub(super) fn write_request(
    stdin: &mut std::process::ChildStdin,
    request: &HelperRequest,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<()> {
    write_requests(stdin, std::iter::once(request), output_tx, protocol)
}

pub(super) fn write_requests<'a>(
    stdin: &mut std::process::ChildStdin,
    requests: impl IntoIterator<Item = &'a HelperRequest>,
    _output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<()> {
    if let Err(error) = encode_and_write_requests(stdin, requests) {
        tracing::warn!(
            protocol = protocol.label(),
            ?error,
            "failed to write remote desktop helper request batch"
        );
        return Err(error);
    }
    Ok(())
}

fn encode_and_write_requests<'a>(
    writer: &mut impl Write,
    requests: impl IntoIterator<Item = &'a HelperRequest>,
) -> anyhow::Result<()> {
    let mut batch = Vec::new();
    for request in requests {
        let line = encode_request_line(request)?;
        if !batch.is_empty() && batch.len().saturating_add(line.len()) > MAX_REQUEST_BATCH_BYTES {
            writer.write_all(&batch)?;
            batch.clear();
        }
        batch.extend_from_slice(line.as_bytes());
    }
    if batch.is_empty() {
        return Ok(());
    }
    writer.write_all(&batch)?;
    writer.flush()?;
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
