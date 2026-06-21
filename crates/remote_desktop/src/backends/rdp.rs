use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{
    RemoteDesktopBackend, RemoteDesktopCapabilities, RemoteDesktopConnectionOptions,
    RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol, RemoteDesktopRuntime,
    RemoteDesktopSize,
    helper_protocol::{HelperEvent, HelperRequest, decode_event_line, encode_request_line},
};

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
        let (output_tx, output_rx) = std::sync::mpsc::channel();
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
                                send_status(&output_tx, "reconnecting remote desktop session");
                                continue;
                            }
                            let delay = reconnect_delay(reconnect_attempt);
                            reconnect_attempt = reconnect_attempt.saturating_add(1);
                            send_status(&output_tx, &reconnect_status_message(&reason, delay));
                            if !wait_before_reconnect(
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

enum HelperRunResult {
    Closed,
    InputClosed,
    Reconnect {
        reason: String,
        manual: bool,
        was_connected: bool,
    },
}

enum BackendSignal {
    Connected,
    Disconnected(String),
    OutputEnded,
}

enum RemoteInputBatch {
    Inputs(Vec<RemoteDesktopInput>),
    Disconnected,
}

fn run_helper_session(
    helper: &HelperProcessConfig,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    protocol: RemoteDesktopProtocol,
) -> HelperRunResult {
    let Ok((mut helper, mut stdin, signal_rx)) =
        start_helper_session(helper, connect, latest_clipboard_text, output_tx)
    else {
        return HelperRunResult::Reconnect {
            reason: "failed to start remote desktop helper".to_string(),
            manual: false,
            was_connected: false,
        };
    };

    let mut was_connected = false;
    loop {
        if let Some(result) = handle_backend_signals(
            &signal_rx,
            &mut helper,
            &mut stdin,
            output_tx,
            &mut was_connected,
        ) {
            return result;
        }
        if let Some(result) = handle_remote_input(
            input_rx,
            connect,
            latest_clipboard_text,
            &mut helper,
            &mut stdin,
            output_tx,
            was_connected,
            protocol,
        ) {
            return result;
        }
        if let Some(result) = poll_helper_exit(&mut helper, was_connected) {
            return result;
        }
        std::thread::sleep(RDP_BACKEND_POLL_INTERVAL);
    }
}

fn start_helper_session(
    helper: &HelperProcessConfig,
    connect: &HelperRequest,
    latest_clipboard_text: &Option<String>,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
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

fn handle_backend_signals(
    signal_rx: &std::sync::mpsc::Receiver<BackendSignal>,
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
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

fn handle_remote_input(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    helper: &mut std::process::Child,
    stdin: &mut std::process::ChildStdin,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
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

fn coalesce_remote_inputs<I>(inputs: I) -> Vec<RemoteDesktopInput>
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
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    protocol: RemoteDesktopProtocol,
) -> Option<String> {
    let request = HelperRequest::from_remote_input_for_protocol(&input, protocol)?;
    write_request(stdin, &request, output_tx)
        .err()
        .map(|_| "failed to send RDP helper request".to_string())
}

fn poll_helper_exit(
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

fn wait_before_reconnect(
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
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
) {
    let _ = write_request(stdin, &HelperRequest::Close, output_tx);
    if let Err(error) = helper.wait() {
        send_failure(output_tx, &format!("failed to wait RDP helper: {error}"));
    }
}

fn reconnect_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(2),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

fn reconnect_status_message(reason: &str, delay: Duration) -> String {
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

fn helper_disconnect_message(event: &HelperEvent) -> Option<String> {
    match event {
        HelperEvent::ConnectionFailure { message } | HelperEvent::Terminated { message } => {
            Some(message.clone())
        }
        _ => None,
    }
}

fn send_status(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

fn spawn_helper(
    helper: &HelperProcessConfig,
    output_tx: std::sync::mpsc::Sender<RemoteDesktopOutput>,
) -> Option<std::process::Child> {
    match Command::new(&helper.command)
        .args(&helper.args)
        .current_dir(&helper.working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            let message = format!(
                "failed to start remote desktop helper {}: {error}",
                helper.command.display()
            );
            send_failure(&output_tx, &message);
            None
        }
    }
}

fn spawn_output_reader(
    stdout: std::process::ChildStdout,
    output_tx: std::sync::mpsc::Sender<RemoteDesktopOutput>,
    signal_tx: std::sync::mpsc::Sender<BackendSignal>,
) {
    let _ = std::thread::Builder::new()
        .name("remote-desktop-rdp-output".to_string())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => forward_helper_line(&line, &output_tx, &signal_tx),
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

fn forward_helper_line(
    line: &str,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    signal_tx: &std::sync::mpsc::Sender<BackendSignal>,
) {
    match decode_event_line(line) {
        Ok(event) => {
            if matches!(event, HelperEvent::Connected { .. }) {
                let _ = signal_tx.send(BackendSignal::Connected);
            }
            if let Some(message) = helper_disconnect_message(&event) {
                let _ = signal_tx.send(BackendSignal::Disconnected(message));
            }
            let Ok(output) = helper_event_to_output(event) else {
                return;
            };
            let _ = output_tx.send(output);
        }
        Err(error) => send_failure(
            output_tx,
            &format!("invalid remote desktop helper event: {error}"),
        ),
    }
}

fn write_request(
    stdin: &mut std::process::ChildStdin,
    request: &HelperRequest,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
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

fn helper_event_to_output(event: HelperEvent) -> anyhow::Result<RemoteDesktopOutput> {
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

fn send_failure(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(message.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResizeSupport;
    use crate::helper_protocol::HelperEvent;

    #[test]
    fn converts_helper_connected_event_to_rdp_capabilities() {
        let output = helper_event_to_output(HelperEvent::Connected {
            width: 1280,
            height: 720,
        })
        .expect("event converts");

        assert_eq!(
            output,
            RemoteDesktopOutput::Connected {
                width: 1280,
                height: 720,
                capabilities: crate::RemoteDesktopCapabilities {
                    resize: ResizeSupport::RemoteResize,
                    clipboard_text: true,
                    cursor_shape: false,
                    audio: false,
                    file_transfer: false,
                }
            }
        );
    }

    #[test]
    fn converts_helper_clipboard_text_event_to_output() {
        let output = helper_event_to_output(HelperEvent::ClipboardText {
            text: "remote 中文".to_string(),
        })
        .expect("event converts");

        assert_eq!(
            output,
            RemoteDesktopOutput::ClipboardText {
                text: "remote 中文".to_string()
            }
        );
    }

    #[test]
    fn reconnect_delay_uses_bounded_backoff() {
        assert_eq!(std::time::Duration::from_secs(1), reconnect_delay(0));
        assert_eq!(std::time::Duration::from_secs(2), reconnect_delay(1));
        assert_eq!(std::time::Duration::from_secs(5), reconnect_delay(2));
        assert_eq!(std::time::Duration::from_secs(10), reconnect_delay(3));
        assert_eq!(std::time::Duration::from_secs(10), reconnect_delay(20));
    }

    #[test]
    fn helper_events_identify_disconnect_signals() {
        assert_eq!(
            Some("network".to_string()),
            helper_disconnect_message(&HelperEvent::ConnectionFailure {
                message: "network".to_string(),
            })
        );
        assert_eq!(
            Some("closed".to_string()),
            helper_disconnect_message(&HelperEvent::Terminated {
                message: "closed".to_string(),
            })
        );
        assert_eq!(
            None,
            helper_disconnect_message(&HelperEvent::Connected {
                width: 1,
                height: 1
            })
        );
    }

    #[test]
    fn reconnect_status_hides_internal_fast_path_error() {
        let status = reconnect_status_message(
            "[Fast-Path @ /Users/hufei/.cargo/git/checkouts/ironrdp/src/lib.rs:98] custom error",
            std::time::Duration::from_secs(1),
        );

        assert_eq!(
            "RDP disconnected: display update error. Reconnecting in 1s",
            status
        );
    }

    #[test]
    fn coalesces_consecutive_mouse_moves_without_reordering_actions() {
        let inputs = vec![
            RemoteDesktopInput::MouseMove { x: 10, y: 10 },
            RemoteDesktopInput::MouseMove { x: 20, y: 20 },
            RemoteDesktopInput::MouseButton {
                button: crate::RemoteMouseButton::Left,
                pressed: true,
            },
            RemoteDesktopInput::MouseMove { x: 30, y: 30 },
            RemoteDesktopInput::MouseMove { x: 40, y: 40 },
            RemoteDesktopInput::Key {
                key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
                pressed: true,
            },
        ];

        let coalesced = coalesce_remote_inputs(inputs);

        assert_eq!(
            vec![
                RemoteDesktopInput::MouseMove { x: 20, y: 20 },
                RemoteDesktopInput::MouseButton {
                    button: crate::RemoteMouseButton::Left,
                    pressed: true,
                },
                RemoteDesktopInput::MouseMove { x: 40, y: 40 },
                RemoteDesktopInput::Key {
                    key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
                    pressed: true,
                },
            ],
            coalesced
        );
    }
}
