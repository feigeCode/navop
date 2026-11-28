//! Real SSH + tmux control-mode integration test.
//!
//! Requires `NAVOP_LIVE_SSH=user:password@host:port` and tmux >= 3.2 on the
//! remote host. Run with:
//!
//! `cargo test -p terminal --test tmux_control_live -- --ignored --nocapture`

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use ssh::{
    HostKeyPolicy, HostKeyVerifier, PtyConfig, SshAuth, SshChannel, SshConnectConfig,
    SshSessionManager,
};
use terminal::tmux::{
    TmuxControlEvent, TmuxSessionState, TmuxSplitPlacement, TmuxTarget, discover_tmux_client,
    open_control_transport, spawn_tmux_control,
};

struct LiveTarget {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl LiveTarget {
    fn from_env() -> Self {
        let raw =
            std::env::var("NAVOP_LIVE_SSH").expect("set NAVOP_LIVE_SSH=user:password@host:port");
        let (userinfo, hostport) = raw.rsplit_once('@').expect("NAVOP_LIVE_SSH needs '@'");
        let (username, password) = userinfo.split_once(':').expect("need user:password");
        let (host, port) = hostport
            .rsplit_once(':')
            .map(|(host, port)| {
                (
                    host.to_string(),
                    port.parse().expect("port must be a number"),
                )
            })
            .unwrap_or((hostport.to_string(), 22));
        Self {
            host,
            port,
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    fn config(&self) -> SshConnectConfig {
        SshConnectConfig {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth: SshAuth::Password(self.password.clone()),
            timeout: Some(Duration::from_secs(30)),
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: Some(3),
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::new(HostKeyPolicy::Insecure, None, None),
            x11_forwarding: false,
            allow_legacy_algorithms: false,
            forward_agent: false,
            agent_identities: Vec::new(),
            proxy_command: None,
        }
    }
}

async fn run_remote(manager: &SshSessionManager, command: &str) -> Result<String> {
    let mut channel = manager.open_channel().await?;
    channel.exec(command).await?;
    let mut output = Vec::new();
    loop {
        match channel.recv().await {
            Some(ssh::ChannelEvent::Data(data)) => output.extend(data),
            Some(ssh::ChannelEvent::Eof)
            | Some(ssh::ChannelEvent::ExitStatus(_))
            | Some(ssh::ChannelEvent::ExitSignal { .. })
            | Some(ssh::ChannelEvent::Close)
            | None => break,
            Some(ssh::ChannelEvent::ExtendedData { .. }) => {}
        }
    }
    let _ = channel.close().await;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

async fn wait_for_remote_file(
    manager: &SshSessionManager,
    path: &str,
    expected: &str,
) -> Result<()> {
    for _ in 0..30 {
        let output = run_remote(manager, &format!("cat {}", shell_single_quote(path))).await?;
        if output.trim() == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow!("timed out waiting for remote file {path}"))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn wait_for_changed_state(
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TmuxControlEvent>,
    predicate: impl Fn(&TmuxSessionState) -> bool,
) -> Result<TmuxSessionState> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("timed out waiting for tmux state change"));
        }
        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(TmuxControlEvent::Changed(state))) if predicate(&state) => return Ok(state),
            Ok(Some(TmuxControlEvent::Detached { reason })) => {
                return Err(anyhow!("tmux control detached while waiting: {reason}"));
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err(anyhow!("tmux control event channel closed")),
            Err(_) => return Err(anyhow!("timed out waiting for tmux state change")),
        }
    }
}

