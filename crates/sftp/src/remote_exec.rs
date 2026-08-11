use crate::TransferCancelled;
use anyhow::{Result, bail};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::Instant;

const REMOTE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(test)]
const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub(crate) struct RemoteCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: u32,
}

#[derive(Debug)]
pub(crate) struct RemoteCommandTimeout {
    stage: &'static str,
}

impl std::fmt::Display for RemoteCommandTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "remote command timed out while {}", self.stage)
    }
}

impl std::error::Error for RemoteCommandTimeout {}

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
    exec_remote_command_with_input_until(manager, command, input, cancelled, None).await
}

pub(crate) async fn exec_remote_command_with_input_deadline(
    manager: &SshSessionManager,
    command: &str,
    input: &[u8],
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<RemoteCommandOutput> {
    exec_remote_command_with_input_until(manager, command, input, cancelled, Some(deadline)).await
}

async fn exec_remote_command_with_input_until(
    manager: &SshSessionManager,
    command: &str,
    input: &[u8],
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
) -> Result<RemoteCommandOutput> {
    ensure_not_cancelled(&cancelled)?;
    let mut channel = await_remote_step(
        manager.open_channel(),
        &cancelled,
        deadline,
        "opening the SSH channel",
    )
    .await?;
    let result = exec_channel_with_input(&mut channel, command, input, cancelled, deadline).await;
    close_channel_bounded(&mut channel).await;
    result
}

async fn exec_channel_with_input(
    channel: &mut impl SshChannel,
    command: &str,
    input: &[u8],
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
) -> Result<RemoteCommandOutput> {
    ensure_not_cancelled(&cancelled)?;
    await_remote_step(
        channel.exec(command),
        &cancelled,
        deadline,
        "starting the remote command",
    )
    .await?;
    if !input.is_empty() {
        await_remote_step(
            channel.send_data(input),
            &cancelled,
            deadline,
            "sending protected input",
        )
        .await
        .map_err(|error| {
            if error.is::<RemoteCommandTimeout>() || error.is::<TransferCancelled>() {
                error
            } else {
                anyhow::anyhow!("failed to send protected input to remote command: {error}")
            }
        })?;
    }
    await_remote_step(
        channel.eof(),
        &cancelled,
        deadline,
        "finishing remote command input",
    )
    .await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut exit_signal = None;
    loop {
        let event = await_remote_step(
            async { Ok(channel.recv().await) },
            &cancelled,
            deadline,
            "waiting for remote command output",
        )
        .await?;
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
    finish_output(stdout, stderr, exit_status, exit_signal)
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

async fn await_remote_step<T>(
    future: impl Future<Output = Result<T>>,
    cancelled: &AtomicBool,
    deadline: Option<Instant>,
    stage: &'static str,
) -> Result<T> {
    ensure_not_cancelled(cancelled)?;
    let deadline_wait = async move {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(future);
    tokio::pin!(deadline_wait);
    loop {
        tokio::select! {
            result = &mut future => return result,
            () = &mut deadline_wait => {
                ensure_not_cancelled(cancelled)?;
                return Err(RemoteCommandTimeout { stage }.into());
            }
            () = tokio::time::sleep(REMOTE_COMMAND_POLL_INTERVAL) => {
                ensure_not_cancelled(cancelled)?;
            }
        }
    }
}

async fn close_channel_bounded(channel: &mut impl SshChannel) {
    let _ = tokio::time::timeout(CHANNEL_CLOSE_TIMEOUT, channel.close()).await;
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

#[cfg(test)]
mod tests {
    use super::{REMOTE_COMMAND_POLL_INTERVAL, RemoteCommandTimeout, exec_channel_with_input};
    use anyhow::Result;
    use async_trait::async_trait;
    use ssh::{ChannelEvent, PtyConfig, SshChannel};
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time::Instant;

    struct FakeChannel {
        events: VecDeque<ChannelEvent>,
        recv_hangs: bool,
        close_hangs: bool,
        close_called: Arc<AtomicBool>,
    }

    impl FakeChannel {
        fn successful() -> Self {
            Self {
                events: VecDeque::from([
                    ChannelEvent::Data(b"ok".to_vec()),
                    ChannelEvent::ExtendedData {
                        ext: 1,
                        data: b"warning".to_vec(),
                    },
                    ChannelEvent::ExitStatus(0),
                    ChannelEvent::Eof,
                ]),
                recv_hangs: false,
                close_hangs: false,
                close_called: Arc::new(AtomicBool::new(false)),
            }
        }

        fn hanging(close_hangs: bool) -> Self {
            Self {
                events: VecDeque::new(),
                recv_hangs: true,
                close_hangs,
                close_called: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl SshChannel for FakeChannel {
        async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
            Ok(())
        }

        async fn exec(&mut self, _command: &str) -> Result<()> {
            Ok(())
        }

        async fn request_shell(&mut self) -> Result<()> {
            Ok(())
        }

        async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
            Ok(())
        }

        async fn send_data(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Option<ChannelEvent> {
            if self.recv_hangs {
                std::future::pending().await
            } else {
                self.events.pop_front()
            }
        }

        async fn eof(&mut self) -> Result<()> {
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.close_called.store(true, Ordering::Relaxed);
            if self.close_hangs {
                std::future::pending().await
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn command_collects_output_and_exit_status() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut channel = FakeChannel::successful();

        let output = exec_channel_with_input(
            &mut channel,
            "true",
            b"protected",
            cancelled,
            Some(Instant::now() + Duration::from_secs(1)),
        )
        .await
        .expect("command output");

        assert_eq!("ok", output.stdout);
        assert_eq!("warning", output.stderr);
        assert_eq!(0, output.exit_status);
    }

    #[tokio::test]
    async fn command_deadline_reports_the_hanging_stage() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut channel = FakeChannel::hanging(false);

        let error = exec_channel_with_input(
            &mut channel,
            "true",
            &[],
            cancelled,
            Some(Instant::now() + Duration::from_millis(20)),
        )
        .await
        .expect_err("hanging receive should time out");

        assert!(error.is::<RemoteCommandTimeout>());
        assert!(
            error
                .to_string()
                .contains("waiting for remote command output")
        );
    }

    #[tokio::test]
    async fn cancellation_wins_before_a_later_deadline() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_from_task = cancelled.clone();
        let mut channel = FakeChannel::hanging(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_from_task.store(true, Ordering::Relaxed);
        });

        let error = exec_channel_with_input(
            &mut channel,
            "true",
            &[],
            cancelled,
            Some(Instant::now() + Duration::from_secs(1)),
        )
        .await
        .expect_err("cancelled receive should stop");

        assert!(error.is::<crate::TransferCancelled>());
        assert!(REMOTE_COMMAND_POLL_INTERVAL < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn hanging_close_is_bounded_after_timeout() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut channel = FakeChannel::hanging(true);
        let close_called = channel.close_called.clone();

        let result = tokio::time::timeout(Duration::from_millis(100), async {
            let result = exec_channel_with_input(
                &mut channel,
                "true",
                &[],
                cancelled,
                Some(Instant::now() + Duration::from_millis(20)),
            )
            .await;
            super::close_channel_bounded(&mut channel).await;
            result
        })
        .await
        .expect("cleanup should not hang");

        assert!(
            result
                .expect_err("command should time out")
                .is::<RemoteCommandTimeout>()
        );
        assert!(close_called.load(Ordering::Relaxed));
    }
}
