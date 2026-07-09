use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

use ssh::{
    ChannelEvent, PtyConfig, ShellIntegrationSetup, SshChannel, SshClient, SshSessionManager,
};

use crate::exec_capture::SshTerminalExecCapture;
use crate::osc::{OscEvent, extract_osc_events};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::shell_integration::{
    embedded_shell_integration_script, normalized_shell_integration_script,
};
use crate::{
    TerminalBackend, TerminalExecCompletion, TerminalExecHandle, TerminalExecOutput,
    TerminalExecRequest, TerminalInputHandle, TerminalSize,
};

/// 整个 shell integration 安装流程的硬超时，避免远端受限或挂死卡住连接。
const SHELL_INTEGRATION_SETUP_TIMEOUT: Duration = Duration::from_secs(10);

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

fn is_channel_open_failure(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("channel open") || msg.contains("maxsessions")
}

fn is_timeout_failure(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("deadline has elapsed")
        || msg.contains("i/o timeout")
}

fn add_connect_error_context(err: anyhow::Error) -> anyhow::Error {
    if is_channel_open_failure(&err) {
        return err.context(
            "服务器拒绝打开新 channel，可能是 MaxSessions 限制（可尝试在 SSH server 设置更大值）",
        );
    }

    if is_timeout_failure(&err) {
        return err.context("连接超时，检查网络/代理/跳板机可达性");
    }

    err
}

fn extract_marker_value(output: &str, marker: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(marker).map(str::to_string))
}

fn build_shell_integration_setup_script(
    script: &str,
    success_marker: &str,
    home_marker: &str,
    session_marker: &str,
    shell_marker: &str,
) -> String {
    let script = normalized_shell_integration_script(script);
    let script = shell_single_quote(&script);
    let managed_block = shell_single_quote(&managed_shell_integration_block());
    let success_marker = shell_single_quote(success_marker);
    let home_marker = shell_single_quote(home_marker);
    let session_marker = shell_single_quote(session_marker);
    let shell_marker = shell_single_quote(shell_marker);

    format!(
        concat!(
            "set -e\n",
            "config_dir=\"$HOME/.config/onetcli\"\n",
            "integration_path=\"$config_dir/shell_integration.sh\"\n",
            "managed_block={managed_block}\n",
            "mkdir -p \"$config_dir\"\n",
            "printf %s {script} > \"$integration_path\"\n",
            "install_onetcli_block() {{\n",
            "    rc_file=\"$1\"\n",
            "    [ -n \"$rc_file\" ] || return 0\n",
            "    tmp_file=\"$rc_file.onetcli.$$\"\n",
            "    if [ -f \"$rc_file\" ]; then\n",
            "        awk '\n",
            "            $0 == \"# BEGIN ONETCLI SHELL INTEGRATION\" {{ skip = 1; next }}\n",
            "            $0 == \"# END ONETCLI SHELL INTEGRATION\" {{ skip = 0; next }}\n",
            "            skip != 1 {{ print }}\n",
            "        ' \"$rc_file\" > \"$tmp_file\"\n",
            "    else\n",
            "        : > \"$tmp_file\"\n",
            "    fi\n",
            "    printf '%s\\n' \"$managed_block\" >> \"$tmp_file\"\n",
            "    cat \"$tmp_file\" > \"$rc_file\"\n",
            "    rm -f \"$tmp_file\"\n",
            "}}\n",
            "install_bash_login_block() {{\n",
            "    for rc_file in \"$HOME/.bash_profile\" \"$HOME/.bash_login\" \"$HOME/.profile\"; do\n",
            "        if [ -f \"$rc_file\" ]; then\n",
            "            install_onetcli_block \"$rc_file\"\n",
            "            return 0\n",
            "        fi\n",
            "    done\n",
            "    install_onetcli_block \"$HOME/.bash_profile\"\n",
            "}}\n",
            "login_shell=\"${{SHELL:-}}\"\n",
            "shell_name=\"${{login_shell##*/}}\"\n",
            "case \"$shell_name\" in\n",
            "    bash)\n",
            "        install_onetcli_block \"$HOME/.bashrc\"\n",
            "        install_bash_login_block\n",
            "        ;;\n",
            "    zsh)\n",
            "        install_onetcli_block \"$HOME/.zshrc\"\n",
            "        ;;\n",
            "    *)\n",
            "        install_onetcli_block \"$HOME/.bashrc\"\n",
            "        install_bash_login_block\n",
            "        install_onetcli_block \"$HOME/.zshrc\"\n",
            "        ;;\n",
            "esac\n",
            "printf '%s%s\\n' {home_marker} \"$HOME\"\n",
            "printf '%s%s\\n' {session_marker} \"$config_dir\"\n",
            "printf '%s%s\\n' {shell_marker} \"$login_shell\"\n",
            "printf '%s\\n' {success_marker}\n"
        ),
        script = script,
        managed_block = managed_block,
        success_marker = success_marker,
        home_marker = home_marker,
        session_marker = session_marker,
        shell_marker = shell_marker,
    )
}

