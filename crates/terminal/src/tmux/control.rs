use async_trait::async_trait;
#[cfg(test)]
use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::command::quote_tmux_arg;
use super::protocol::{TmuxControlLine, parse_control_line};
use super::state::{
    LIST_PANES_COMMAND_FORMAT, LIST_PANES_FORMAT, LIST_WINDOWS_FORMAT, TmuxSessionState,
    parse_list_panes_with_command_state, parse_list_windows,
};
use super::version::TmuxVersion;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TmuxControlError {
    Disconnected,
    Timeout,
    Command(String),
    Invalid(String),
}

impl fmt::Display for TmuxControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => formatter.write_str("tmux control channel disconnected"),
            Self::Timeout => formatter.write_str("tmux control command timed out"),
            Self::Command(message) => write!(formatter, "tmux command failed: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid tmux command: {message}"),
        }
    }
}

impl std::error::Error for TmuxControlError {}

#[async_trait]
pub trait TmuxControlTransport: Send {
    async fn write_line(&mut self, line: &str) -> anyhow::Result<()>;
    async fn read_line(&mut self) -> anyhow::Result<Option<Vec<u8>>>;
    async fn shutdown(&mut self);
}

#[async_trait]
impl<T: TmuxControlTransport + ?Sized> TmuxControlTransport for Box<T> {
    async fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        (**self).write_line(line).await
    }

    async fn read_line(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        (**self).read_line().await
    }

    async fn shutdown(&mut self) {
        (**self).shutdown().await;
    }
}

#[derive(Debug)]
enum TmuxControlCommand {
    Run {
        line: String,
        reply: oneshot::Sender<Result<String, TmuxControlError>>,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum TmuxControlEvent {
    Attached(TmuxSessionState),
    Changed(TmuxSessionState),
    Detached { reason: String },
}

#[derive(Clone)]
pub struct TmuxControllerHandle {
    command_tx: mpsc::UnboundedSender<TmuxControlCommand>,
}

impl fmt::Debug for TmuxControllerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TmuxControllerHandle")
    }
}

impl TmuxControllerHandle {
    pub async fn run(&self, line: impl Into<String>) -> Result<String, TmuxControlError> {
        let (reply, response) = oneshot::channel();
        self.command_tx
            .send(TmuxControlCommand::Run {
                line: line.into(),
                reply,
            })
            .map_err(|_| TmuxControlError::Disconnected)?;
        tokio::time::timeout(COMMAND_TIMEOUT, response)
            .await
            .map_err(|_| TmuxControlError::Timeout)?
            .map_err(|_| TmuxControlError::Disconnected)?
    }

    pub async fn send_keys(
        &self,
        pane_id: &str,
        keys: &str,
        submit: bool,
    ) -> Result<(), TmuxControlError> {
        let line =
            super::command::send_keys_command(pane_id, keys).map_err(TmuxControlError::Invalid)?;
        self.run(line).await?;
        if submit {
            self.run(super::command::send_enter_command(pane_id))
                .await?;
        }
        Ok(())
    }

    pub async fn split_window(
        &self,
        pane_id: &str,
        placement: super::TmuxSplitPlacement,
    ) -> Result<(), TmuxControlError> {
        self.run(super::command::split_window_command(pane_id, placement))
            .await?;
        Ok(())
    }

    pub async fn select_window(&self, window_id: &str) -> Result<(), TmuxControlError> {
        self.run(super::command::select_window_command(window_id))
            .await?;
        Ok(())
    }

    pub async fn select_pane(
        &self,
        window_id: &str,
        pane_id: &str,
    ) -> Result<(), TmuxControlError> {
        self.select_window(window_id).await?;
        self.run(super::command::select_pane_command(pane_id))
            .await?;
        Ok(())
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.send(TmuxControlCommand::Shutdown);
    }
}

