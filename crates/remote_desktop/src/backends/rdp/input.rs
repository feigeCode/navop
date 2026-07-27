use super::reconnect::remember_reconnect_state;
use super::transport::{BackendSignal, send_failure, write_request};
use super::*;

enum RemoteInputBatch {
    Inputs(Vec<RemoteDesktopInput>),
    Disconnected,
}

pub(super) struct RemoteInputContext<'a> {
    pub(super) connect: &'a mut HelperRequest,
    pub(super) latest_clipboard_text: &'a mut Option<String>,
    pub(super) latest_clipboard_files: &'a mut Option<ClipboardFilesSnapshot>,
    pub(super) helper: &'a mut std::process::Child,
    pub(super) stdin: &'a mut std::process::ChildStdin,
    pub(super) output_tx: &'a OutputMailboxSender,
    pub(super) protocol: RemoteDesktopProtocol,
}

pub(super) fn handle_backend_signals(
    signal_rx: &std::sync::mpsc::Receiver<BackendSignal>,
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
    was_connected: &mut bool,
    protocol: RemoteDesktopProtocol,
) -> Option<HelperRunResult> {
    while let Ok(signal) = signal_rx.try_recv() {
        match signal {
            BackendSignal::Connected => *was_connected = true,
            BackendSignal::Disconnected(reason) => {
                close_helper(helper, stdin, output_tx, protocol);
                return Some(reconnect_result(reason, false, *was_connected));
            }
            BackendSignal::OutputEnded => {
                close_helper(helper, stdin, output_tx, protocol);
                return Some(reconnect_result(
                    format!("{} helper output ended", protocol.label()),
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
    context: &mut RemoteInputContext<'_>,
    was_connected: bool,
) -> Option<HelperRunResult> {
    let inputs = match drain_remote_inputs(input_rx) {
        RemoteInputBatch::Inputs(inputs) => inputs,
        RemoteInputBatch::Disconnected => {
            close_helper(
                context.helper,
                context.stdin,
                context.output_tx,
                context.protocol,
            );
            return Some(HelperRunResult::InputClosed);
        }
    };

    for input in inputs {
        match input {
            RemoteDesktopInput::Close => {
                close_helper(
                    context.helper,
                    context.stdin,
                    context.output_tx,
                    context.protocol,
                );
                return Some(HelperRunResult::Closed);
            }
            RemoteDesktopInput::Reconnect => {
                close_helper(
                    context.helper,
                    context.stdin,
                    context.output_tx,
                    context.protocol,
                );
                return Some(reconnect_result(
                    "manual reconnect".to_string(),
                    true,
                    was_connected,
                ));
            }
            input => {
                remember_reconnect_state(
                    &input,
                    context.connect,
                    context.latest_clipboard_text,
                    context.latest_clipboard_files,
                    context.protocol,
                );
                if let Some(reason) =
                    forward_remote_input(input, context.stdin, context.output_tx, context.protocol)
                {
                    close_helper(
                        context.helper,
                        context.stdin,
                        context.output_tx,
                        context.protocol,
                    );
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
    write_request(stdin, &request, output_tx, protocol)
        .err()
        .map(|_| format!("failed to send {} helper request", protocol.label()))
}

pub(super) fn poll_helper_exit(
    helper: &mut std::process::Child,
    was_connected: bool,
    protocol: RemoteDesktopProtocol,
) -> Option<HelperRunResult> {
    match helper.try_wait() {
        Ok(Some(status)) => Some(reconnect_result(
            format!("{} helper exited with {status}", protocol.label()),
            false,
            was_connected,
        )),
        Ok(None) => None,
        Err(error) => Some(reconnect_result(
            format!("failed to poll {} helper: {error}", protocol.label()),
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

fn close_helper(
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &OutputMailboxSender,
    protocol: RemoteDesktopProtocol,
) {
    let _ = write_request(stdin, &HelperRequest::Close, output_tx, protocol);
    if let Err(error) = helper.wait() {
        send_failure(
            output_tx,
            &format!("failed to wait {} helper: {error}", protocol.label()),
        );
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
