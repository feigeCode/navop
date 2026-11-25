use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

use ssh::SshChannel;

use super::command::quote_shell_arg;
use super::control::TmuxControlTransport;
use super::transport::{LocalTmuxTransport, SshTmuxTransport};
use super::version::TmuxVersion;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub enum TmuxTarget {
    Local,
    Ssh(Arc<ssh::SshSessionManager>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTmux {
    pub session_id: String,
    pub version: Option<TmuxVersion>,
}

pub async fn discover_tmux_client(
    target: &TmuxTarget,
    interactive_tty: &str,
    socket: Option<&str>,
) -> anyhow::Result<Option<DiscoveredTmux>> {
    let clients = capture(target, &shell_command_for_list_clients(socket)).await?;
    tracing::debug!(interactive_tty, clients = %clients, "tmux clients listed");
    let session_id = clients.lines().find_map(|line| {
        let (tty, session) = line.split_once('|')?;
        (tty == interactive_tty).then(|| session.to_string())
    });
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let version = capture(target, &format!("{} -V", tmux_command_prefix(socket)))
        .await
        .ok()
        .and_then(|output| TmuxVersion::parse(&output));
    let Some(version) = version.filter(TmuxVersion::supports_control_mode_integration) else {
        return Ok(None);
    };
    Ok(Some(DiscoveredTmux {
        session_id,
        version: Some(version),
    }))
}

pub async fn open_control_transport(
    target: &TmuxTarget,
    session_id: &str,
    socket: Option<&str>,
) -> anyhow::Result<Box<dyn TmuxControlTransport>> {
    match target {
        TmuxTarget::Local => Ok(Box::new(
            LocalTmuxTransport::connect(session_id, socket).await?,
        )),
        TmuxTarget::Ssh(manager) => Ok(Box::new(
            SshTmuxTransport::connect(manager, session_id, socket).await?,
        )),
    }
}

async fn capture(target: &TmuxTarget, command: &str) -> anyhow::Result<String> {
    match target {
        TmuxTarget::Local => capture_local(command).await,
        TmuxTarget::Ssh(manager) => capture_ssh(manager, command).await,
    }
}

async fn capture_local(command: &str) -> anyhow::Result<String> {
    let output = tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        Command::new("sh").arg("-c").arg(command).output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("tmux discovery timed out"))??;
    anyhow::ensure!(
        output.status.success(),
        "tmux discovery exited with {}",
        output.status
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn capture_ssh(
    manager: &Arc<ssh::SshSessionManager>,
    command: &str,
) -> anyhow::Result<String> {
    tokio::time::timeout(DISCOVERY_TIMEOUT, capture_ssh_inner(manager, command))
        .await
        .map_err(|_| anyhow::anyhow!("tmux discovery timed out"))?
}

async fn capture_ssh_inner(
    manager: &Arc<ssh::SshSessionManager>,
    command: &str,
) -> anyhow::Result<String> {
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

pub fn tmux_command_prefix(socket: Option<&str>) -> String {
    match socket.map(str::trim).filter(|socket| !socket.is_empty()) {
        Some(socket) => format!("tmux -S {}", quote_shell_arg(socket)),
        None => "tmux".to_string(),
    }
}

pub fn shell_command_for_list_clients(socket: Option<&str>) -> String {
    format!(
        "{} list-clients -F '#{{q:client_tty}}|#{{q:client_session}}'",
        tmux_command_prefix(socket)
    )
}

pub fn shell_command_for_attach(session_id: &str, socket: Option<&str>) -> String {
    format!(
        "{} -C attach-session -t {}",
        tmux_command_prefix(socket),
        quote_shell_arg(session_id)
    )
}

#[cfg(test)]
mod tests {
    use super::{shell_command_for_attach, shell_command_for_list_clients, tmux_command_prefix};

    #[test]
    fn shell_commands_are_stable() {
        assert!(shell_command_for_list_clients(None).contains("list-clients"));
        assert_eq!(
            shell_command_for_attach("%1", None),
            "tmux -C attach-session -t '%1'"
        );
    }

    #[test]
    fn custom_socket_is_applied_to_discovery_and_attach() {
        assert_eq!(
            tmux_command_prefix(Some("/tmp/tmux-1000/custom.sock")),
            "tmux -S '/tmp/tmux-1000/custom.sock'"
        );
        assert_eq!(
            shell_command_for_attach("%1", Some("/tmp/custom.sock")),
            "tmux -S '/tmp/custom.sock' -C attach-session -t '%1'"
        );
        assert!(
            shell_command_for_list_clients(Some("/tmp/custom.sock"))
                .starts_with("tmux -S '/tmp/custom.sock' list-clients")
        );
    }
}