pub fn spawn_tmux_control(
    mut transport: impl TmuxControlTransport + 'static,
    version: Option<TmuxVersion>,
    session_id: String,
    interactive_tty: String,
    event_tx: mpsc::UnboundedSender<TmuxControlEvent>,
) -> TmuxControllerHandle {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let initial_session_name = session_id.clone();
        let mut state = TmuxSessionState {
            session_id,
            session_name: initial_session_name,
            attached: true,
            version,
            ..TmuxSessionState::default()
        };
        if let Err(reason) = initialize_control(&mut transport, &mut state).await {
            let _ = event_tx.send(TmuxControlEvent::Detached { reason });
            transport.shutdown().await;
            return;
        }

        if let Err(reason) = subscribe_to_pane_changes(&mut transport, state.version.as_ref()).await
        {
            tracing::debug!(%reason, "tmux subscription setup failed; polling remains active");
        }

        let _ = event_tx.send(TmuxControlEvent::Attached(state.clone()));
        let mut refresh = tokio::time::interval(REFRESH_INTERVAL);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                command = command_rx.recv() => {
                    match command {
                        Some(TmuxControlCommand::Run { line, reply }) => {
                            let result = run_command(&mut transport, &line).await;
                            let _ = reply.send(result);
                        }
                        Some(TmuxControlCommand::Shutdown) | None => {
                            transport.shutdown().await;
                            return;
                        }
                    }
                }
                line = transport.read_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if !handle_async_notification(&line, &mut state, &event_tx) {
                                continue;
                            }
                            if matches!(parse_control_line(&line), Some(TmuxControlLine::Exit { .. })) {
                                let _ = event_tx.send(TmuxControlEvent::Detached {
                                    reason: "tmux control client exited".to_string(),
                                });
                                transport.shutdown().await;
                                return;
                            }
                        }
                        Ok(None) | Err(_) => {
                            let _ = event_tx.send(TmuxControlEvent::Detached {
                                reason: "tmux control channel closed".to_string(),
                            });
                            transport.shutdown().await;
                            return;
                        }
                    }
                }
                _ = refresh.tick() => {
                    match interactive_client_present(&mut transport, &interactive_tty).await {
                        Ok(true) => {}
                        Ok(false) => {
                            let _ = event_tx.send(TmuxControlEvent::Detached {
                                reason: "interactive tmux client detached".to_string(),
                            });
                            transport.shutdown().await;
                            return;
                        }
                        Err(_) => {}
                    }
                    match refresh_state(&mut transport, &mut state).await {
                        Ok(changed) => {
                            if changed {
                                let _ = event_tx.send(TmuxControlEvent::Changed(state.clone()));
                            }
                        }
                        Err(_) => {
                            let _ = event_tx.send(TmuxControlEvent::Detached {
                                reason: "tmux state refresh failed".to_string(),
                            });
                            transport.shutdown().await;
                            return;
                        }
                    }
                }
            }
        }
    });

    TmuxControllerHandle { command_tx }
}

async fn interactive_client_present(
    transport: &mut impl TmuxControlTransport,
    interactive_tty: &str,
) -> anyhow::Result<bool> {
    let output = run_command(
        transport,
        "list-clients -F '#{q:client_tty}|#{q:client_session}'",
    )
    .await?;
    Ok(output.lines().any(|line| {
        line.split_once('|')
            .is_some_and(|(tty, _)| tty == interactive_tty)
    }))
}