fn build_shell_integration_uninstall_script(success_marker: &str, home_marker: &str) -> String {
    let success_marker = shell_single_quote(success_marker);
    let home_marker = shell_single_quote(home_marker);

    format!(
        concat!(
            "set -e\n",
            "config_dir=\"$HOME/.config/onetcli\"\n",
            "remove_onetcli_block() {{\n",
            "    rc_file=\"$1\"\n",
            "    [ -f \"$rc_file\" ] || return 0\n",
            "    tmp_file=\"$rc_file.onetcli.$$\"\n",
            "    awk '\n",
            "        $0 == \"# BEGIN ONETCLI SHELL INTEGRATION\" {{ skip = 1; next }}\n",
            "        $0 == \"# END ONETCLI SHELL INTEGRATION\" {{ skip = 0; next }}\n",
            "        skip != 1 {{ print }}\n",
            "    ' \"$rc_file\" > \"$tmp_file\"\n",
            "    cat \"$tmp_file\" > \"$rc_file\"\n",
            "    rm -f \"$tmp_file\"\n",
            "}}\n",
            "remove_onetcli_block \"$HOME/.bashrc\"\n",
            "remove_onetcli_block \"$HOME/.bash_profile\"\n",
            "remove_onetcli_block \"$HOME/.bash_login\"\n",
            "remove_onetcli_block \"$HOME/.profile\"\n",
            "remove_onetcli_block \"$HOME/.zshrc\"\n",
            "rm -f \"$config_dir/shell_integration.sh\"\n",
            "rm -rf \"$config_dir/sessions\"\n",
            "rmdir \"$config_dir\" 2>/dev/null || true\n",
            "printf '%s%s\\n' {home_marker} \"$HOME\"\n",
            "printf '%s\\n' {success_marker}\n"
        ),
        success_marker = success_marker,
        home_marker = home_marker,
    )
}

fn managed_shell_integration_block() -> String {
    concat!(
        "# BEGIN ONETCLI SHELL INTEGRATION\n",
        "case \"$-\" in\n",
        "    *i*) __onetcli_interactive=1 ;;\n",
        "    *) __onetcli_interactive= ;;\n",
        "esac\n",
        "if [ -n \"$__onetcli_interactive\" ] && { [ -n \"${BASH_VERSION:-}\" ] || [ -n \"${ZSH_VERSION:-}\" ]; }; then\n",
        "    __onetcli_si=\"$HOME/.config/onetcli/shell_integration.sh\"\n",
        "    [ -r \"$__onetcli_si\" ] && . \"$__onetcli_si\"\n",
        "    unset __onetcli_si\n",
        "fi\n",
        "unset __onetcli_interactive\n",
        "# END ONETCLI SHELL INTEGRATION\n",
    )
    .to_string()
}

