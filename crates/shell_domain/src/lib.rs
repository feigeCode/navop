//! `navopShell` 的领域实现。
//!
//! 当前覆盖 KnownHosts 与本地终端；SSH/SFTP/端口转发等较重的域
//! 后续按同样方式加入。

rust_i18n::i18n!("locales", fallback = "en");

mod connection_forms;
mod connection_openers;
mod session_logs;
mod settings;
mod tab_restore;

use connection_forms::ShellConnectionFormProvider;
use connection_openers::{FtpConnectionOpener, SerialConnectionOpener, TelnetConnectionOpener};
use session_logs::SessionLogsPage;
use settings::ShellSettingsProvider;

use domain_api::{
    ConnectionOpener, Domain, DomainId, DomainRegistry, HomeContext, NotifyLevel, TabIntent,
    TabOpener, TerminalProfileKind,
};
use gpui::{App, AppContext, Window};
use known_hosts_view::KnownHostsPage;
use one_core::settings::{AppSettings, LocalTerminalProfileKind};
use one_core::storage::{
    ConnectionType, RemoteDesktopProtocol as StoredRemoteDesktopProtocol, StoredConnection,
};
use one_core::tab_container::{TabItem, TabOpenMode};
use port_forwarding_view::{PortForwardingTab, PortForwardingTabConfig};
use remote_desktop::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol};
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use std::sync::Arc;
use terminal::{local_config_from_settings, local_config_from_settings_with_profile};
use terminal_view::{TerminalWorkspace, TerminalWorkspaceEvent};

pub struct ShellDomain;

fn register_mcp_toolsets() {
    mcp_host::register_toolset(
        mcp_host::McpToolset::Terminal,
        Arc::new(|cx: &App| {
            let Some(registry) = terminal_view::public_mcp::registry(cx) else {
                return Vec::new();
            };
            vec![
                mcp_host::runtime_provider(public_mcp::tools::terminal_exec_tool_registry(
                    registry.clone(),
                )),
                mcp_host::runtime_provider(public_mcp::tools::terminal_read_tool_registry(
                    registry.clone(),
                )),
                mcp_host::runtime_provider(public_mcp::tools::terminal_control_tool_registry(
                    registry.clone(),
                )),
                mcp_host::runtime_provider(public_mcp::tools::terminal_write_keys_tool_registry(
                    registry.clone(),
                )),
                mcp_host::runtime_provider(public_mcp::tools::remote_ops_tool_registry(registry)),
            ]
        }),
    );
    mcp_host::register_toolset(
        mcp_host::McpToolset::Sftp,
        Arc::new(|cx: &App| {
            let Some(repo) = mcp_host::connection_repository(cx) else {
                return Vec::new();
            };
            vec![mcp_host::runtime_provider(
                runtime_sftp::sftp_tools::sftp_tool_registry(repo),
            )]
        }),
    );

    mcp_host::register_toolset(
        mcp_host::McpToolset::Connections,
        Arc::new(|cx: &App| {
            let Some(repo) = mcp_host::connection_repository(cx) else {
                return Vec::new();
            };
            let workspaces = mcp_host::workspace_repository(cx);
            vec![mcp_host::runtime_provider(
                navop_runtime::connections::connection_tool_registry_with_workspaces(
                    repo, workspaces,
                ),
            )]
        }),
    );
}

impl Domain for ShellDomain {
    fn id(&self) -> DomainId {
        DomainId::SHELL
    }

    fn register(&self, registry: &mut DomainRegistry) {
        registry.register_tab_opener(Arc::new(KnownHostsTabOpener));
        registry.register_tab_opener(Arc::new(TerminalTabOpener));
        registry.register_tab_opener(Arc::new(SessionLogsTabOpener));
        registry.register_connection_opener(ConnectionType::SshSftp, Arc::new(SshConnectionOpener));
        registry.register_connection_opener(ConnectionType::Ftp, Arc::new(FtpConnectionOpener));
        registry
            .register_connection_opener(ConnectionType::Serial, Arc::new(SerialConnectionOpener));
        registry
            .register_connection_opener(ConnectionType::Telnet, Arc::new(TelnetConnectionOpener));
        registry.register_connection_opener(
            ConnectionType::PortForwarding,
            Arc::new(PortForwardingConnectionOpener),
        );
        registry.register_connection_opener(
            ConnectionType::Rdp,
            Arc::new(RemoteDesktopConnectionOpener),
        );
        registry.register_connection_opener(
            ConnectionType::Vnc,
            Arc::new(RemoteDesktopConnectionOpener),
        );
        registry.register_form_provider(Arc::new(ShellConnectionFormProvider));
        registry.register_settings_provider(Arc::new(ShellSettingsProvider));
        // Tab 会话恢复：把上次关闭时的终端 / SFTP / 远程桌面等标签重建出来。
        registry.register_tab_restorer(Arc::new(tab_restore::ShellTabRestorer));
        register_mcp_toolsets();
    }

