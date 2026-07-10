use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use super::*;

pub(super) enum BackendSignal {
    Connected,
    Disconnected(String),
    OutputEnded,
}

pub(super) fn start_helper_session(
    helper: &HelperProcessConfig,
    connect: &HelperRequest,
    latest_clipboard_text: &Option<String>,
    output_tx: &OutputMailboxSender,
) -> Result<
    (
        std::process::Child,
        std::process::ChildStdin,
        std::sync::mpsc::Receiver<BackendSignal>,
    ),
    (),
> {
    send_status(output_tx, "starting remote desktop helper");
    let Some(mut helper) = spawn_helper(helper, output_tx.clone()) else {
        return Err(());
    };
    let Some(stdout) = helper.stdout.take() else {
        send_failure(output_tx, "remote desktop helper stdout unavailable");
        return Err(());
    };
    let Some(mut stdin) = helper.stdin.take() else {
        send_failure(output_tx, "remote desktop helper stdin unavailable");
        return Err(());
    };
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    spawn_output_reader(stdout, output_tx.clone(), signal_tx);
    write_request(&mut stdin, connect, output_tx).map_err(|_| ())?;
    if let Some(text) = latest_clipboard_text.clone() {
        let _ = write_request(
            &mut stdin,
            &HelperRequest::ClipboardText { text },
            output_tx,
        );
    }
    Ok((helper, stdin, signal_rx))
}

fn spawn_helper(
    helper: &HelperProcessConfig,
    output_tx: OutputMailboxSender,
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
            send_failure(
                &output_tx,
                &format!(
                    "failed to start remote desktop helper {}: {error}",
                    helper.command.display()
                ),
            );
            None
        }
    }
}

fn spawn_output_reader(
    stdout: std::process::ChildStdout,
    output_tx: OutputMailboxSender,
    signal_tx: std::sync::mpsc::Sender<BackendSignal>,
) {
    let _ = std::thread::Builder::new()
        .name("remote-desktop-rdp-output".to_string())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_helper_output(&mut reader) {
                    Ok(Some(output)) => forward_helper_output(output, &output_tx, &signal_tx),
                    Ok(None) => break,
                    Err(error) => {
                        send_failure(
                            &output_tx,
                            &format!("failed to read remote desktop helper: {error}"),
                        );
                        break;
                    }
                }
            }
            let _ = signal_tx.send(BackendSignal::OutputEnded);
        });
}

pub(super) struct HelperOutput {
    pub(super) output: RemoteDesktopOutput,
    pub(super) connected: bool,
    pub(super) disconnect_message: Option<String>,
}

pub(super) fn read_helper_output(
    reader: &mut impl BufRead,
) -> anyhow::Result<Option<HelperOutput>> {
    let mut line = Vec::new();
    let header_bytes = reader.read_until(b'\n', &mut line)?;
    if header_bytes == 0 {
        return Ok(None);
    }
    let event = decode_event_line(std::str::from_utf8(&line)?)?;
    let connected = matches!(event, HelperEvent::Connected { .. });
    let disconnect_message = helper_disconnect_message(&event);
    match event {
        HelperEvent::FrameBytes {
            width,
            height,
            rgba_len,
        } => read_binary_frame_output(reader, width, height, rgba_len).map(Some),
        HelperEvent::FrameBgraBytes {
            width,
            height,
            bgra_len,
        } => read_binary_bgra_frame_output(reader, width, height, bgra_len).map(Some),
        event => Ok(Some(HelperOutput {
            output: helper_event_to_output(event)?,
            connected,
            disconnect_message,
        })),
    }
}