fn format_numbered_script(script: &str) -> String {
    script
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>2} | {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_setup_failure_context(script: &str, stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    format!(
        "stderr: {}\nstdout: {}\nsetup script:\n{}",
        stderr.trim(),
        stdout.trim(),
        format_numbered_script(script)
    )
}

enum SshCommand {
    Write(Vec<u8>),
    Resize(TerminalSize),
    Shutdown,
}

pub struct SshBackend {
    command_tx: UnboundedSender<SshCommand>,
    exec_capture: SshTerminalExecCapture,
}

impl SshBackend {
    pub async fn uninstall_shell_integration(
        session_manager: Arc<SshSessionManager>,
    ) -> anyhow::Result<()> {
        let client = session_manager
            .client()
            .await
            .map_err(add_connect_error_context)?;
        let result = {
            let mut guard = client.lock().await;
            Self::uninstall_shell_integration_for_client(&mut *guard).await
        };
        if result.is_ok() {
            session_manager.invalidate().await;
        }
        result.map_err(add_connect_error_context)
    }

    pub async fn connect(
        session_manager: Arc<SshSessionManager>,
        pty_config: PtyConfig,
        connection_id: Option<i64>,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        event_tx: UnboundedSender<TerminalEvent>,
        notify_tx: UnboundedSender<()>,
        on_disconnect: Option<UnboundedSender<()>>,
        init_commands: Option<String>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<Self> {
        let (client, mut channel) = Self::establish_channel(
            &session_manager,
            &pty_config,
            connection_id,
            disable_shell_integration,
        )
        .await
        .map_err(add_connect_error_context)?;
        // 关联变量，避免 clippy 警告未使用。
        let _keep_client = client;

        // ③ init_commands 改为等 shell ready 后发送
        let pending_init = init_commands;

        let (command_tx, mut command_rx) = unbounded_channel::<SshCommand>();
        let exec_capture = SshTerminalExecCapture::new();
        let task_exec_capture = exec_capture.clone();

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
                                let osc_events = extract_osc_events(&data);
                                task_exec_capture.record_chunk(&data, &osc_events);
                                for osc_event in &osc_events {
                                    tracing::debug!(
                                        target: "terminal.history_prompt.osc",
                                        event = ?osc_event,
                                        "ssh backend observed osc event"
                                    );
                                    match osc_event {
                                        OscEvent::WorkingDirChanged(path) => {
                                            let _ = event_tx.send(TerminalEvent::WorkingDirChanged(path.clone()));
                                        }
                                        OscEvent::PromptStart => {
                                            let _ = event_tx.send(TerminalEvent::PromptStart);
                                        }
                                        OscEvent::InputStart => {
                                            let _ = event_tx.send(TerminalEvent::InputStart);
                                            // 133;B: prompt 渲染完，用户可以输入了
                                            // 第一次收到时发送 init_commands
                                            if !shell_ready {
                                                shell_ready = true;
                                            }
                                        }
                                        OscEvent::CommandStart => {
                                            let _ = event_tx.send(TerminalEvent::CommandStart);
                                        }
                                        OscEvent::CommandFinished { exit_code } => {
                                            // 133;D: 命令执行完毕
                                            let _ = event_tx.send(
                                                TerminalEvent::CommandFinished { exit_code: *exit_code }
                                            );
                                        }
                                        OscEvent::CommandRecorded(command) => {
                                            let _ = event_tx.send(
                                                TerminalEvent::CommandRecorded(command.clone())
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
                                task_exec_capture.finish_active_on_disconnect();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !shutdown {
                let _ = session_manager.invalidate().await;
            }
            if let Some(tx) = on_disconnect {
                let _ = tx.send(());
            }
        });

        Ok(Self {
            command_tx,
            exec_capture,
        })
    }

    /// 获取一个 interactive channel，封装了"channel open 失败时 invalidate 并重试一次"的重连逻辑。
    /// 同时把首次 setup 成功的 `ShellIntegrationSetup` 写回 manager，供其他 terminal 复用。
    async fn establish_channel(
        session_manager: &Arc<SshSessionManager>,
        pty_config: &PtyConfig,
        connection_id: Option<i64>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<(Arc<tokio::sync::Mutex<ssh::RusshClient>>, ssh::RusshChannel)> {
        let mut attempt = 0usize;
        loop {
            let client = session_manager.client().await?;
            let cached = session_manager.cached_shell_integration().await;

            let result = {
                let mut guard = client.lock().await;
                Self::prepare_ssh_channel(
                    &mut *guard,
                    pty_config,
                    connection_id,
                    cached,
                    disable_shell_integration,
                )
                .await
            };

            match result {
                Ok((channel, new_setup)) => {
                    if let Some(setup) = new_setup {
                        session_manager.set_shell_integration(&client, setup).await;
                    }
                    return Ok((client, channel));
                }
                Err(err) if attempt == 0 && is_channel_open_failure(&err) => {
                    tracing::warn!(
                        target: "terminal.ssh.connect",
                        error = %err,
                        "channel open 失败，尝试 invalidate 并重连一次（可能是 MaxSessions 限制）"
                    );
                    session_manager.invalidate().await;
                    attempt += 1;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn prepare_ssh_channel<C: SshClient>(
        client: &mut C,
        pty_config: &PtyConfig,
        connection_id: Option<i64>,
        cached: Option<ShellIntegrationSetup>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<(C::Channel, Option<ShellIntegrationSetup>)> {
        let (setup, new_setup) = if disable_shell_integration {
            // 用户在连接配置里显式关闭了 shell integration:先 best-effort 卸载远端
            // managed block,再走裸 request_shell 路径。
            Self::try_uninstall_shell_integration(client).await;
            (None, None)
        } else if let Some(cached) = cached {
            (Some(cached), None)
        } else {
            // 首次连接：尝试安装 integration，失败降级为"无 integration"分支。
            let setup = Self::try_install_shell_integration(client, connection_id).await;
            (setup.clone(), setup)
        };

        let mut channel = client.open_channel().await?;
        Self::start_interactive_shell(&mut channel, pty_config, setup.as_ref()).await?;
        Ok((channel, new_setup))
    }

    /// 打开一个临时 channel 跑 integration 安装脚本。任何失败（open 失败 / setup 出错 / 超时）
    /// 都只记 warn 日志并返回 `None`，不阻断 SSH 连接。
    async fn try_install_shell_integration<C: SshClient>(
        client: &mut C,
        connection_id: Option<i64>,
    ) -> Option<ShellIntegrationSetup> {
        Self::try_install_shell_integration_with_timeout(
            client,
            connection_id,
            SHELL_INTEGRATION_SETUP_TIMEOUT,
        )
        .await
    }

    async fn try_install_shell_integration_with_timeout<C: SshClient>(
        client: &mut C,
        connection_id: Option<i64>,
        timeout: Duration,
    ) -> Option<ShellIntegrationSetup> {
        let mut setup_channel = match client.open_channel().await {
            Ok(ch) => ch,
            Err(err) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    error = %err,
                    "打开 shell integration 安装通道失败，降级为无 integration 模式"
                );
                return None;
            }
        };

        let setup_future = Self::run_shell_integration_setup(&mut setup_channel, connection_id);
        let result = match tokio::time::timeout(timeout, setup_future).await {
            Ok(r) => r,
            Err(_) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    timeout_secs = timeout.as_secs(),
                    "shell integration 安装超时，降级为无 integration 模式"
                );
                let _ = setup_channel.close().await;
                return None;
            }
        };
        let _ = setup_channel.close().await;

        match result {
            Ok(setup) => Some(setup),
            Err(err) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    error = %err,
                    "shell integration 安装失败，降级为无 integration 模式（终端仍可使用，\
                     但无 prompt hook / 命令记录）"
                );
                None
            }
        }
    }

    async fn try_uninstall_shell_integration<C: SshClient>(client: &mut C) {
        if let Err(err) = Self::uninstall_shell_integration_for_client(client).await {
            tracing::warn!(
                target: "terminal.ssh.setup",
                error = %err,
                "卸载 shell integration 失败，继续使用裸 shell"
            );
        }
    }

    async fn uninstall_shell_integration_for_client<C: SshClient>(
        client: &mut C,
    ) -> anyhow::Result<()> {
        let mut channel = client.open_channel().await?;
        let result = tokio::time::timeout(
            SHELL_INTEGRATION_SETUP_TIMEOUT,
            Self::run_shell_integration_uninstall(&mut channel),
        )
        .await;
        let _ = channel.close().await;

        match result {
            Ok(result) => result,
            Err(_) => anyhow::bail!(
                "shell integration uninstall timed out after {}s",
                SHELL_INTEGRATION_SETUP_TIMEOUT.as_secs()
            ),
        }
    }

    async fn run_shell_integration_uninstall(channel: &mut dyn SshChannel) -> anyhow::Result<()> {
        const SUCCESS_MARKER: &str = "__ONETCLI_UNINSTALL_OK__";
        const HOME_MARKER: &str = "__ONETCLI_UNINSTALL_HOME__=";
        let uninstall_script =
            build_shell_integration_uninstall_script(SUCCESS_MARKER, HOME_MARKER);
        let cmd = format!("sh -c {}", shell_single_quote(&uninstall_script));

        channel.exec(&cmd).await?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        loop {
            match channel.recv().await {
                Some(ChannelEvent::Data(data)) => stdout.extend(data),
                Some(ChannelEvent::ExtendedData { data, .. }) => stderr.extend(data),
                Some(ChannelEvent::ExitStatus(code)) => {
                    let context = format_setup_failure_context(&uninstall_script, &stdout, &stderr);
                    anyhow::ensure!(
                        code == 0,
                        "shell integration uninstall failed with exit code {code}: {context}",
                    );
                }
                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                    let output = String::from_utf8_lossy(&stdout);
                    let context = format_setup_failure_context(&uninstall_script, &stdout, &stderr);
                    anyhow::ensure!(
                        output.contains(SUCCESS_MARKER),
                        "shell integration uninstall ended before confirming completion: {context}",
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// 在 PTY 之前写入 integration 脚本。
    async fn run_shell_integration_setup(
        channel: &mut dyn SshChannel,
        connection_id: Option<i64>,
    ) -> anyhow::Result<ShellIntegrationSetup> {
        const SUCCESS_MARKER: &str = "__ONETCLI_SETUP_OK__";
        const HOME_MARKER: &str = "__ONETCLI_HOME__=";
        const SESSION_MARKER: &str = "__ONETCLI_SESSION_DIR__=";
        const SHELL_MARKER: &str = "__ONETCLI_LOGIN_SHELL__=";
        let script = embedded_shell_integration_script();
        let setup_script = build_shell_integration_setup_script(
            &script,
            SUCCESS_MARKER,
            HOME_MARKER,
            SESSION_MARKER,
            SHELL_MARKER,
        );
        let cmd = format!("sh -c {}", shell_single_quote(&setup_script));

        channel.exec(&cmd).await?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        loop {
            match channel.recv().await {
                Some(ChannelEvent::Data(data)) => stdout.extend(data),
                Some(ChannelEvent::ExtendedData { data, .. }) => {
                    stderr.extend(data);
                }
                Some(ChannelEvent::ExitStatus(code)) => {
                    let context = format_setup_failure_context(&setup_script, &stdout, &stderr);
                    if code != 0 {
                        tracing::error!(
                            target: "terminal.ssh.setup",
                            connection_id,
                            exit_code = code,
                            %context,
                            "shell integration setup failed"
                        );
                    }
                    anyhow::ensure!(
                        code == 0,
                        "shell integration setup failed with exit code {code}: {context}",
                    );
                }
                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                    let output = String::from_utf8_lossy(&stdout);
                    let context = format_setup_failure_context(&setup_script, &stdout, &stderr);
                    if !output.contains(SUCCESS_MARKER) {
                        tracing::error!(
                            target: "terminal.ssh.setup",
                            connection_id,
                            %context,
                            "shell integration setup ended before success marker"
                        );
                    }
                    anyhow::ensure!(
                        output.contains(SUCCESS_MARKER),
                        "shell integration setup ended before confirming completion: {context}",
                    );
                    let home_dir = extract_marker_value(&output, HOME_MARKER)
                        .ok_or_else(|| anyhow::anyhow!("missing setup home directory marker"))?;
                    let session_dir = extract_marker_value(&output, SESSION_MARKER)
                        .ok_or_else(|| anyhow::anyhow!("missing setup session directory marker"))?;
                    let login_shell = extract_marker_value(&output, SHELL_MARKER)
                        .filter(|value| !value.trim().is_empty());
                    return Ok(ShellIntegrationSetup {
                        home_dir,
                        session_dir,
                        login_shell,
                    });
                }
                _ => {}
            }
        }
    }

    async fn start_interactive_shell(
        channel: &mut dyn SshChannel,
        pty_config: &PtyConfig,
        _setup: Option<&ShellIntegrationSetup>,
    ) -> anyhow::Result<()> {
        channel.request_pty(pty_config).await?;
        channel.request_shell().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::parse_osc_payload;
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use ssh::SshConnectConfig;
    use std::collections::VecDeque;
    use std::fs;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use tokio::time::sleep;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ChannelOp {
        Exec,
        SetEnv(String, String),
        RequestPty,
        RequestShell,
        Close,
    }

    #[derive(Default)]
    struct MockChannelState {
        ops: Vec<ChannelOp>,
        events: VecDeque<ChannelEvent>,
        exec_consumes_session: bool,
        recv_delay: Option<Duration>,
    }

    struct MockChannel {
        state: Arc<Mutex<MockChannelState>>,
    }

    impl MockChannel {
        fn new(
            events: impl IntoIterator<Item = ChannelEvent>,
            exec_consumes_session: bool,
        ) -> (Self, Arc<Mutex<MockChannelState>>) {
            Self::new_with_delay(events, exec_consumes_session, None)
        }

        fn new_with_delay(
            events: impl IntoIterator<Item = ChannelEvent>,
            exec_consumes_session: bool,
            recv_delay: Option<Duration>,
        ) -> (Self, Arc<Mutex<MockChannelState>>) {
            let state = Arc::new(Mutex::new(MockChannelState {
                ops: Vec::new(),
                events: events.into_iter().collect(),
                exec_consumes_session,
                recv_delay,
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
                return Err(anyhow!(
                    "cannot request shell after exec on the same session"
                ));
            }
            Ok(())
        }

        async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
            self.state
                .lock()
                .expect("mock channel state should lock")
                .ops
                .push(ChannelOp::SetEnv(_name.to_string(), _value.to_string()));
            Ok(())
        }

        async fn send_data(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Option<ChannelEvent> {
            let delay = {
                self.state
                    .lock()
                    .expect("mock channel state should lock")
                    .recv_delay
            };
            if let Some(delay) = delay {
                sleep(delay).await;
            }
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
    async fn prepare_ssh_channel_uses_plain_request_shell_for_zsh_after_setup() {
        let (setup_channel, setup_state) = MockChannel::new(
            [
                ChannelEvent::Data(
                    b"__ONETCLI_HOME__=/tmp/home\n__ONETCLI_SESSION_DIR__=/tmp/home/.config/onetcli\n__ONETCLI_LOGIN_SHELL__=/bin/zsh\n__ONETCLI_SETUP_OK__\n"
                        .to_vec(),
                ),
                ChannelEvent::ExitStatus(0),
            ],
            true,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([setup_channel, interactive_channel]);

        let result = SshBackend::prepare_ssh_channel(
            &mut client,
            &PtyConfig::default(),
            Some(42),
            None,
            false,
        )
        .await;

        let (_channel, new_setup) =
            result.expect("安装 shell integration 不应占用交互 shell 的 channel");
        assert!(
            new_setup.is_some(),
            "首次成功安装应返回新 setup 以便写入 manager 缓存"
        );
        assert_eq!(
            recorded_ops(&setup_state),
            vec![ChannelOp::Exec, ChannelOp::Close]
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
    }

    #[tokio::test]
    async fn prepare_ssh_channel_uses_plain_request_shell_for_bash_after_setup() {
        let (setup_channel, setup_state) = MockChannel::new(
            [
                ChannelEvent::Data(
                    b"__ONETCLI_HOME__=/tmp/home\n__ONETCLI_SESSION_DIR__=/tmp/home/.config/onetcli\n__ONETCLI_LOGIN_SHELL__=/bin/bash\n__ONETCLI_SETUP_OK__\n"
                        .to_vec(),
                ),
                ChannelEvent::ExitStatus(0),
            ],
            true,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([setup_channel, interactive_channel]);

        let result = SshBackend::prepare_ssh_channel(
            &mut client,
            &PtyConfig::default(),
            Some(42),
            None,
            false,
        )
        .await;

        let (_channel, new_setup) = result.expect("bash shell wrapper 应通过独立交互 channel 启动");
        assert!(new_setup.is_some());
        assert_eq!(
            recorded_ops(&setup_state),
            vec![ChannelOp::Exec, ChannelOp::Close]
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
    }

    #[tokio::test]
    async fn run_shell_integration_setup_fails_without_success_signal() {
        let (mut channel, _) = MockChannel::new([ChannelEvent::Close], false);

        let result = SshBackend::run_shell_integration_setup(&mut channel, Some(42)).await;

        assert!(
            result.is_err(),
            "仅收到 Close 不能视为 shell integration 安装成功"
        );
    }

    #[tokio::test]
    async fn run_shell_integration_setup_accepts_success_marker_before_close() {
        let (mut channel, _) = MockChannel::new(
            [
                ChannelEvent::Data(
                    b"__ONETCLI_HOME__=/tmp/home\n__ONETCLI_SESSION_DIR__=/tmp/home/.config/onetcli\n__ONETCLI_LOGIN_SHELL__=/bin/zsh\n__ONETCLI_SETUP_OK__\n"
                        .to_vec(),
                ),
                ChannelEvent::Close,
            ],
            false,
        );

        let result = SshBackend::run_shell_integration_setup(&mut channel, Some(42)).await;

        assert!(result.is_ok(), "收到成功标记后应接受无 ExitStatus 的 Close");
    }

    #[tokio::test]
    async fn run_shell_integration_setup_exit_status_error_includes_numbered_script_context() {
        let (mut channel, _) = MockChannel::new(
            [
                ChannelEvent::ExtendedData {
                    ext: 1,
                    data: b"sh: 7: cannot create /tmp/x: Directory nonexistent".to_vec(),
                },
                ChannelEvent::ExitStatus(1),
            ],
            false,
        );

        let error = SshBackend::run_shell_integration_setup(&mut channel, Some(42))
            .await
            .expect_err("exit code 1 应返回带上下文的错误");
        let message = error.to_string();

        assert!(
            message.contains("sh: 7: cannot create /tmp/x: Directory nonexistent"),
            "错误消息应保留远端 stderr，实际: {message}"
        );
        assert!(
            message.contains("setup script:"),
            "错误消息应包含编号后的 setup script，实际: {message}"
        );
        assert!(
            message.contains("7 |"),
            "错误消息应包含脚本行号，实际: {message}"
        );
    }

    #[tokio::test]
    async fn prepare_ssh_channel_falls_back_to_plain_shell_when_setup_fails() {
        // setup 通道返回 exit 1，应该被降级路径捕获：interactive 通道不 set_env、只 pty+shell。
        let (setup_channel, setup_state) = MockChannel::new(
            [
                ChannelEvent::ExtendedData {
                    ext: 1,
                    data:
                        b"mkdir: cannot create directory '/root/.config/onetcli': Permission denied"
                            .to_vec(),
                },
                ChannelEvent::ExitStatus(1),
            ],
            false,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([setup_channel, interactive_channel]);

        let (_ch, new_setup) = SshBackend::prepare_ssh_channel(
            &mut client,
            &PtyConfig::default(),
            Some(42),
            None,
            false,
        )
        .await
        .expect("setup 失败时 prepare_ssh_channel 不应整体失败");

        assert!(
            new_setup.is_none(),
            "失败降级不应向 manager 写入任何 integration 缓存"
        );
        assert_eq!(
            recorded_ops(&setup_state),
            vec![ChannelOp::Exec, ChannelOp::Close],
            "setup 通道仍应正常跑完 exec + close"
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell],
            "降级路径绝对不能调 set_env，也不能走 bash wrapper exec"
        );
    }

    #[tokio::test]
    async fn prepare_ssh_channel_skips_setup_when_cache_hit() {
        // 命中缓存：只应打开 1 个 channel（interactive）。mock client 只提供 1 个 channel。
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([interactive_channel]);

        let cached = ShellIntegrationSetup {
            home_dir: "/tmp/home".into(),
            session_dir: "/tmp/home/.config/onetcli".into(),
            login_shell: Some("/bin/zsh".into()),
        };

        let (_ch, new_setup) = SshBackend::prepare_ssh_channel(
            &mut client,
            &PtyConfig::default(),
            Some(42),
            Some(cached),
            false,
        )
        .await
        .expect("缓存命中时应直接复用 setup 结果");

        assert!(
            new_setup.is_none(),
            "缓存命中不应再向 manager 写入新的 integration"
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
    }

    #[tokio::test]
    async fn prepare_ssh_channel_skips_setup_when_disabled() {
        // 用户在连接配置里显式关闭 shell integration:先卸载远端 managed block,再开
        // interactive channel 走裸 PTY + shell;且不向 manager 写入任何缓存。
        let (uninstall_channel, uninstall_state) = MockChannel::new(
            [
                ChannelEvent::Data(
                    b"__ONETCLI_UNINSTALL_HOME__=/tmp/home\n__ONETCLI_UNINSTALL_OK__\n".to_vec(),
                ),
                ChannelEvent::Close,
            ],
            false,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let mut client = MockClient::new([uninstall_channel, interactive_channel]);

        let (_ch, new_setup) = SshBackend::prepare_ssh_channel(
            &mut client,
            &PtyConfig::default(),
            Some(42),
            None,
            true,
        )
        .await
        .expect("禁用 shell integration 时仍应建立 interactive channel");

        assert!(
            new_setup.is_none(),
            "禁用路径不应向 manager 写入任何 integration 缓存"
        );
        assert_eq!(
            recorded_ops(&uninstall_state),
            vec![ChannelOp::Exec, ChannelOp::Close],
            "禁用路径应先 best-effort 卸载远端 integration"
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell],
            "禁用路径只跑 pty + shell,不调 set_env / exec wrapper"
        );
    }

    #[tokio::test]
    async fn try_install_shell_integration_times_out_in_ten_seconds() {
        // 测试里用短 timeout 验证逻辑；生产路径仍走 10s 常量。
        let (setup_channel, _) = MockChannel::new_with_delay(
            [ChannelEvent::Data(b"pending...".to_vec())],
            false,
            Some(Duration::from_millis(20)),
        );
        let mut client = MockClient::new([setup_channel]);

        let res = SshBackend::try_install_shell_integration_with_timeout(
            &mut client,
            Some(42),
            Duration::from_millis(1),
        )
        .await;
        assert!(res.is_none(), "10s 超时后应降级为 None");
    }

    #[test]
    fn add_connect_error_context_wraps_channel_open_failures() {
        let err = anyhow!("channel open failed: administratively prohibited");
        let message = add_connect_error_context(err).to_string();

        assert!(
            message.contains("服务器拒绝打开新 channel"),
            "channel open 错误应补充 MaxSessions 提示，实际: {message}"
        );
    }

    #[test]
    fn add_connect_error_context_wraps_timeout_failures() {
        let err = anyhow!("dial tcp 10.0.0.8:22: i/o timeout");
        let message = add_connect_error_context(err).to_string();

        assert!(
            message.contains("连接超时"),
            "timeout 错误应补充网络/代理排查提示，实际: {message}"
        );
    }

    #[test]
    fn build_shell_integration_setup_command_writes_bash_managed_blocks_idempotently() {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-shell-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let home_dir = temp_dir.join("home");
        fs::create_dir_all(&home_dir).expect("应创建 home 目录");
        let bashrc_path = home_dir.join(".bashrc");
        let bash_profile_path = home_dir.join(".bash_profile");
        fs::write(
            &bashrc_path,
            "# user bashrc\n# BEGIN ONETCLI SHELL INTEGRATION\nold\n# END ONETCLI SHELL INTEGRATION\n",
        )
        .expect("应写入用户 bashrc");
        fs::write(&bash_profile_path, "# user bash_profile\n").expect("应写入用户 bash_profile");
        let script = "echo 'quoted'\nPS1='prompt'\n";
        let command = build_shell_integration_setup_script(
            script,
            "__TEST_OK__",
            "__HOME__=",
            "__SESSION__=",
            "__SHELL__=",
        );

        for _ in 0..2 {
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .env("HOME", &home_dir)
                .env("SHELL", "/bin/bash")
                .output()
                .expect("应能执行本地 shell setup 命令");

            assert!(
                output.status.success(),
                "shell setup 命令应成功执行: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                format!(
                    "__HOME__={}\n__SESSION__={}\n__SHELL__=/bin/bash\n__TEST_OK__",
                    home_dir.display(),
                    home_dir.join(".config/onetcli").display()
                )
            );
        }

        let integration_path = home_dir.join(".config/onetcli/shell_integration.sh");
        assert_eq!(
            fs::read_to_string(&integration_path).expect("应写入 integration 文件"),
            script
        );

        let bashrc = fs::read_to_string(&bashrc_path).expect("应读取用户 bashrc");
        assert!(
            bashrc.starts_with("# user bashrc\n"),
            "应保留用户 bashrc 原内容，实际: {bashrc}"
        );
        assert!(
            !bashrc.contains("\nold\n"),
            "再次安装应替换旧 managed block，实际: {bashrc}"
        );
        assert_eq!(
            bashrc.matches("# BEGIN ONETCLI SHELL INTEGRATION").count(),
            1,
            "重复安装不应追加多个 begin marker: {bashrc}"
        );
        assert_eq!(
            bashrc.matches("# END ONETCLI SHELL INTEGRATION").count(),
            1,
            "重复安装不应追加多个 end marker: {bashrc}"
        );
        assert!(
            bashrc.contains("case \"$-\" in"),
            "managed block 应先判断是否交互 shell，避免 rsync/scp 等非交互通道被 OSC 污染: {bashrc}"
        );
        assert!(
            bashrc.contains("shell_integration.sh"),
            "managed block 应 source 持久 integration 脚本: {bashrc}"
        );

        let bash_profile = fs::read_to_string(&bash_profile_path).expect("应读取用户 bash_profile");
        assert!(
            bash_profile.starts_with("# user bash_profile\n"),
            "应保留用户 bash_profile 原内容，实际: {bash_profile}"
        );
        assert!(
            bash_profile.contains("# BEGIN ONETCLI SHELL INTEGRATION"),
            "bash login shell 启动文件也应写入 managed block: {bash_profile}"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn build_shell_integration_setup_command_normalizes_crlf_script_contents() {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-shell-setup-crlf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let home_dir = temp_dir.join("home");
        fs::create_dir_all(&home_dir).expect("应创建 home 目录");

        let command = build_shell_integration_setup_script(
            "echo one\r\necho two\r\n",
            "__TEST_OK__",
            "__HOME__=",
            "__SESSION__=",
            "__SHELL__=",
        );

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", &home_dir)
            .output()
            .expect("应能执行本地 shell setup 命令");

        assert!(
            output.status.success(),
            "shell setup 命令应成功执行: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let integration_path = home_dir.join(".config/onetcli/shell_integration.sh");
        assert_eq!(
            fs::read_to_string(&integration_path).expect("应写入 integration 文件"),
            "echo one\necho two\n",
            "session integration 脚本应统一写成 LF，避免远端 bash 解析 $'\\r'"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn build_shell_integration_uninstall_command_removes_managed_blocks_and_scripts() {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-shell-uninstall-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        let home_dir = temp_dir.join("home");
        let config_dir = home_dir.join(".config/onetcli");
        fs::create_dir_all(config_dir.join("sessions/42")).expect("应创建 legacy sessions");
        fs::write(
            config_dir.join("shell_integration.sh"),
            "echo integration\n",
        )
        .expect("应写入 integration 脚本");
        fs::write(
            config_dir.join("sessions/42/shell_integration.sh"),
            "legacy\n",
        )
        .expect("应写入 legacy session 脚本");

        let managed = managed_shell_integration_block();
        fs::write(
            home_dir.join(".bashrc"),
            format!("before bash\n{managed}after bash\n"),
        )
        .expect("应写入 bashrc");
        fs::write(
            home_dir.join(".bash_profile"),
            format!("before profile\n{managed}after profile\n"),
        )
        .expect("应写入 bash_profile");
        fs::write(
            home_dir.join(".zshrc"),
            format!("before zsh\n{managed}after zsh\n"),
        )
        .expect("应写入 zshrc");

        let command =
            build_shell_integration_uninstall_script("__TEST_UNINSTALL_OK__", "__HOME__=");
        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", &home_dir)
            .output()
            .expect("应能执行本地 uninstall 命令");

        assert!(
            output.status.success(),
            "uninstall 命令应成功执行: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("__HOME__={}\n__TEST_UNINSTALL_OK__", home_dir.display())
        );
        assert_eq!(
            fs::read_to_string(home_dir.join(".bashrc")).expect("应读取 bashrc"),
            "before bash\nafter bash\n"
        );
        assert_eq!(
            fs::read_to_string(home_dir.join(".bash_profile")).expect("应读取 bash_profile"),
            "before profile\nafter profile\n"
        );
        assert_eq!(
            fs::read_to_string(home_dir.join(".zshrc")).expect("应读取 zshrc"),
            "before zsh\nafter zsh\n"
        );
        assert!(!config_dir.join("shell_integration.sh").exists());
        assert!(!config_dir.join("sessions").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn run_shell_integration_uninstall_accepts_success_marker_before_close() {
        let (mut channel, _) = MockChannel::new(
            [
                ChannelEvent::Data(
                    b"__ONETCLI_UNINSTALL_HOME__=/tmp/home\n__ONETCLI_UNINSTALL_OK__\n".to_vec(),
                ),
                ChannelEvent::Close,
            ],
            false,
        );

        let result = SshBackend::run_shell_integration_uninstall(&mut channel).await;

        assert!(
            result.is_ok(),
            "收到卸载成功标记后应接受无 ExitStatus 的 Close"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_managed_block_sources_integration_only_for_onetcli_interactive_shells() {
        if Command::new("bash").arg("--version").output().is_err() {
            eprintln!("跳过 bash managed block 测试：当前环境未安装 bash");
            return;
        }
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-bash-managed-block-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let home_dir = temp_dir.join("home");
        fs::create_dir_all(&home_dir).expect("应创建 home 目录");

        let script = "export __ONETCLI_INTEGRATION_LOADED=1\n";
        let command = build_shell_integration_setup_script(
            script,
            "__TEST_OK__",
            "__HOME__=",
            "__SESSION__=",
            "__SHELL__=",
        );
        let setup = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", &home_dir)
            .env("SHELL", "/bin/bash")
            .output()
            .expect("应执行 setup 脚本");
        assert!(
            setup.status.success(),
            "setup 脚本应成功: {}",
            String::from_utf8_lossy(&setup.stderr)
        );

        let non_interactive = Command::new("bash")
            .arg("-c")
            .arg(". \"$HOME/.bashrc\"; echo loaded=${__ONETCLI_INTEGRATION_LOADED:-0}")
            .env("HOME", &home_dir)
            .output()
            .expect("应执行非交互 bash");
        assert!(
            non_interactive.status.success(),
            "非交互 bash 应成功: {}",
            String::from_utf8_lossy(&non_interactive.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&non_interactive.stdout).trim(),
            "loaded=0",
            "非交互 shell 不应 source integration"
        );

        let interactive = Command::new("bash")
            .arg("-i")
            .arg("-c")
            .arg("echo loaded=${__ONETCLI_INTEGRATION_LOADED:-0}")
            .env("HOME", &home_dir)
            .output()
            .expect("应执行交互 bash");

        assert!(
            interactive.status.success(),
            "交互 bash 应成功执行: {}",
            String::from_utf8_lossy(&interactive.stderr)
        );
        assert!(
            String::from_utf8_lossy(&interactive.stdout).contains("loaded=1"),
            "OnetCli 交互 bash 应 source integration，实际 stdout: {}",
            String::from_utf8_lossy(&interactive.stdout)
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn zsh_setup_writes_zshrc_managed_block() {
        if Command::new("zsh").arg("--version").output().is_err() {
            eprintln!("跳过 zsh managed block 测试：当前环境未安装 zsh");
            return;
        }
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-zsh-managed-block-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let home_dir = temp_dir.join("home");
        fs::create_dir_all(&home_dir).expect("应创建 home 目录");
        fs::write(home_dir.join(".zshrc"), "# user zshrc\n").expect("应写入用户 .zshrc");

        let script = "export __ONETCLI_INTEGRATION_LOADED=1\n";
        let command = build_shell_integration_setup_script(
            script,
            "__TEST_OK__",
            "__HOME__=",
            "__SESSION__=",
            "__SHELL__=",
        );
        let setup = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", &home_dir)
            .env("SHELL", "zsh")
            .output()
            .expect("应执行 setup 脚本");
        assert!(
            setup.status.success(),
            "setup 脚本应成功: {}",
            String::from_utf8_lossy(&setup.stderr)
        );

        let zshrc = fs::read_to_string(home_dir.join(".zshrc")).expect("应读取 zshrc");
        assert!(
            zshrc.starts_with("# user zshrc\n"),
            "应保留用户 zshrc 原内容，实际: {zshrc}"
        );
        assert!(
            zshrc.contains("# BEGIN ONETCLI SHELL INTEGRATION"),
            "zshrc 应包含 managed block: {zshrc}"
        );
        assert!(
            zshrc.contains("case \"$-\" in"),
            "zshrc managed block 应保护非交互 shell: {zshrc}"
        );
        assert!(
            !home_dir.join(".zprofile").exists(),
            "zsh 不需要通过 ZDOTDIR/session wrapper 改写 .zprofile"
        );

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

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let tx = self.command_tx.clone();
        Some(TerminalInputHandle::new(move |data| {
            let _ = tx.send(SshCommand::Write(data));
        }))
    }

    fn exec_handle(&self) -> Option<TerminalExecHandle> {
        let tx = self.command_tx.clone();
        let capture = self.exec_capture.clone();
        Some(TerminalExecHandle::new(move |request| {
            exec_in_ssh_terminal(tx.clone(), capture.clone(), request)
        }))
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

fn exec_in_ssh_terminal(
    tx: UnboundedSender<SshCommand>,
    capture: SshTerminalExecCapture,
    request: TerminalExecRequest,
) -> anyhow::Result<TerminalExecOutput> {
    let mut input = request.command.clone().into_bytes();
    if request.submit {
        input.push(b'\n');
    }

    if !request.submit || !request.wait_for_output {
        tx.send(SshCommand::Write(input))
            .map_err(|_| anyhow::anyhow!("terminal input handle is not ready"))?;
        return Ok(TerminalExecOutput {
            completion: TerminalExecCompletion::SubmittedOnly,
            exit_code: None,
            output: String::new(),
            duration_ms: 0,
        });
    }

    let session = capture.start(request.command, request.timeout)?;
    if tx.send(SshCommand::Write(input)).is_err() {
        capture.cancel(session.id());
        return Err(anyhow::anyhow!("terminal input handle is not ready"));
    }
    let result = session.wait();

    Ok(TerminalExecOutput {
        completion: result.completion,
        exit_code: result.exit_code,
        output: result.output,
        duration_ms: result.duration_ms,
    })
}