    fn init(&self, _cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }
}

struct SessionLogsTabOpener;

impl SessionLogsTabOpener {
    fn supports(intent: &TabIntent) -> bool {
        matches!(intent, TabIntent::SessionLogs)
    }
}

impl TabOpener for SessionLogsTabOpener {
    fn open(
        &self,
        intent: &TabIntent,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports(intent) {
            return false;
        }
        home.open_tab_lazy(
            "session-logs".to_string(),
            mode,
            window,
            cx,
            Box::new(move |window, cx| {
                let page = cx.new(|cx| SessionLogsPage::new(window, cx));
                TabItem::new("session-logs", "home", page)
            }),
        )
    }
}

struct KnownHostsTabOpener;

impl KnownHostsTabOpener {
    fn supports(intent: &TabIntent) -> bool {
        matches!(intent, TabIntent::KnownHosts)
    }
}

impl TabOpener for KnownHostsTabOpener {
    fn open(
        &self,
        intent: &TabIntent,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports(intent) {
            return false;
        }
        let page = cx.new(|cx| KnownHostsPage::new(window, cx));
        let item = TabItem::from_dyn("known-hosts", "home", Arc::new(page));
        home.open_tab(item, mode, window, cx)
    }
}

struct TerminalTabOpener;

impl TerminalTabOpener {
    fn supports(intent: &TabIntent) -> bool {
        matches!(intent, TabIntent::Terminal { .. })
    }
}

impl TabOpener for TerminalTabOpener {
    fn open(
        &self,
        intent: &TabIntent,
        _mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports(intent) {
            return false;
        }
        let TabIntent::Terminal { profile } = intent else {
            return false;
        };
        let settings = AppSettings::global(cx);
        let config = match profile.as_ref().map(|profile| profile.kind) {
            Some(kind) => {
                local_config_from_settings_with_profile(settings, local_profile_kind(kind), None)
            }
            None => local_config_from_settings(settings, None),
        };
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                home.notify(
                    NotifyLevel::Error,
                    &format!("本地终端配置无效：{error}"),
                    window,
                    cx,
                );
                return false;
            }
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let tab_id = format!("terminal-{timestamp}");
        let tab_index = home.next_tab_index("Terminal", cx);
        let view = cx.new(|cx| TerminalWorkspace::new_with_index(config, tab_index, window, cx));
        let item = TabItem::from_dyn(tab_id, "home", Arc::new(view));
        home.open_tab_with_focus(item, window, cx)
    }
}

struct SshConnectionOpener;

impl ConnectionOpener for SshConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::SshSftp {
            return false;
        }
        let connection_id = connection.id.unwrap_or(0);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let tab_id = format!("ssh-terminal-{connection_id}-{timestamp}");
        let tab_index = home.next_tab_index(&connection.name, cx);
        let sync_path = AppSettings::global(cx).terminal_sync_path_with_terminal;
        let terminal = cx.new(|cx| {
            TerminalWorkspace::new_ssh_with_index(
                connection.clone(),
                tab_index,
                window,
                cx,
                None,
                sync_path,
            )
        });
        let window_handle = window.window_handle();
        let subscription = cx.subscribe(&terminal, move |_, event: &TerminalWorkspaceEvent, cx| {
            let Some(host) = app_kit::shell_host() else {
                return;
            };
            match event {
                TerminalWorkspaceEvent::OpenSftp(connection) => {
                    let connection = connection.clone();
                    let _ = window_handle.update(cx, move |_, window, cx| {
                        host.open_sftp(connection, window, cx);
                    });
                }
                TerminalWorkspaceEvent::SaveAsConnection(connection) => {
                    let connection = connection.clone();
                    let _ = window_handle.update(cx, move |_, window, cx| {
                        host.save_temporary_connection(connection, window, cx);
                    });
                }
            }
        });
        home.keep_subscription(subscription);
        let item = TabItem::new(tab_id, "ssh", terminal);
        home.open_tab(item, mode, window, cx)
    }

    fn open_sftp(
        &self,
        connection: &StoredConnection,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !matches!(
            connection.connection_type,
            ConnectionType::SshSftp | ConnectionType::Ftp
        ) {
            return false;
        }
        connection_openers::TabEnvironment {
            mode: TabOpenMode::Activate,
            home,
            window,
            cx,
        }
        .open_sftp_view(connection)
    }
}