async fn initialize_control(
    transport: &mut impl TmuxControlTransport,
    state: &mut TmuxSessionState,
) -> Result<(), String> {
    synchronize_control(transport)
        .await
        .map_err(|error| error.to_string())?;
    run_command(transport, "refresh-client -f no-output")
        .await
        .map_err(|error| error.to_string())?;
    refresh_state(transport, state)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn synchronize_control(
    transport: &mut impl TmuxControlTransport,
) -> Result<(), TmuxControlError> {
    const SENTINEL: &str = "__NAVOP_TMUX_CONTROL_SYNC__";
    transport
        .write_line("display-message -p '__NAVOP_TMUX_CONTROL_SYNC__'")
        .await
        .map_err(|_| TmuxControlError::Disconnected)?;
    let mut in_block = false;
    let mut output = String::new();
    loop {
        let line = transport
            .read_line()
            .await
            .map_err(|_| TmuxControlError::Disconnected)?
            .ok_or(TmuxControlError::Disconnected)?;
        match parse_control_line(&line) {
            Some(TmuxControlLine::Begin { .. }) => {
                in_block = true;
                output.clear();
            }
            Some(TmuxControlLine::End { .. }) if in_block => {
                if output.contains(SENTINEL) {
                    return Ok(());
                }
                in_block = false;
                output.clear();
            }
            Some(TmuxControlLine::Error { .. }) if in_block => {
                return Err(TmuxControlError::Command(output.trim().to_string()));
            }
            Some(TmuxControlLine::Other(_)) | None if in_block => {
                if !line.starts_with(b"%") {
                    output.push_str(&String::from_utf8_lossy(&line));
                }
            }
            _ => {}
        }
    }
}

async fn subscribe_to_pane_changes(
    transport: &mut impl TmuxControlTransport,
    version: Option<&TmuxVersion>,
) -> anyhow::Result<()> {
    let Some(version) = version else {
        return Ok(());
    };
    if !version.supports_format_subscription() {
        return Ok(());
    }
    let subscription = format!(
        "refresh-client -B 'navop-panes:%*:#{{session_id}}|#{{window_id}}|#{{pane_id}}|#{{pane_current_path}}|#{{pane_current_command}}|#{{pane_in_mode}}'"
    );
    run_command(transport, &subscription).await?;
    Ok(())
}

async fn refresh_state(
    transport: &mut impl TmuxControlTransport,
    state: &mut TmuxSessionState,
) -> anyhow::Result<bool> {
    // `list-panes -s` without `-t` follows the server's most recently used
    // session, not necessarily the control client's attached session. Always
    // target the discovered session id so a Navop terminal never absorbs panes
    // from unrelated tmux sessions on the same server.
    let session_id = quote_tmux_arg(&state.session_id);
    let windows_output = run_command(
        transport,
        &format!("list-windows -t {session_id} -F '{LIST_WINDOWS_FORMAT}'"),
    )
    .await?;
    let command_state = state
        .version
        .as_ref()
        .is_some_and(TmuxVersion::supports_command_state);
    let panes_format = if command_state {
        LIST_PANES_COMMAND_FORMAT
    } else {
        LIST_PANES_FORMAT
    };
    let panes_output = run_command(
        transport,
        &format!("list-panes -s -t {session_id} -F '{panes_format}'"),
    )
    .await?;
    let session_name = run_command(
        transport,
        &format!("display-message -t {session_id} -p '#{{session_name}}'"),
    )
    .await
    .ok()
    .map(|name| name.trim().to_string())
    .filter(|name| !name.is_empty());

    let previous = state.clone();
    state.windows = parse_list_windows(&windows_output);
    state.panes = parse_list_panes_with_command_state(&panes_output, command_state);
    if let Some(session_name) = session_name {
        state.session_name = session_name;
    }
    state.attached = true;
    Ok(previous != *state)
}

async fn run_command(
    transport: &mut impl TmuxControlTransport,
    line: &str,
) -> Result<String, TmuxControlError> {
    tokio::time::timeout(COMMAND_TIMEOUT, run_command_inner(transport, line))
        .await
        .map_err(|_| TmuxControlError::Timeout)?
}

async fn run_command_inner(
    transport: &mut impl TmuxControlTransport,
    line: &str,
) -> Result<String, TmuxControlError> {
    transport
        .write_line(line)
        .await
        .map_err(|_| TmuxControlError::Disconnected)?;
    let mut output = String::new();
    let mut in_block = false;
    loop {
        let line = transport
            .read_line()
            .await
            .map_err(|_| TmuxControlError::Disconnected)?
            .ok_or(TmuxControlError::Disconnected)?;
        match parse_control_line(&line) {
            Some(TmuxControlLine::Begin { .. }) => {
                in_block = true;
                output.clear();
            }
            Some(TmuxControlLine::End { .. }) if in_block => return Ok(output),
            Some(TmuxControlLine::Error { .. }) if in_block => {
                return Err(TmuxControlError::Command(output.trim().to_string()));
            }
            Some(TmuxControlLine::Other(_)) | None if in_block => {
                if !line.starts_with(b"%") {
                    output.push_str(&String::from_utf8_lossy(&line));
                }
            }
            Some(_) | None => {}
        }
    }
}

fn handle_async_notification(
    line: &[u8],
    state: &mut TmuxSessionState,
    event_tx: &mpsc::UnboundedSender<TmuxControlEvent>,
) -> bool {
    let Some(control_line) = parse_control_line(line) else {
        return false;
    };
    match control_line {
        TmuxControlLine::SessionChanged { id, name } => {
            state.session_id = id;
            state.session_name = name;
            state.windows.clear();
            state.panes.clear();
            let _ = event_tx.send(TmuxControlEvent::Changed(state.clone()));
        }
        TmuxControlLine::SessionRenamed { name, .. } => {
            state.session_name = name;
            let _ = event_tx.send(TmuxControlEvent::Changed(state.clone()));
        }
        TmuxControlLine::WindowAdded { .. }
        | TmuxControlLine::WindowClosed { .. }
        | TmuxControlLine::WindowRenamed { .. }
        | TmuxControlLine::WindowPaneChanged { .. }
        | TmuxControlLine::LayoutChanged { .. }
        | TmuxControlLine::PaneModeChanged { .. }
        | TmuxControlLine::SessionWindowChanged { .. }
        | TmuxControlLine::SessionsChanged
        | TmuxControlLine::SubscriptionChanged { .. } => {}
        TmuxControlLine::Exit { .. } => return true,
        _ => {}
    }
    false
}

#[cfg(test)]
pub struct FakeTransport {
    lines: VecDeque<Vec<u8>>,
}

#[cfg(test)]
impl FakeTransport {
    pub fn new(lines: impl IntoIterator<Item = impl Into<Vec<u8>>>) -> Self {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl TmuxControlTransport for FakeTransport {
    async fn write_line(&mut self, _line: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read_line(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.lines.pop_front())
    }

    async fn shutdown(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::{FakeTransport, TmuxControlEvent, run_command, spawn_tmux_control};
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn command_reads_until_end_and_ignores_notifications() {
        let mut transport = FakeTransport::new([
            "%window-add @1\n",
            "%begin 1 2 1\n",
            "main\n",
            "%end 1 2 1\n",
        ]);
        let output = run_command(&mut transport, "display-message -p '#{session_name}'")
            .await
            .expect("command should complete");
        assert_eq!(output, "main\n");
    }

    #[tokio::test]
    async fn command_reports_error_block() {
        let mut transport =
            FakeTransport::new(["%begin 1 2 1\n", "bad command\n", "%error 1 2 1\n"]);
        let error = run_command(&mut transport, "bad")
            .await
            .expect_err("command should fail");
        assert_eq!(error.to_string(), "tmux command failed: bad command");
    }

    #[tokio::test]
    async fn actor_initializes_without_pane_output_and_emits_attached_snapshot() {
        let transport = FakeTransport::new([
            "%begin 1 0 1\n",
            "__NAVOP_TMUX_CONTROL_SYNC__\n",
            "%end 1 0 1\n",
            "%begin 1 1 1\n",
            "%end 1 1 1\n",
            "%begin 1 2 1\n",
            "$0|@1|0|editor|1\n",
            "%end 1 2 1\n",
            "%begin 1 3 1\n",
            "$0|@1|0|%1|80|24|1|1|/srv/app|bash|0|shell|10\n",
            "%end 1 3 1\n",
            "%begin 1 4 1\n",
            "main\n",
            "%end 1 4 1\n",
        ]);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let controller = spawn_tmux_control(
            transport,
            None,
            "$0".to_string(),
            "/dev/pts/1".to_string(),
            event_tx,
        );

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("actor should attach")
            .expect("attach event");
        let TmuxControlEvent::Attached(state) = event else {
            panic!("expected attached event");
        };
        assert_eq!(state.session_name, "main");
        assert_eq!(state.active_cwd(), Some("/srv/app"));
        controller.shutdown();
    }
}
