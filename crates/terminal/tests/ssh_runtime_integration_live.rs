//! 真机 SSH 运行时 Shell Integration 集成测试。
//!
//! 验证 meatshell 式运行时注入的完整生命周期：
//! 只读探测 → 首屏输出注入 → 回显抑制 → ready marker 就绪 → 命令记录 → 远端零文件写入。
//!
//! 通过环境变量 `NAVOP_LIVE_SSH` 提供目标（缺省时使用内网默认值），格式：
//! `NAVOP_LIVE_SSH=user:password@host:port`
//!
//! 运行：`cargo test -p terminal --test ssh_runtime_integration_live -- --ignored --nocapture`

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ssh::{
    HostKeyPolicy, HostKeyVerifier, PtyConfig, SshAuth, SshChannel, SshClient, SshConnectConfig,
    SshSessionManager,
};
use terminal::SshBackend;

struct LiveTarget {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl LiveTarget {
    fn from_env() -> Self {
        let raw = std::env::var("NAVOP_LIVE_SSH")
            .unwrap_or_else(|_| "root:IA@seeyon@2023@113@10.1.131.181:22".to_string());
        // user:password@host:port，密码里可能含 '@'，从右往左拆。
        let (userinfo, hostport) = raw.rsplit_once('@').expect("NAVOP_LIVE_SSH 需要包含 '@'");
        let (user, password) = userinfo.split_once(':').expect("需要 user:password");
        let (host, port) = hostport
            .rsplit_once(':')
            .map(|(h, p)| (h.to_string(), p.parse().expect("端口必须是数字")))
            .unwrap_or((hostport.to_string(), 22));
        Self {
            host,
            port,
            username: user.to_string(),
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
        }
    }
}

/// 远端快照：~/.config/onetcli 与 rc 文件的指纹，用于证明零写入。
async fn remote_snapshot(channel: &mut dyn SshChannel) -> Result<String> {
    channel
        .exec("ls -la $HOME/.config/onetcli 2>&1; ls -la $HOME/.bashrc $HOME/.zshrc $HOME/.profile 2>/dev/null; md5sum $HOME/.bashrc $HOME/.zshrc $HOME/.profile 2>/dev/null; printf '__SNAPSHOT_DONE__'")
        .await?;
    let mut output = Vec::new();
    loop {
        match channel.recv().await {
            Some(ssh::ChannelEvent::Data(data)) => output.extend_from_slice(&data),
            Some(ssh::ChannelEvent::Eof)
            | Some(ssh::ChannelEvent::Close)
            | Some(ssh::ChannelEvent::ExitStatus(_))
            | None => break,
            _ => {}
        }
    }
    Ok(String::from_utf8_lossy(&output).to_string())
}

#[tokio::test]
#[ignore = "需要可达的真机 SSH 服务器"]
async fn live_ssh_runtime_injection_completes_without_remote_writes() -> Result<()> {
    let target = LiveTarget::from_env();
    let manager = Arc::new(SshSessionManager::new(target.config()));

    // 1. 连接前快照（通过 exec channel 读取）；若存在旧版持久注入则先自动卸载。
    {
        let client = manager.client().await?;
        let mut guard = client.lock().await;
        let mut probe = guard.open_channel().await?;
        let mut before = remote_snapshot(&mut probe).await?;
        probe.close().await?;
        if before.contains("shell_integration.sh") {
            println!("检测到旧版持久注入残留，先执行卸载……");
            drop(guard);
            SshBackend::uninstall_shell_integration(manager.clone()).await?;
            println!("卸载完成");
            let client = manager.client().await?;
            let mut guard = client.lock().await;
            let mut probe = guard.open_channel().await?;
            before = remote_snapshot(&mut probe).await?;
            probe.close().await?;
            drop(guard);
        }
        println!("=== 连接前快照 ===\n{before}");
        assert!(
            !before.contains("shell_integration.sh"),
            "卸载后远端不应残留持久注入文件: {before}"
        );
    }

    // 2. 建立交互通道（走新的探测 + 裸 PTY 路径）。
    let connect_started = std::time::Instant::now();
    let (_client, mut channel, shell_integration_requested) =
        SshBackend::establish_channel_for_test(&manager, &PtyConfig::default(), Some(1), false)
            .await
            .context("建立交互通道失败")?;
    println!("交互通道建立耗时: {:?}", connect_started.elapsed());
    assert!(
        shell_integration_requested,
        "Ubuntu 默认 shell 应探测为支持注入"
    );

    // 3. 模拟 actor：读首屏 → 注入 → 抑制回显直到 ready marker。
    // ready marker 已证明 hook 安装完成；部分提示符框架不会补出 133;B，
    // 因此连接就绪不能再依赖它。
    use terminal::test_support::{
        FilteredShellOutput, RuntimeShellIntegration, ShellIntegrationReady,
    };

    let mut integration = RuntimeShellIntegration::new(shell_integration_requested);
    let mut saw_ready_marker = false;
    let mut bootstrap_sent = false;
    let mut injected = false;
    let setup_started = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    while tokio::time::Instant::now() < deadline {
        let event = tokio::select! {
            event = channel.recv() => event,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        let Some(ssh::ChannelEvent::Data(data)) = event else {
            continue;
        };
        if !bootstrap_sent && integration.should_inject(&data, true, false) {
            use ssh::SshChannel as _;
            channel
                .send_data(integration.bootstrap_command())
                .await
                .context("Bootstrap 命令发送失败")?;
            integration.begin_bootstrap();
            bootstrap_sent = true;
            continue;
        }
        match integration.filter_output(data) {
            FilteredShellOutput::Suppressed => continue,
            FilteredShellOutput::BootstrapReady => {
                use ssh::SshChannel as _;
                channel
                    .send_data(integration.injection_command())
                    .await
                    .context("注入命令发送失败")?;
                integration.begin_injection();
                injected = true;
                continue;
            }
            FilteredShellOutput::Forward { data, ready } => {
                assert!(
                    matches!(
                        ready,
                        ShellIntegrationReady::None | ShellIntegrationReady::Integrated
                    ),
                    "真机注入不应失败或超时降级"
                );
                let text = String::from_utf8_lossy(&data);
                assert!(
                    !text.contains("__ONETCLI_RUNTIME_SETUP_1"),
                    "注入命令回显不得泄漏到转发输出: {text:?}"
                );
                if ready == ShellIntegrationReady::Integrated {
                    saw_ready_marker = true;
                    break;
                }
            }
        }
    }
    assert!(bootstrap_sent, "应完成 Bootstrap 探测");
    assert!(injected, "应完成注入");
    if !saw_ready_marker {
        let suppressed = integration.suppressed_output();
        let tail_start = suppressed.len().saturating_sub(4096);
        let tail = String::from_utf8_lossy(&suppressed[tail_start..]);
        eprintln!(
            "未收到 ready/failure marker；suppressed_len={} tail={:?}",
            suppressed.len(),
            tail.as_ref()
        );
    }
    assert!(
        saw_ready_marker,
        "应在超时前收到 ready marker（集成脚本执行完成）"
    );
    println!("运行时注入就绪耗时: {:?}", setup_started.elapsed());
    assert!(integration.accepts_terminal_input());

    // 4. 命令记录：执行 echo，等待 OSC 1337;Command=<base64>。
    channel.send_data(b"echo LIVE_INTEGRATION_OK\r").await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut recorded_command = None;
    while recorded_command.is_none() && tokio::time::Instant::now() < deadline {
        let event = tokio::select! {
            event = channel.recv() => event,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        let Some(ssh::ChannelEvent::Data(data)) = event else {
            continue;
        };
        // 集成完成后 filter_output 直接转发，可安全解析 OSC。
        match integration.filter_output(data) {
            FilteredShellOutput::Forward { data, .. } => {
                let text = String::from_utf8_lossy(&data).to_string();
                for payload in text
                    .split("\x1b]")
                    .filter(|p| p.starts_with("1337;Command="))
                {
                    let encoded = payload
                        .trim_start_matches("1337;Command=")
                        .trim_end_matches('\x07')
                        .trim_end_matches('\x1b');
                    use base64::Engine;
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        recorded_command = Some(String::from_utf8_lossy(&decoded).to_string());
                    }
                }
            }
            FilteredShellOutput::Suppressed | FilteredShellOutput::BootstrapReady => {}
        }
    }
    assert_eq!(
        recorded_command.as_deref(),
        Some("echo LIVE_INTEGRATION_OK"),
        "应通过 OSC 1337 记录用户命令"
    );

    let _ = channel.close().await;

    // 5. 连接后快照：与连接前一致（零写入）。
    {
        let client = manager.client().await?;
        let mut guard = client.lock().await;
        let mut probe = guard.open_channel().await?;
        let after = remote_snapshot(&mut probe).await?;
        probe.close().await?;
        println!("=== 连接后快照 ===\n{after}");
        assert!(
            !after.contains("shell_integration.sh"),
            "运行时注入不得在远端创建文件"
        );
        assert!(
            !after.contains("# BEGIN ONETCLI SHELL INTEGRATION"),
            "运行时注入不得修改 rc 文件"
        );
    }

    println!("真机运行时注入全链路验证通过 ✅");
    Ok(())
}