struct PortForwardingConnectionOpener;

impl ConnectionOpener for PortForwardingConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if connection.connection_type != ConnectionType::PortForwarding {
            return false;
        }
        let Ok(config) = port_forwarding_config(connection, home) else {
            return false;
        };
        let connection_id = connection.id.unwrap_or(0);
        let tab_id = format!("port-forwarding-{connection_id}");
        home.open_tab_lazy(
            tab_id.clone(),
            mode,
            window,
            cx,
            Box::new(move |_window, cx| {
                let tab = cx.new(|cx| PortForwardingTab::new(config, cx));
                TabItem::new(tab_id, "home", tab)
            }),
        )
    }
}

struct RemoteDesktopConnectionOpener;

impl ConnectionOpener for RemoteDesktopConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let protocol = match connection.connection_type {
            ConnectionType::Rdp => RemoteDesktopProtocol::Rdp,
            ConnectionType::Vnc => RemoteDesktopProtocol::Vnc,
            _ => return false,
        };
        // 已注册的 RDP/VNC 域不再回退到 main 的旧实现；失败提示由宿主完成。
        let Some(connection) = home.resolve_connection_credentials(connection, window, cx) else {
            return true;
        };
        let Some(options) = remote_desktop_options(&connection, protocol) else {
            return true;
        };
        let connection_id = connection.id.unwrap_or(0);
        let tab_id = remote_desktop_tab_id(protocol, connection_id);
        let tab_index = home.next_tab_index(&connection.name, cx);
        let title = connection.name.clone();
        let tab_kind = remote_desktop_tab_kind(protocol);
        let window_handle = window.window_handle();
        home.open_tab_lazy(
            tab_id.clone(),
            mode,
            window,
            cx,
            Box::new(move |_window, cx| {
                let view = cx.new(move |cx| {
                    RemoteDesktopView::new(
                        RemoteDesktopViewConfig {
                            options,
                            title,
                            tab_index,
                        },
                        window_handle,
                        cx,
                    )
                });
                TabItem::new(tab_id, tab_kind, view)
            }),
        )
    }
}

fn remote_desktop_options(
    connection: &StoredConnection,
    protocol: RemoteDesktopProtocol,
) -> Option<RemoteDesktopConnectionOptions> {
    let mut params = connection.to_remote_desktop_params().ok()?;
    params.protocol = match protocol {
        RemoteDesktopProtocol::Rdp => StoredRemoteDesktopProtocol::Rdp,
        RemoteDesktopProtocol::Vnc => StoredRemoteDesktopProtocol::Vnc,
    };
    Some(RemoteDesktopConnectionOptions::from_storage_params(params))
}

fn remote_desktop_tab_kind(protocol: RemoteDesktopProtocol) -> &'static str {
    match protocol {
        RemoteDesktopProtocol::Rdp => "rdp",
        RemoteDesktopProtocol::Vnc => "vnc",
    }
}

fn remote_desktop_tab_id(protocol: RemoteDesktopProtocol, connection_id: i64) -> String {
    format!("{}-{}", remote_desktop_tab_kind(protocol), connection_id)
}

fn port_forwarding_config(
    connection: &StoredConnection,
    home: &mut dyn HomeContext,
) -> anyhow::Result<PortForwardingTabConfig> {
    connection
        .id
        .ok_or_else(|| anyhow::anyhow!("missing connection id"))?;
    let params = connection.to_port_forwarding_params()?;
    let ssh_connection = home
        .connections()
        .iter()
        .find(|conn| conn.id == Some(params.ssh_connection_id))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("referenced SSH connection is missing"))?;
    let runtime = home
        .port_forwarding_runtime()
        .ok_or_else(|| anyhow::anyhow!("port forwarding runtime is unavailable"))?;
    PortForwardingTabConfig::new(connection.clone(), ssh_connection, runtime)
}