fn read_binary_frame_output<R>(
    reader: &mut R,
    width: u16,
    height: u16,
    rgba_len: usize,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    let expected_len = usize::from(width) * usize::from(height) * 4;
    if rgba_len != expected_len {
        anyhow::bail!(
            "invalid binary frame payload length: expected {expected_len}, got {rgba_len}"
        );
    }
    let mut rgba = vec![0; rgba_len];
    reader.read_exact(&mut rgba)?;
    Ok(HelperOutput {
        output: RemoteDesktopOutput::Frame {
            width,
            height,
            rgba,
        },
        connected: false,
        disconnect_message: None,
    })
}

fn read_binary_bgra_frame_output<R>(
    reader: &mut R,
    width: u16,
    height: u16,
    bgra_len: usize,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    let expected_len = usize::from(width) * usize::from(height) * 4;
    if bgra_len != expected_len {
        anyhow::bail!("invalid BGRA frame payload length: expected {expected_len}, got {bgra_len}");
    }
    let mut bgra = vec![0; bgra_len];
    reader.read_exact(&mut bgra)?;
    Ok(HelperOutput {
        output: RemoteDesktopOutput::FrameBgra {
            width,
            height,
            bgra,
        },
        connected: false,
        disconnect_message: None,
    })
}

pub(super) fn forward_helper_output(
    helper_output: HelperOutput,
    output_tx: &OutputMailboxSender,
    signal_tx: &std::sync::mpsc::Sender<BackendSignal>,
) {
    if helper_output.connected {
        let _ = signal_tx.send(BackendSignal::Connected);
    }
    if let Some(message) = helper_output.disconnect_message {
        let _ = signal_tx.send(BackendSignal::Disconnected(message));
    }
    let _ = output_tx.send(helper_output.output);
}

pub(super) fn write_request(
    stdin: &mut std::process::ChildStdin,
    request: &HelperRequest,
    output_tx: &OutputMailboxSender,
) -> anyhow::Result<()> {
    let line = encode_request_line(request)?;
    if let Err(error) = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()) {
        send_failure(
            output_tx,
            &format!("failed to write RDP helper request: {error}"),
        );
        anyhow::bail!(error);
    }
    Ok(())
}

pub(super) fn helper_event_to_output(event: HelperEvent) -> anyhow::Result<RemoteDesktopOutput> {
    Ok(match event {
        HelperEvent::Status { message } => RemoteDesktopOutput::Status(message),
        HelperEvent::Connected { width, height } => RemoteDesktopOutput::Connected {
            width,
            height,
            capabilities: rdp_capabilities(),
        },
        HelperEvent::Frame { width, height, .. } => RemoteDesktopOutput::Frame {
            width,
            height,
            rgba: event.into_rgba()?,
        },
        HelperEvent::FrameBytes { .. } | HelperEvent::FrameBgraBytes { .. } => {
            anyhow::bail!("binary frame payload is missing")
        }
        HelperEvent::CursorDefault => RemoteDesktopOutput::CursorDefault,
        HelperEvent::CursorHidden => RemoteDesktopOutput::CursorHidden,
        HelperEvent::CursorPosition { x, y } => RemoteDesktopOutput::CursorPosition { x, y },
        HelperEvent::ClipboardText { text } => RemoteDesktopOutput::ClipboardText { text },
        HelperEvent::ConnectionFailure { message } => {
            RemoteDesktopOutput::ConnectionFailure(message)
        }
        HelperEvent::Terminated { message } => RemoteDesktopOutput::Terminated(message),
    })
}

fn rdp_capabilities() -> RemoteDesktopCapabilities {
    RemoteDesktopCapabilities {
        clipboard_text: true,
        ..RemoteDesktopCapabilities::rdp_mvp()
    }
}

pub(super) fn helper_disconnect_message(event: &HelperEvent) -> Option<String> {
    match event {
        HelperEvent::ConnectionFailure { message } | HelperEvent::Terminated { message } => {
            Some(message.clone())
        }
        _ => None,
    }
}

pub(super) fn send_status(output_tx: &OutputMailboxSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

pub(super) fn send_failure(output_tx: &OutputMailboxSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(message.to_string()));
}
