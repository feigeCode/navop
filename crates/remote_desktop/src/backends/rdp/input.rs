use std::time::{Duration, Instant};

use super::transport::{BackendSignal, send_failure, write_request};
use super::*;

enum RemoteInputBatch {
    Inputs(Vec<RemoteDesktopInput>),
    Disconnected,
}

pub(super) fn handle_backend_signals(
    signal_rx: &std::sync::mpsc::Receiver<BackendSignal>,
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
    was_connected: &mut bool,
) -> Option<HelperRunResult> {
    while let Ok(signal) = signal_rx.try_recv() {
        match signal {
            BackendSignal::Connected => *was_connected = true,
            BackendSignal::Disconnected(reason) => {
                close_helper(helper, stdin, output_tx);
                return Some(reconnect_result(reason, false, *was_connected));
            }
            BackendSignal::OutputEnded => {
                close_helper(helper, stdin, output_tx);
                return Some(reconnect_result(
                    "remote desktop helper output ended".to_string(),
                    false,
                    *was_connected,
                ));
            }
        }
    }
    None
}

pub(super) fn handle_remote_input(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
    was_connected: bool,
    protocol: RemoteDesktopProtocol,
) -> Option<HelperRunResult> {
    let inputs = match drain_remote_inputs(input_rx) {
        RemoteInputBatch::Inputs(inputs) => inputs,
        RemoteInputBatch::Disconnected => {
            close_helper(helper, stdin, output_tx);
            return Some(HelperRunResult::InputClosed);
        }
    };

    for input in inputs {
        match input {
            RemoteDesktopInput::Close => {
                close_helper(helper, stdin, output_tx);
                return Some(HelperRunResult::Closed);
            }
            RemoteDesktopInput::Reconnect => {
                close_helper(helper, stdin, output_tx);
                return Some(reconnect_result(
                    "manual reconnect".to_string(),
                    true,
                    was_connected,
                ));
            }
            input => {
                remember_reconnect_state(&input, connect, latest_clipboard_text);
                if let Some(reason) = forward_remote_input(input, stdin, output_tx, protocol) {
                    close_helper(helper, stdin, output_tx);
                    return Some(reconnect_result(reason, false, was_connected));
                }
            }
        }
    }
    None
}

fn drain_remote_inputs(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
) -> RemoteInputBatch {
    let mut inputs = Vec::new();
    for _ in 0..MAX_INPUTS_PER_POLL {
        match input_rx.try_recv() {
            Ok(input) => inputs.push(input),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                if inputs.is_empty() {
                    return RemoteInputBatch::Disconnected;
                }
                inputs.push(RemoteDesktopInput::Close);
                break;
            }
        }
    }
    RemoteInputBatch::Inputs(coalesce_remote_inputs(inputs))
}

pub(super) fn coalesce_remote_inputs<I>(inputs: I) -> Vec<RemoteDesktopInput>
where
    I: IntoIterator<Item = RemoteDesktopInput>,
{
    let mut coalesced = Vec::new();
    let mut pending_mouse_move = None;
    for input in inputs {
        match input {
            RemoteDesktopInput::MouseMove { .. } => pending_mouse_move = Some(input),
            input => {
                if let Some(mouse_move) = pending_mouse_move.take() {
                    coalesced.push(mouse_move);
                }
                coalesced.push(input);
            }
        }
    }
    if let Some(mouse_move) = pending_mouse_move {
        coalesced.push(mouse_move);
    }
    coalesced
}

fn forward_remote_input(
    input: RemoteDesktopInput,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) -> Option<String> {
    let request = HelperRequest::from_remote_input_for_protocol(&input, protocol)?;
    write_request(stdin, &request, output_tx)
        .err()
        .map(|_| "failed to send RDP helper request".to_string())
}

pub(super) fn poll_helper_exit(
    helper: &mut std::process::Child,
    was_connected: bool,
) -> Option<HelperRunResult> {
    match helper.try_wait() {
        Ok(Some(status)) => Some(reconnect_result(
            format!("RDP helper exited with {status}"),
            false,
            was_connected,
        )),
        Ok(None) => None,
        Err(error) => Some(reconnect_result(
            format!("failed to poll RDP helper: {error}"),
            false,
            was_connected,
        )),
    }
}

fn reconnect_result(reason: String, manual: bool, was_connected: bool) -> HelperRunResult {
    HelperRunResult::Reconnect {
        reason,
        manual,
        was_connected,
    }
}

pub(super) fn wait_before_reconnect(
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    delay: Duration,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        match input_rx.try_recv() {
            Ok(RemoteDesktopInput::Close) => return false,
            Ok(RemoteDesktopInput::Reconnect) => return true,
            Ok(input) => remember_reconnect_state(&input, connect, latest_clipboard_text),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
        }
        if Instant::now() >= deadline {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn remember_reconnect_state(
    input: &RemoteDesktopInput,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
) {
    match input {
        RemoteDesktopInput::Resize { width, height } => {
            if let HelperRequest::Connect {
                width: connect_width,
                height: connect_height,
                ..
            } = connect
            {
                *connect_width = *width;
                *connect_height = *height;
            }
        }
        RemoteDesktopInput::ClipboardText { text } => {
            *latest_clipboard_text = Some(text.clone());
        }
        _ => {}
    }
}

fn close_helper(
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
) {
    let _ = write_request(stdin, &HelperRequest::Close, output_tx);
    if let Err(error) = helper.wait() {
        send_failure(output_tx, &format!("failed to wait RDP helper: {error}"));
    }
}

pub(super) fn reconnect_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(2),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

pub(super) fn reconnect_status_message(reason: &str, delay: Duration) -> String {
    format!(
        "RDP disconnected: {}. Reconnecting in {}s",
        user_facing_disconnect_reason(reason),
        delay.as_secs()
    )
}

fn user_facing_disconnect_reason(reason: &str) -> &'static str {
    if reason.contains("Fast-Path") {
        return "display update error";
    }
    if reason.contains("/Users/") || reason.contains(".cargo/git/checkouts") {
        return "session error";
    }
    if reason.trim().is_empty() {
        return "connection lost";
    }
    "connection lost"
}