fn local_profile_kind(kind: TerminalProfileKind) -> LocalTerminalProfileKind {
    match kind {
        TerminalProfileKind::System => LocalTerminalProfileKind::System,
        TerminalProfileKind::PowerShell => LocalTerminalProfileKind::PowerShell,
        TerminalProfileKind::Cmd => LocalTerminalProfileKind::Cmd,
        TerminalProfileKind::Wsl => LocalTerminalProfileKind::Wsl,
        TerminalProfileKind::GitBash => LocalTerminalProfileKind::GitBash,
        TerminalProfileKind::Zsh => LocalTerminalProfileKind::Zsh,
        TerminalProfileKind::Bash => LocalTerminalProfileKind::Bash,
        TerminalProfileKind::Fish => LocalTerminalProfileKind::Fish,
        TerminalProfileKind::Nushell => LocalTerminalProfileKind::Nushell,
        TerminalProfileKind::Custom => LocalTerminalProfileKind::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_domain_registers_its_tab_restorer() {
        let mut registry = DomainRegistry::default();
        ShellDomain.register(&mut registry);
        assert_eq!(1, registry.tab_restorers().len());
        let keys = registry.tab_restorers()[0].content_keys();
        for key in [
            "Terminal",
            "SFTP",
            "RemoteDesktop",
            "KnownHosts",
            "SessionLogs",
        ] {
            assert!(keys.contains(&key), "缺少 {key}");
        }
    }

    #[test]
    fn shell_domain_has_stable_id() {
        assert_eq!(ShellDomain.id(), DomainId::SHELL);
    }

    #[test]
    fn shell_domain_registers_ssh_connection_opener() {
        let mut registry = DomainRegistry::default();
        ShellDomain.register(&mut registry);
        assert!(registry.has_connection_opener(ConnectionType::SshSftp));
    }

    #[test]
    fn shell_domain_registers_terminal_and_file_openers() {
        let mut registry = DomainRegistry::default();
        ShellDomain.register(&mut registry);
        assert!(registry.has_connection_opener(ConnectionType::Ftp));
        assert!(registry.has_connection_opener(ConnectionType::Serial));
        assert!(registry.has_connection_opener(ConnectionType::Telnet));
    }

    #[test]
    fn shell_domain_registers_port_forwarding_opener() {
        let mut registry = DomainRegistry::default();
        ShellDomain.register(&mut registry);
        assert!(registry.has_connection_opener(ConnectionType::PortForwarding));
    }

    #[test]
    fn shell_domain_registers_remote_desktop_openers() {
        let mut registry = DomainRegistry::default();
        ShellDomain.register(&mut registry);
        assert!(registry.has_connection_opener(ConnectionType::Rdp));
        assert!(registry.has_connection_opener(ConnectionType::Vnc));
    }

    #[test]
    fn remote_desktop_tab_id_is_stable_per_connection_and_protocol() {
        assert_eq!(
            remote_desktop_tab_id(RemoteDesktopProtocol::Rdp, 42),
            "rdp-42"
        );
        assert_eq!(
            remote_desktop_tab_id(RemoteDesktopProtocol::Vnc, 42),
            "vnc-42"
        );
        assert_ne!(
            remote_desktop_tab_id(RemoteDesktopProtocol::Rdp, 42),
            remote_desktop_tab_id(RemoteDesktopProtocol::Vnc, 42)
        );
    }

    #[test]
    fn session_logs_opener_only_accepts_session_logs_intent() {
        assert!(SessionLogsTabOpener::supports(&TabIntent::SessionLogs));
        assert!(!SessionLogsTabOpener::supports(&TabIntent::KnownHosts));
    }

    #[test]
    fn known_hosts_opener_only_accepts_known_hosts_intent() {
        assert!(KnownHostsTabOpener::supports(&TabIntent::KnownHosts));
        assert!(!KnownHostsTabOpener::supports(&TabIntent::PortForwarding));
    }

    #[test]
    fn terminal_opener_only_accepts_terminal_intent() {
        assert!(TerminalTabOpener::supports(&TabIntent::Terminal {
            profile: None
        }));
        assert!(!TerminalTabOpener::supports(&TabIntent::KnownHosts));
    }
}
