use crate::direct_copy_scripts::{RECEIVER_SCRIPT, SENDER_SCRIPT};
use crate::{ServerCopyItem, TransferCancelled, TransferProgress};
use anyhow::{Result, anyhow};
use ssh::{ChannelEvent, SshChannel, SshConnectConfig, SshSessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) async fn try_direct_copy(
    source_config: SshConnectConfig,
    target_config: SshConnectConfig,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    progress: impl Fn(TransferProgress) + Send + Sync + 'static,
) -> Result<()> {
    let token = random_token();
    let target_root = target_root(items)?;
    let target_manager = SshSessionManager::new(target_config.clone());
    let mut target_channel = target_manager.open_channel().await?;
    target_channel
        .exec(&python_command(
            RECEIVER_SCRIPT,
            &[&encode_arg(&target_root), &token],
        ))
        .await?;
    let port = wait_for_ready(&mut target_channel, &cancelled).await?;

    let source_manager = SshSessionManager::new(source_config);
    let mut source_channel = source_manager.open_channel().await?;
    let paths = items
        .iter()
        .map(|item| item.source_path.clone())
        .collect::<Vec<_>>();
    let paths = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(&paths)?,
    );
    source_channel
        .exec(&python_command(
            SENDER_SCRIPT,
            &[
                &encode_arg(&target_config.host),
                &port.to_string(),
                &token,
                &paths,
            ],
        ))
        .await?;

    let abort = AtomicBool::new(false);
    let cancellation = DirectCancellation {
        cancelled: &cancelled,
        abort: &abort,
    };
    let source_future = wait_for_sender(&mut source_channel, cancellation, &progress);
    let target_future = wait_for_exit(&mut target_channel, cancellation);
    tokio::pin!(source_future);
    tokio::pin!(target_future);
    let (source_result, target_result) = tokio::select! {
        source_result = &mut source_future => {
            if source_result.is_err() {
                abort.store(true, Ordering::Relaxed);
            }
            let target_result = target_future.as_mut().await;
            (source_result, target_result)
        }
        target_result = &mut target_future => {
            if target_result.is_err() {
                abort.store(true, Ordering::Relaxed);
            }
            let source_result = source_future.as_mut().await;
            (source_result, target_result)
        }
    };
    source_result?;
    target_result
}

async fn wait_for_ready(channel: &mut ssh::RusshChannel, cancelled: &AtomicBool) -> Result<u16> {
    let mut output = Vec::new();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        let event = tokio::time::timeout(Duration::from_secs(8), channel.recv())
            .await
            .map_err(|_| anyhow!("direct transfer listener did not become ready"))?
            .ok_or_else(|| anyhow!("direct transfer listener closed before ready"))?;
        match event {
            ChannelEvent::Data(data) => {
                output.extend_from_slice(&data);
                if let Some(port) = parse_ready_port(&output) {
                    return Ok(port);
                }
            }
            ChannelEvent::ExtendedData { data, .. } => output.extend_from_slice(&data),
            ChannelEvent::ExitStatus(status) => {
                return Err(anyhow!("direct transfer listener exited with {status}"));
            }
            ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            } => return Err(anyhow!("direct listener {signal_name}: {error_message}")),
            ChannelEvent::Eof | ChannelEvent::Close => {
                return Err(anyhow!("direct transfer listener closed before ready"));
            }
        }
    }
}

async fn wait_for_sender(
    channel: &mut ssh::RusshChannel,
    cancellation: DirectCancellation<'_>,
    progress: &(dyn Fn(TransferProgress) + Send + Sync),
) -> Result<()> {
    let mut sender_progress = SenderProgress {
        total: 0,
        transferred: 0,
        progress,
        buffer: Vec::new(),
    };
    let mut status = None;
    while let Some(event) = next_event(channel, cancellation).await? {
        match event {
            ChannelEvent::Data(data) | ChannelEvent::ExtendedData { data, .. } => {
                sender_progress.consume(&data);
            }
            ChannelEvent::ExitStatus(value) => status = Some(value),
            ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            } => return Err(anyhow!("direct sender {signal_name}: {error_message}")),
            ChannelEvent::Eof | ChannelEvent::Close => break,
        }
    }
    match status {
        Some(0) | None => Ok(()),
        Some(value) => Err(anyhow!("direct sender exited with {value}")),
    }
}

