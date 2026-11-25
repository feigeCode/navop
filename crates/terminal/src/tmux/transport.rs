use async_trait::async_trait;
use std::collections::VecDeque;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use ssh::SshChannel;

use super::control::TmuxControlTransport;
use super::discovery::shell_command_for_attach;

pub struct LocalTmuxTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl LocalTmuxTransport {
    pub async fn connect(session_id: &str, socket: Option<&str>) -> anyhow::Result<Self> {
        let mut command = Command::new("tmux");
        if let Some(socket) = socket.map(str::trim).filter(|socket| !socket.is_empty()) {
            command.args(["-S", socket]);
        }
        let mut child = command
            .args(["-C", "attach-session", "-t", session_id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("tmux stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("tmux stdout"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }
}

#[async_trait]
impl TmuxControlTransport for LocalTmuxTransport {
    async fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        self.stdin.write_all(line.as_bytes()).await?;
        if !line.ends_with('\n') {
            self.stdin.write_all(b"\n").await?;
        }
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        let bytes = self.stdout.read_until(b'\n', &mut line).await?;
        Ok((bytes != 0).then_some(line))
    }

    async fn shutdown(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

pub struct SshTmuxTransport {
    channel: ssh::RusshChannel,
    pending: VecDeque<u8>,
}

impl SshTmuxTransport {
    pub async fn connect(
        manager: &ssh::SshSessionManager,
        session_id: &str,
        socket: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut channel = manager.open_channel().await?;
        let command = shell_command_for_attach(session_id, socket);
        channel.exec(&command).await?;
        Ok(Self {
            channel,
            pending: VecDeque::new(),
        })
    }
}

#[async_trait]
impl TmuxControlTransport for SshTmuxTransport {
    async fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        let mut data = line.as_bytes().to_vec();
        if !line.ends_with('\n') {
            data.push(b'\n');
        }
        self.channel.send_data(&data).await
    }

    async fn read_line(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        loop {
            if let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line = self.pending.drain(..=index).collect();
                return Ok(Some(line));
            }
            match self.channel.recv().await {
                Some(ssh::ChannelEvent::Data(data)) => self.pending.extend(data),
                Some(ssh::ChannelEvent::ExtendedData { .. }) => {}
                Some(ssh::ChannelEvent::Eof)
                | Some(ssh::ChannelEvent::ExitStatus(_))
                | Some(ssh::ChannelEvent::ExitSignal { .. })
                | Some(ssh::ChannelEvent::Close)
                | None => {
                    if self.pending.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some(self.pending.drain(..).collect()));
                }
            }
        }
    }

    async fn shutdown(&mut self) {
        let _ = self.channel.eof().await;
        let _ = self.channel.close().await;
    }
}