#[tokio::test]
#[ignore = "requires a reachable SSH host with tmux >= 3.2"]
async fn tmux_control_mode_reads_state_and_sends_keys_over_real_ssh() -> Result<()> {
    let target = LiveTarget::from_env();
    let manager = Arc::new(SshSessionManager::new(target.config()));
    let session_name = "navop-live-tmux";
    let decoy_session_name = "navop-live-decoy";
    let marker_path = "/tmp/navop-live-tmux-marker";

    run_remote(
        &manager,
        &format!(
            "tmux kill-session -t {session_name} 2>/dev/null || true; \
             tmux kill-session -t {decoy_session_name} 2>/dev/null || true; \
             tmux new-session -d -s {session_name} -c /tmp 'bash --noprofile --norc -i'; \
             tmux new-session -d -s {decoy_session_name} -c /var/tmp 'bash --noprofile --norc -i'; \
             tmux split-window -t {decoy_session_name} -c / 'bash --noprofile --norc -i'"
        ),
    )
    .await?;

    let mut interactive = manager.open_channel().await?;
    interactive.request_pty(&PtyConfig::default()).await?;
    interactive
        .exec(&format!("tmux attach-session -t {session_name}"))
        .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let clients = run_remote(
        &manager,
        "tmux list-clients -F '#{q:client_tty}|#{q:client_session}'",
    )
    .await?;
    let (interactive_tty, session_id) = clients
        .lines()
        .filter_map(|line| line.split_once('|'))
        .find(|(_, session)| *session == session_name)
        .ok_or_else(|| anyhow!("tmux client was not listed: {clients:?}"))?;

    let target = TmuxTarget::Ssh(manager.clone());
    let discovered = discover_tmux_client(&target, interactive_tty, None)
        .await?
        .ok_or_else(|| anyhow!("tmux discovery did not find the interactive client"))?;
    assert_eq!(discovered.session_id, session_id);
    assert!(
        discovered
            .version
            .as_ref()
            .is_some_and(|version| (version.major, version.minor) >= (3, 2))
    );

    let transport = open_control_transport(&target, &discovered.session_id, None).await?;
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let controller = spawn_tmux_control(
        transport,
        discovered.version,
        discovered.session_id,
        interactive_tty.to_string(),
        event_tx,
    );

    let attached = tokio::time::timeout(Duration::from_secs(10), event_rx.recv())
        .await
        .context("tmux control attach timed out")?
        .ok_or_else(|| anyhow!("tmux control event channel closed"))?;
    let state = match attached {
        TmuxControlEvent::Attached(state) => state,
        other => return Err(anyhow!("expected Attached, got {other:?}")),
    };
    assert!(state.attached);
    assert!(!state.windows.is_empty());
    assert_eq!(state.panes.len(), 1);
    assert_eq!(state.active_cwd(), Some("/tmp"));

    let original_pane_id = state
        .active_pane()
        .map(|pane| pane.id.clone())
        .ok_or_else(|| anyhow!("no active tmux pane"))?;

    controller
        .split_window(&original_pane_id, TmuxSplitPlacement::Right)
        .await?;

    let split_state = wait_for_changed_state(&mut event_rx, |state| state.panes.len() == 2).await?;
    let split_pane_id = split_state
        .panes
        .iter()
        .find(|pane| pane.id != original_pane_id)
        .map(|pane| pane.id.clone())
        .ok_or_else(|| anyhow!("split pane was not added"))?;
    assert_eq!(
        split_state.active_pane().map(|pane| pane.id.as_str()),
        Some(split_pane_id.as_str())
    );
    assert_eq!(
        split_state
            .pane(&split_pane_id)
            .and_then(|pane| pane.cwd.as_deref()),
        Some("/tmp")
    );

    controller
        .send_keys(
            &original_pane_id,
            &format!("printf '%s\\n' tmux-live-ok > {marker_path}"),
            true,
        )
        .await?;
    wait_for_remote_file(&manager, marker_path, "tmux-live-ok").await?;
    let active_panes = run_remote(
        &manager,
        &format!("tmux list-panes -t {session_name} -F '#{{pane_id}}|#{{pane_active}}'"),
    )
    .await?;
    assert!(
        active_panes
            .lines()
            .any(|line| line == format!("{split_pane_id}|1")),
        "sending to a non-active pane must not change the active pane: {active_panes:?}"
    );

    let _ = run_remote(
        &manager,
        &format!(
            "rm -f {marker_path}; \
             tmux kill-session -t {session_name} 2>/dev/null || true; \
             tmux kill-session -t {decoy_session_name} 2>/dev/null || true"
        ),
    )
    .await;
    let _ = interactive.close().await;
    controller.shutdown();
    Ok(())
}

#[tokio::test]
#[ignore = "requires a reachable SSH host with tmux >= 3.2"]
async fn tmux_custom_socket_is_discovered_and_controlled_over_real_ssh() -> Result<()> {
    let target = LiveTarget::from_env();
    let manager = Arc::new(SshSessionManager::new(target.config()));
    let session_name = "navop-custom-socket";
    let socket_path = "/tmp/navop-live-tmux.sock";
    let marker_path = "/tmp/navop-custom-socket-marker";
    let socket = shell_single_quote(socket_path);

    run_remote(
        &manager,
        &format!(
            "tmux -S {socket} kill-server 2>/dev/null || true; \
             rm -f {socket}; \
             tmux -S {socket} new-session -d -s {session_name} -c /tmp \
                 'bash --noprofile --norc -i'"
        ),
    )
    .await?;

    let mut interactive = manager.open_channel().await?;
    interactive.request_pty(&PtyConfig::default()).await?;
    interactive
        .exec(&format!(
            "tmux -S {socket} attach-session -t {session_name}"
        ))
        .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let clients = run_remote(
        &manager,
        &format!("tmux -S {socket} list-clients -F '#{{q:client_tty}}|#{{q:client_session}}'"),
    )
    .await?;
    let (interactive_tty, session_id) = clients
        .lines()
        .filter_map(|line| line.split_once('|'))
        .find(|(_, session)| *session == session_name)
        .ok_or_else(|| anyhow!("custom-socket tmux client was not listed: {clients:?}"))?;

    let target = TmuxTarget::Ssh(manager.clone());
    let discovered = discover_tmux_client(&target, interactive_tty, Some(socket_path))
        .await?
        .ok_or_else(|| anyhow!("custom-socket tmux discovery failed"))?;
    assert_eq!(discovered.session_id, session_id);

    let transport =
        open_control_transport(&target, &discovered.session_id, Some(socket_path)).await?;
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let controller = spawn_tmux_control(
        transport,
        discovered.version,
        discovered.session_id,
        interactive_tty.to_string(),
        event_tx,
    );

    let attached = tokio::time::timeout(Duration::from_secs(10), event_rx.recv())
        .await
        .context("custom-socket tmux attach timed out")?
        .ok_or_else(|| anyhow!("custom-socket control event channel closed"))?;
    let state = match attached {
        TmuxControlEvent::Attached(state) => state,
        other => return Err(anyhow!("expected Attached, got {other:?}")),
    };
    let pane_id = state
        .active_pane()
        .map(|pane| pane.id.clone())
        .ok_or_else(|| anyhow!("custom-socket session has no active pane"))?;
    controller
        .send_keys(
            &pane_id,
            &format!("printf '%s\\n' custom-socket-ok > {marker_path}"),
            true,
        )
        .await?;
    wait_for_remote_file(&manager, marker_path, "custom-socket-ok").await?;

    let _ = run_remote(
        &manager,
        &format!(
            "rm -f {marker_path}; tmux -S {socket} kill-server 2>/dev/null || true; rm -f {socket}"
        ),
    )
    .await;
    let _ = interactive.close().await;
    controller.shutdown();
    Ok(())
}