struct SenderProgress<'a> {
    total: u64,
    transferred: u64,
    progress: &'a (dyn Fn(TransferProgress) + Send + Sync),
    buffer: Vec<u8>,
}

impl SenderProgress<'_> {
    fn consume(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            self.consume_line(&line[..line.len().saturating_sub(1)]);
        }
    }

    fn consume_line(&mut self, line: &[u8]) {
        if let Some(value) = line.strip_prefix(b"NAVOP_TOTAL ") {
            self.total = String::from_utf8_lossy(value).trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix(b"NAVOP_PROGRESS ") {
            self.transferred += String::from_utf8_lossy(value)
                .trim()
                .parse::<u64>()
                .unwrap_or(0);
            (self.progress)(TransferProgress {
                transferred: self.transferred,
                total: self.total,
                speed: 0.0,
                current_file: None,
                current_file_transferred: 0,
                current_file_total: 0,
            });
        }
    }
}

async fn wait_for_exit(
    channel: &mut ssh::RusshChannel,
    cancellation: DirectCancellation<'_>,
) -> Result<()> {
    let mut status = None;
    while let Some(event) = next_event(channel, cancellation).await? {
        match event {
            ChannelEvent::ExitStatus(value) => status = Some(value),
            ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            } => return Err(anyhow!("direct receiver {signal_name}: {error_message}")),
            ChannelEvent::Eof | ChannelEvent::Close => break,
            ChannelEvent::Data(_) | ChannelEvent::ExtendedData { .. } => {}
        }
    }
    match status {
        Some(0) | None => Ok(()),
        Some(value) => Err(anyhow!("direct receiver exited with {value}")),
    }
}

async fn next_event(
    channel: &mut ssh::RusshChannel,
    cancellation: DirectCancellation<'_>,
) -> Result<Option<ChannelEvent>> {
    tokio::select! {
        event = channel.recv() => Ok(event),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {
            if cancellation.cancelled.load(Ordering::Relaxed) {
                Err(TransferCancelled.into())
            } else if cancellation.abort.load(Ordering::Relaxed) {
                Err(anyhow!("direct transfer aborted"))
            } else {
                Ok(Some(ChannelEvent::Data(Vec::new())))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct DirectCancellation<'a> {
    cancelled: &'a AtomicBool,
    abort: &'a AtomicBool,
}

fn target_root(items: &[ServerCopyItem]) -> Result<String> {
    let first = items.first().ok_or_else(|| anyhow!("no files selected"))?;
    let parent = parent_path(&first.target_path);
    if items
        .iter()
        .any(|item| parent_path(&item.target_path) != parent)
    {
        return Err(anyhow!(
            "server copy items must share a destination directory"
        ));
    }
    Ok(parent.to_string())
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or(".")
        .to_string()
}

fn random_token() -> String {
    let bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode_arg(value: &str) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
}

fn python_command(script: &str, args: &[&str]) -> String {
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, script);
    let args = args
        .iter()
        .map(|arg| format!("'{arg}'"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("python3 -c 'import base64;exec(base64.b64decode(\"{encoded}\"))' {args}")
}

fn parse_ready_port(output: &[u8]) -> Option<u16> {
    let line = output
        .split(|byte| *byte == b'\n')
        .find(|line| line.starts_with(b"NAVOP_READY "))?;
    String::from_utf8_lossy(&line[12..]).trim().parse().ok()
}

#[cfg(test)]
#[path = "direct_copy_tests.rs"]
mod tests;
