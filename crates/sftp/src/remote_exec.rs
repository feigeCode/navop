use crate::TransferCancelled;
use anyhow::{Result, bail};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) struct RemoteCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: u32,
}

pub(crate) async fn exec_remote_command(
    manager: &SshSessionManager,
    command: &str,
    cancelled: Arc<AtomicBool>,
) -> Result<RemoteCommandOutput> {
    exec_remote_command_with_input(manager, command, &[], cancelled).await
}

pub(crate) async fn exec_remote_command_with_input(
    manager: &SshSessionManager,
    command: &str,
    input: &[u8],
    cancelled: Arc<AtomicBool>,
) -> Result<RemoteCommandOutput> {
    ensure_not_cancelled(&cancelled)?;
    let mut channel = manager.open_channel().await?;
    ensure_not_cancelled_with_channel(&cancelled, &mut channel).await?;
    channel.exec(command).await?;
    ensure_not_cancelled_with_channel(&cancelled, &mut channel).await?;
    if !input.is_empty() {
        if channel.send_data(input).await.is_err() {
            let _ = channel.close().await;
            bail!("failed to send protected input to remote command");
        }
        ensure_not_cancelled_with_channel(&cancelled, &mut channel).await?;
    }
    channel.eof().await?;
    ensure_not_cancelled_with_channel(&cancelled, &mut channel).await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut exit_signal = None;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = channel.close().await;
            return Err(TransferCancelled.into());
        }
        let event = tokio::select! {
            event = channel.recv() => event,
            () = tokio::time::sleep(Duration::from_millis(100)) => continue,
        };
        if handle_event(
            event,
            &mut stdout,
            &mut stderr,
            &mut exit_status,
            &mut exit_signal,
        ) {
            break;
        }
    }
    let _ = channel.close().await;
    finish_output(stdout, stderr, exit_status, exit_signal)
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

async fn ensure_not_cancelled_with_channel(
    cancelled: &AtomicBool,
    channel: &mut impl SshChannel,
) -> Result<()> {
    if ensure_not_cancelled(cancelled).is_ok() {
        return Ok(());
    }
    let _ = channel.close().await;
    Err(TransferCancelled.into())
}

fn handle_event(
    event: Option<ChannelEvent>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    exit_status: &mut Option<u32>,
    exit_signal: &mut Option<(String, String)>,
) -> bool {
    match event {
        Some(ChannelEvent::Data(data)) => stdout.extend_from_slice(&data),
        Some(ChannelEvent::ExtendedData { ext: 1, data }) => stderr.extend_from_slice(&data),
        Some(ChannelEvent::ExtendedData { .. }) => {}
        Some(ChannelEvent::ExitStatus(status)) => *exit_status = Some(status),
        Some(ChannelEvent::ExitSignal {
            signal_name,
            error_message,
        }) => *exit_signal = Some((signal_name, error_message)),
        Some(ChannelEvent::Eof | ChannelEvent::Close) | None => return true,
    }
    false
}

fn finish_output(
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_status: Option<u32>,
    exit_signal: Option<(String, String)>,
) -> Result<RemoteCommandOutput> {
    if let Some((signal, message)) = exit_signal {
        bail!("remote command terminated by signal {signal}: {message}");
    }
    let Some(exit_status) = exit_status else {
        bail!("remote command closed without reporting an exit status");
    };
    Ok(RemoteCommandOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_status,
    })
}
