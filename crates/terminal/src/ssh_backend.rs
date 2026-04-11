use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;

use ssh::{ChannelEvent, PtyConfig, RusshClient, SshChannel, SshClient, SshConnectConfig};

use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::{TerminalBackend, TerminalSize};

/// OSC 事件类型（基于 OSC 133 协议）
#[derive(Debug)]
enum OscEvent {
    PromptStart,                       // 133;A
    InputStart,                        // 133;B
    CommandStart,                      // 133;C
    CommandFinished { exit_code: i32 }, // 133;D;<code>
    WorkingDirChanged(String),         // 7;file://host/path
    CommandRecorded(String),           // 1337;Command=<base64>
}

/// 从字节流中提取所有 OSC 事件（一次 data 里可能含多个）
fn extract_osc_events(data: &[u8]) -> Vec<OscEvent> {
    let text = String::from_utf8_lossy(data);
    let mut events = Vec::new();

    // OSC 格式: ESC ] <payload> BEL  或  ESC ] <payload> ESC \
    // 用简单的状态机扫描
    let mut i = 0;
    let chars: Vec<char> = text.chars().collect();

    while i < chars.len() {
        // 找 ESC ]
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == ']' {
            i += 2;
            let start = i;

            // 找结束符 BEL(\x07) 或 ST(ESC \)
            while i < chars.len() {
                if chars[i] == '\x07' {
                    let payload: String = chars[start..i].iter().collect();
                    if let Some(ev) = parse_osc_payload(&payload) {
                        events.push(ev);
                    }
                    i += 1;
                    break;
                }
                if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                    let payload: String = chars[start..i].iter().collect();
                    if let Some(ev) = parse_osc_payload(&payload) {
                        events.push(ev);
                    }
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    events
}

/// 解析 OSC payload 内容
fn parse_osc_payload(payload: &str) -> Option<OscEvent> {
    // OSC 133 协议：shell 集成标记
    if let Some(rest) = payload.strip_prefix("133;") {
        return match rest {
            "A" => Some(OscEvent::PromptStart),
            "B" => Some(OscEvent::InputStart),
            "C" => Some(OscEvent::CommandStart),
            d if d.starts_with("D;") => {
                let code = d[2..].parse::<i32>().unwrap_or(-1);
                Some(OscEvent::CommandFinished { exit_code: code })
            }
            _ => None,
        };
    }

    // OSC 7：工作目录变更
    if let Some(rest) = payload.strip_prefix("7;file://") {
        // "hostname/path/to/dir" 或 "/path/to/dir"
        let path = rest
            .splitn(2, '/')
            .nth(1)
            .map(|p| format!("/{p}"))
            .unwrap_or_default();
        return Some(OscEvent::WorkingDirChanged(path));
    }

    if let Some(encoded) = payload.strip_prefix("1337;Command=") {
        let command = BASE64_STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())?;
        return Some(OscEvent::CommandRecorded(command));
    }

    None
}

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

fn shell_double_quote(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn build_shell_integration_setup_script(
    script: &str,
    remote_path: &str,
    rc_files: &[&str],
    success_marker: &str,
) -> String {
    let marker = shell_single_quote("# onetcli shell integration");
    let script = shell_single_quote(script);
    let setup_line = shell_single_quote(&format!(
        "[ -f \"{remote_path}\" ] && . \"{remote_path}\""
    ));
    let success_marker = shell_single_quote(success_marker);
    let remote_path = shell_double_quote(remote_path);
    let rc_files = rc_files
        .iter()
        .map(|path| format!("\"{}\"", shell_double_quote(path)))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        concat!(
            "set -e\n",
            "remote_path=\"{remote_path}\"\n",
            "mkdir -p \"$(dirname -- \"$remote_path\")\"\n",
            "printf %s {script} > \"$remote_path\"\n",
            "marker={marker}\n",
            "setup_line={setup_line}\n",
            "for rc_file in {rc_files}; do\n",
            "    if ! grep -qF \"$marker\" \"$rc_file\" 2>/dev/null; then\n",
            "        printf '\\n%s\\n%s\\n' \"$marker\" \"$setup_line\" >> \"$rc_file\"\n",
            "    fi\n",
            "done\n",
            "printf '%s\\n' {success_marker}\n"
        ),
        remote_path = remote_path,
        script = script,
        marker = marker,
        setup_line = setup_line,
        rc_files = rc_files,
        success_marker = success_marker,
    )
}

fn build_shell_integration_setup_command(
    script: &str,
    remote_path: &str,
    rc_files: &[&str],
    success_marker: &str,
) -> String {
    let script = build_shell_integration_setup_script(script, remote_path, rc_files, success_marker);
    format!("sh -c {}", shell_single_quote(&script))
}

enum SshCommand {
    Write(Vec<u8>),
    Resize(TerminalSize),
    Shutdown,
}

pub struct SshBackend {
    command_tx: UnboundedSender<SshCommand>,
}

impl SshBackend {
    pub async fn connect(
        config: SshConnectConfig,
        pty_config: PtyConfig,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        event_tx: UnboundedSender<TerminalEvent>,
        notify_tx: UnboundedSender<()>,
        on_disconnect: Option<UnboundedSender<()>>,
        init_commands: Option<String>,
    ) -> anyhow::Result<Self> {
        let mut client = RusshClient::connect(config).await?;
        let mut channel = Self::prepare_ssh_channel(&mut client, &pty_config).await?;

        // ③ init_commands 改为等 shell ready 后发送
        let pending_init = init_commands;

        let (command_tx, mut command_rx) = unbounded_channel::<SshCommand>();

        // 创建 PtyWrite 回写通道
        let (pty_write_tx, mut pty_write_rx) = unbounded_channel::<Vec<u8>>();
        event_proxy.set_ssh_write_back(pty_write_tx);

        tokio::spawn(async move {
            let mut shutdown = false;
            let mut processor: Processor<StdSyncHandler> = Processor::new();
            // 用来判断 shell 是否已经 ready（收到第一个 133;B 后才发 init_commands）
            let mut shell_ready = false;
            let mut init_sent = false;

            loop {
                tokio::select! {
                    biased;
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            SshCommand::Write(data) => {
                                let send_result = tokio::time::timeout(
                                    Duration::from_secs(30),
                                    channel.send_data(&data)
                                ).await;
                                if send_result.is_err() || send_result.is_ok_and(|r| r.is_err()) {
                                    break;
                                }
                            }
                            SshCommand::Resize(size) => {
                                let _ = channel.resize_pty(size.cols as u32, size.rows as u32).await;
                            }
                            SshCommand::Shutdown => {
                                shutdown = true;
                                let _ = channel.close().await;
                                break;
                            }
                        }
                    }
                    Some(data) = pty_write_rx.recv() => {
                        let send_result = tokio::time::timeout(
                            Duration::from_secs(30),
                            channel.send_data(&data)
                        ).await;
                        if send_result.is_err() || send_result.is_ok_and(|r| r.is_err()) {
                            break;
                        }
                    }
                    event = channel.recv() => {
                        match event {
                            Some(ChannelEvent::Data(data)) | Some(ChannelEvent::ExtendedData { data, .. }) => {
                                // 解析所有 OSC 事件
                                for osc_event in extract_osc_events(&data) {
                                    match osc_event {
                                        OscEvent::WorkingDirChanged(path) => {
                                            let _ = event_tx.send(TerminalEvent::WorkingDirChanged(path));
                                        }
                                        OscEvent::PromptStart => {
                                            // 133;A: shell 准备好显示 prompt
                                        }
                                        OscEvent::InputStart => {
                                            // 133;B: prompt 渲染完，用户可以输入了
                                            // 第一次收到时发送 init_commands
                                            if !shell_ready {
                                                shell_ready = true;
                                            }
                                        }
                                        OscEvent::CommandStart => {
                                            // 133;C: 命令开始执行
                                        }
                                        OscEvent::CommandFinished { exit_code } => {
                                            // 133;D: 命令执行完毕
                                            let _ = event_tx.send(
                                                TerminalEvent::CommandFinished { exit_code }
                                            );
                                        }
                                        OscEvent::CommandRecorded(command) => {
                                            let _ = event_tx.send(
                                                TerminalEvent::CommandRecorded(command)
                                            );
                                        }
                                    }
                                }

                                // shell ready 后发送 init_commands（只发一次）
                                if shell_ready && !init_sent {
                                    init_sent = true;
                                    if let Some(ref commands) = pending_init {
                                        for line in commands.lines() {
                                            if !line.trim().is_empty() {
                                                let mut cmd_data = line.as_bytes().to_vec();
                                                cmd_data.push(b'\n');
                                                let _ = channel.send_data(&cmd_data).await;
                                            }
                                        }
                                    }
                                }

                                processor.advance(&mut *term.lock(), &data);
                                let _ = notify_tx.send(());
                            }
                            Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !shutdown {
                let _ = client.disconnect().await;
            }
            if let Some(tx) = on_disconnect {
                let _ = tx.send(());
            }
        });

        Ok(Self { command_tx })
    }

    async fn prepare_ssh_channel<C: SshClient>(
        client: &mut C,
        pty_config: &PtyConfig,
    ) -> anyhow::Result<C::Channel> {
        let mut setup_channel = client.open_channel().await?;
        let setup_result = Self::run_shell_integration_setup(&mut setup_channel).await;
        let _ = setup_channel.close().await;
        setup_result?;

        let mut channel = client.open_channel().await?;
        channel.request_pty(pty_config).await?;
        channel.request_shell().await?;
        Ok(channel)
    }

    /// 在 PTY 之前写入 integration 脚本。
    async fn run_shell_integration_setup(channel: &mut dyn SshChannel) -> anyhow::Result<()> {
        const SCRIPT: &str = include_str!("shell_integration.sh");
        const REMOTE_PATH: &str = "$HOME/.config/onetcli/shell_integration.sh";
        const SUCCESS_MARKER: &str = "__ONETCLI_SETUP_OK__";
        let cmd = build_shell_integration_setup_command(
            SCRIPT,
            REMOTE_PATH,
            &["$HOME/.bashrc", "$HOME/.zshrc"],
            SUCCESS_MARKER,
        );

        channel.exec(&cmd).await?;

        let mut setup_succeeded = false;
        let mut stderr = Vec::new();

        loop {
            match channel.recv().await {
                Some(ChannelEvent::Data(data)) => {
                    if data.windows(SUCCESS_MARKER.len()).any(|w| w == SUCCESS_MARKER.as_bytes()) {
                        setup_succeeded = true;
                    }
                }
                Some(ChannelEvent::ExtendedData { data, .. }) => {
                    stderr.extend(data);
                }
                Some(ChannelEvent::ExitStatus(code)) => {
                    anyhow::ensure!(
                        code == 0,
                        "shell integration setup failed with exit code {code}: {}",
                        String::from_utf8_lossy(&stderr).trim()
                    );
                    anyhow::ensure!(
                        setup_succeeded,
                        "shell integration setup exited successfully but did not confirm completion"
                    );
                    return Ok(());
                }
                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                    anyhow::ensure!(
                        setup_succeeded,
                        "shell integration setup ended before confirming completion: {}",
                        String::from_utf8_lossy(&stderr).trim()
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use std::fs;
    use std::collections::VecDeque;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ChannelOp {
        Exec,
        RequestPty,
        RequestShell,
        Close,
    }

    #[derive(Default)]
    struct MockChannelState {
        ops: Vec<ChannelOp>,
        events: VecDeque<ChannelEvent>,
        exec_consumes_session: bool,
    }

    struct MockChannel {
        state: Arc<Mutex<MockChannelState>>,
    }

    impl MockChannel {
        fn new(
            events: impl IntoIterator<Item = ChannelEvent>,
            exec_consumes_session: bool,
        ) -> (Self, Arc<Mutex<MockChannelState>>) {
            let state = Arc::new(Mutex::new(MockChannelState {
                ops: Vec::new(),
                events: events.into_iter().collect(),
                exec_consumes_session,
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    #[async_trait]
    impl SshChannel for MockChannel {
        async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::RequestPty);
            if state.exec_consumes_session {
                return Err(anyhow!("cannot request pty after exec on the same session"));
            }
            Ok(())
        }

        async fn exec(&mut self, _command: &str) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::Exec);
            Ok(())
        }

        async fn request_shell(&mut self) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::RequestShell);
            if state.exec_consumes_session {
                return Err(anyhow!("cannot request shell after exec on the same session"));
            }
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
            self.state
                .lock()
                .expect("mock channel state should lock")
                .events
                .pop_front()
        }

        async fn eof(&mut self) -> Result<()> {
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.state
                .lock()
                .expect("mock channel state should lock")
                .ops
                .push(ChannelOp::Close);
            Ok(())
        }
    }

    struct MockClient {
        channels: VecDeque<MockChannel>,
    }

    impl MockClient {
        fn new(channels: impl IntoIterator<Item = MockChannel>) -> Self {
            Self {
                channels: channels.into_iter().collect(),
            }
        }
    }

    #[async_trait]
    impl SshClient for MockClient {
        type Channel = MockChannel;

        async fn connect(_config: SshConnectConfig) -> Result<Self>
        where
            Self: Sized,
        {
            unreachable!("mock client connect is not used in this test")
        }

        async fn open_channel(&mut self) -> Result<Self::Channel> {
            self.channels
                .pop_front()
                .ok_or_else(|| anyhow!("no more mock channels"))
        }

        async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }
    }

    fn recorded_ops(state: &Arc<Mutex<MockChannelState>>) -> Vec<ChannelOp> {
        state
            .lock()
            .expect("mock channel state should lock")
            .ops
            .clone()
    }

    #[tokio::test]
    async fn prepare_ssh_channel_uses_dedicated_setup_channel() {
        let (setup_channel, setup_state) = MockChannel::new(
            [
                ChannelEvent::Data(b"__ONETCLI_SETUP_OK__\n".to_vec()),
                ChannelEvent::ExitStatus(0),
            ],
            true,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([setup_channel, interactive_channel]);

        let result = SshBackend::prepare_ssh_channel(&mut client, &PtyConfig::default()).await;

        assert!(
            result.is_ok(),
            "安装 shell integration 不应占用交互 shell 的 channel"
        );
        assert_eq!(recorded_ops(&setup_state), vec![ChannelOp::Exec, ChannelOp::Close]);
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
    }

    #[tokio::test]
    async fn run_shell_integration_setup_fails_without_success_signal() {
        let (mut channel, _) = MockChannel::new([ChannelEvent::Close], false);

        let result = SshBackend::run_shell_integration_setup(&mut channel).await;

        assert!(
            result.is_err(),
            "仅收到 Close 不能视为 shell integration 安装成功"
        );
    }

    #[tokio::test]
    async fn run_shell_integration_setup_accepts_success_marker_before_close() {
        let (mut channel, _) = MockChannel::new(
            [
                ChannelEvent::Data(b"__ONETCLI_SETUP_OK__\n".to_vec()),
                ChannelEvent::Close,
            ],
            false,
        );

        let result = SshBackend::run_shell_integration_setup(&mut channel).await;

        assert!(result.is_ok(), "收到成功标记后应接受无 ExitStatus 的 Close");
    }

    #[test]
    fn build_shell_integration_setup_command_handles_single_quotes_in_script() {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-shell-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let remote_path = temp_dir.join("shell_integration.sh");
        let bashrc_path = temp_dir.join(".bashrc");
        let script = "echo 'quoted'\nPS1='prompt'\n";
        let command = build_shell_integration_setup_script(
            script,
            &remote_path.to_string_lossy(),
            &[&bashrc_path.to_string_lossy()],
            "__TEST_OK__",
        );

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .expect("应能执行本地 shell setup 命令");

        assert!(
            output.status.success(),
            "shell setup 命令应成功执行: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(&remote_path).expect("应写入 integration 文件"),
            script
        );
        let bashrc = fs::read_to_string(&bashrc_path).expect("应写入 rc 文件");
        assert!(bashrc.contains("# onetcli shell integration"));
        assert!(bashrc.contains(remote_path.to_string_lossy().as_ref()));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "__TEST_OK__");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn parse_osc_payload_decodes_recorded_command() {
        let payload = "1337;Command=Z2l0IHN0YXR1cw==";

        let event = parse_osc_payload(payload);

        match event {
            Some(OscEvent::CommandRecorded(command)) => {
                assert_eq!(command, "git status");
            }
            other => panic!("expected recorded command event, got {other:?}"),
        }
    }

    #[test]
    fn extract_osc_events_keeps_command_recording_between_prompt_events() {
        let events = extract_osc_events(
            b"\x1b]133;A\x07\x1b]1337;Command=Z2l0IHN0YXR1cw==\x07\x1b]133;D;0\x07",
        );

        assert!(matches!(events.first(), Some(OscEvent::PromptStart)));
        assert!(
            matches!(events.get(1), Some(OscEvent::CommandRecorded(cmd)) if cmd == "git status")
        );
        assert!(matches!(
            events.get(2),
            Some(OscEvent::CommandFinished { exit_code: 0 })
        ));
    }
}


impl TerminalBackend for SshBackend {
    fn write(&self, data: Vec<u8>) {
        let _ = self.command_tx.send(SshCommand::Write(data));
    }

    fn resize(&self, size: TerminalSize) {
        tracing::info!(
            "SshBackend::resize: 发送 resize 命令到远程 PTY: {}x{}",
            size.cols,
            size.rows
        );
        let _ = self.command_tx.send(SshCommand::Resize(size));
    }

    fn shutdown(&self) {
        let _ = self.command_tx.send(SshCommand::Shutdown);
    }
}
