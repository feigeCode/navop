use std::time::{Duration, Instant};

use anyhow::Context as _;
use tokio::net::TcpStream;
use vnc_client::{PixelFormat, VncClient, VncConnector, VncEncoding, X11Event};

use super::vnc_encoding::ConnectedVncSession;
use super::vnc_input::{VncInputAction, handle_pending_inputs};
use crate::{RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput};

const VNC_POLL_INTERVAL: Duration = Duration::from_millis(8);

pub fn run_vnc_thread(
    options: RemoteDesktopConnectionOptions,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: std::sync::mpsc::Sender<RemoteDesktopOutput>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        send_failure(&output_tx, "failed to start VNC runtime");
        return;
    };
    runtime.block_on(run_vnc_backend(options, input_rx, &output_tx));
}

async fn run_vnc_backend(
    options: RemoteDesktopConnectionOptions,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
) {
    let mut latest_clipboard_text = None;
    let mut reconnect_attempt = 0usize;
    loop {
        match run_vnc_session(&options, &mut latest_clipboard_text, input_rx, output_tx).await {
            VncSessionResult::Closed | VncSessionResult::InputClosed => break,
            VncSessionResult::Reconnect {
                reason,
                manual,
                was_connected,
            } => {
                if was_connected || manual {
                    reconnect_attempt = 0;
                }
                if manual {
                    send_status(output_tx, "reconnecting VNC session");
                    continue;
                }
                let delay = reconnect_delay(reconnect_attempt);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                send_status(output_tx, &reconnect_status_message(&reason, delay));
                if !wait_before_reconnect(input_rx, &mut latest_clipboard_text, delay).await {
                    break;
                }
            }
        }
    }
}

enum VncSessionResult {
    Closed,
    InputClosed,
    Reconnect {
        reason: String,
        manual: bool,
        was_connected: bool,
    },
}

async fn run_vnc_session(
    options: &RemoteDesktopConnectionOptions,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
) -> VncSessionResult {
    send_status(
        output_tx,
        &format!("connecting to VNC {}", options.destination),
    );
    let client = match connect_vnc(options).await {
        Ok(client) => client,
        Err(error) => return reconnect_result(error.to_string(), false, false),
    };
    if let Some(text) = latest_clipboard_text.clone() {
        let _ = client.input(X11Event::CopyText(text)).await;
    }
    run_connected_vnc_session(client, latest_clipboard_text, input_rx, output_tx).await
}

async fn connect_vnc(options: &RemoteDesktopConnectionOptions) -> anyhow::Result<VncClient> {
    let tcp = TcpStream::connect(&options.destination)
        .await
        .with_context(|| format!("failed to connect VNC {}", options.destination))?;
    let password = options.password.clone().unwrap_or_default();
    let state = VncConnector::new(tcp)
        .set_auth_method(async move { Ok(password) })
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::CursorPseudo)
        .add_encoding(VncEncoding::Raw)
        .allow_shared(true)
        .set_pixel_format(PixelFormat::rgba())
        .build()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let client = state
        .try_start()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .finish()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(client)
}

async fn run_connected_vnc_session(
    client: VncClient,
    latest_clipboard_text: &mut Option<String>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
) -> VncSessionResult {
    let mut session = ConnectedVncSession::new(client);
    loop {
        if let Err(reason) = session.poll_events(output_tx).await {
            return reconnect_result(reason, false, session.was_connected);
        }
        let action = handle_pending_inputs(
            &session.client,
            latest_clipboard_text,
            input_rx,
            &mut session.pointer,
            output_tx,
        )
        .await;
        if let Some(result) = session_result_from_action(action, session.was_connected) {
            return result;
        }
        if let Err(reason) = session.refresh_if_needed().await {
            return reconnect_result(reason, false, session.was_connected);
        }
        tokio::time::sleep(VNC_POLL_INTERVAL).await;
    }
}

fn session_result_from_action(
    action: VncInputAction,
    was_connected: bool,
) -> Option<VncSessionResult> {
    match action {
        VncInputAction::Continue => None,
        VncInputAction::Closed => Some(VncSessionResult::Closed),
        VncInputAction::InputClosed => Some(VncSessionResult::InputClosed),
        VncInputAction::Reconnect => Some(reconnect_result(
            "manual reconnect".to_string(),
            true,
            was_connected,
        )),
        VncInputAction::Failed(reason) => Some(reconnect_result(reason, false, was_connected)),
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
        "VNC disconnected: {reason}. Reconnecting in {}s",
        delay.as_secs()
    )
}

fn reconnect_result(reason: String, manual: bool, was_connected: bool) -> VncSessionResult {
    VncSessionResult::Reconnect {
        reason,
        manual,
        was_connected,
    }
}

async fn wait_before_reconnect(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    latest_clipboard_text: &mut Option<String>,
    delay: Duration,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        match handle_wait_input(input_rx, latest_clipboard_text) {
            WaitAction::Continue => {}
            WaitAction::ReconnectNow => return true,
            WaitAction::Stop => return false,
        }
        if Instant::now() >= deadline {
            return true;
        }
        tokio::time::sleep(VNC_POLL_INTERVAL).await;
    }
}

enum WaitAction {
    Continue,
    ReconnectNow,
    Stop,
}

fn handle_wait_input(
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    latest_clipboard_text: &mut Option<String>,
) -> WaitAction {
    match input_rx.try_recv() {
        Ok(RemoteDesktopInput::Close) => WaitAction::Stop,
        Ok(RemoteDesktopInput::Reconnect) => WaitAction::ReconnectNow,
        Ok(RemoteDesktopInput::ClipboardText { text }) => {
            *latest_clipboard_text = Some(text);
            WaitAction::Continue
        }
        Ok(RemoteDesktopInput::Text { text }) => {
            *latest_clipboard_text = Some(text);
            WaitAction::Continue
        }
        Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => WaitAction::Continue,
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => WaitAction::Stop,
    }
}

fn send_status(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

fn send_failure(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(message.to_string()));
}
