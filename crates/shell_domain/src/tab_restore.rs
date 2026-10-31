//! Shell 域的 Tab 会话恢复：把上次关闭时的标签还原成真实视图。
//!
//! 快照里每条的 `id` / `from` / `key` 都来自打开时用的 `TabItem`，所以这里
//! 只按 `key` 分派，再用 `from` + `id` 反解连接。连接查不到（已删除）或
//! 视图构造失败时返回 `None`，该标签被静默跳过。

use domain_api::TabRestorer;
use gpui::{App, AppContext as _, Entity, Window};
use known_hosts_view::KnownHostsPage;
use one_core::settings::AppSettings;
use one_core::storage::traits::Repository as _;
use one_core::storage::{ConnectionRepository, GlobalStorageState, StoredConnection};
use one_core::tab_container::{TabContentView, TabItemState};
use remote_desktop::RemoteDesktopProtocol;
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use sftp_view::{SftpView, SftpViewEvent};
use std::sync::Arc;
use terminal::local_config_from_settings;
use terminal_view::{TerminalWorkspace, TerminalWorkspaceEvent};

use crate::session_logs::SessionLogsPage;

/// shell 产品注册到 `app_kit::tabs` 的恢复器。
pub struct ShellTabRestorer;

impl TabRestorer for ShellTabRestorer {
    fn content_keys(&self) -> Vec<&'static str> {
        vec![
            "Terminal",
            "SFTP",
            "RemoteDesktop",
            "KnownHosts",
            "SessionLogs",
        ]
    }

    fn restore(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>> {
        match state.key.as_ref() {
            "KnownHosts" => Some(Arc::new(cx.new(|cx| KnownHostsPage::new(window, cx)))),
            "SessionLogs" => Some(Arc::new(cx.new(|cx| SessionLogsPage::new(window, cx)))),
            "Terminal" => restore_terminal(state, window, cx),
            "SFTP" => restore_sftp(state, window, cx),
            "RemoteDesktop" => restore_remote_desktop(state, window, cx),
            _ => None,
        }
    }
}

/// 每个标签打开时用的 `TabItem::from`，恢复时靠它区分同一 content_key 下的子类型。
fn restore_terminal(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    match state.from.as_ref() {
        // 本地终端的 id 形如 `terminal-{millis}`，没有连接可查。
        "home" => {
            let config = local_config_from_settings(AppSettings::global(cx), None).ok()?;
            let view = cx.new(|cx| TerminalWorkspace::new_with_index(config, None, window, cx));
            Some(Arc::new(view))
        }
        "ssh" => {
            let connection = lookup_connection(state.id.as_ref(), &["ssh-terminal-"], cx)?;
            let sync_path = AppSettings::global(cx).terminal_sync_path_with_terminal;
            let terminal = cx.new(|cx| {
                TerminalWorkspace::new_ssh_with_index(connection, None, window, cx, None, sync_path)
            });
            subscribe_terminal_events(&terminal, window, cx);
            Some(Arc::new(terminal))
        }
        "serial" => {
            let connection = lookup_connection(state.id.as_ref(), &["serial-terminal-"], cx)?;
            let view =
                cx.new(|cx| TerminalWorkspace::new_serial_with_index(connection, None, window, cx));
            Some(Arc::new(view))
        }
        "telnet" => {
            let connection = lookup_connection(state.id.as_ref(), &["telnet-terminal-"], cx)?;
            let view =
                cx.new(|cx| TerminalWorkspace::new_telnet_with_index(connection, None, window, cx));
            Some(Arc::new(view))
        }
        _ => None,
    }
}

fn restore_sftp(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    let connection = lookup_connection(state.id.as_ref(), &["sftp-"], cx)?;
    let sftp = cx.new(|cx| SftpView::new_with_index(connection, None, window, cx));
    let window_handle = window.window_handle();
    cx.subscribe(&sftp, move |_, event: &SftpViewEvent, cx| {
        let Some(host) = app_kit::shell_host() else {
            return;
        };
        match event {
            SftpViewEvent::OpenLocalTerminal { working_dir } => {
                let working_dir = working_dir.clone();
                let _ = window_handle.update(cx, move |_, window, cx| {
                    host.open_local_terminal(working_dir, window, cx);
                });
            }
            SftpViewEvent::OpenSshTerminal {
                connection,
                working_dir,
            } => {
                let connection = connection.clone();
                let working_dir = working_dir.clone();
                let _ = window_handle.update(cx, move |_, window, cx| {
                    host.open_ssh_terminal(connection, working_dir, window, cx);
                });
            }
        }
    })
    // 恢复出来的标签没有 Home 代为保管订阅，detach 让它跟随视图存活。
    .detach();
    Some(Arc::new(sftp))
}

fn restore_remote_desktop(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    let protocol = match state.from.as_ref() {
        "rdp" => RemoteDesktopProtocol::Rdp,
        "vnc" => RemoteDesktopProtocol::Vnc,
        _ => return None,
    };
    let connection = lookup_connection(state.id.as_ref(), &["rdp-", "vnc-"], cx)?;
    // 与打开时一致：先解析凭据，解析失败就不恢复该标签。
    let connection = resolve_credentials(&connection, cx)?;
    let options = crate::remote_desktop_options(&connection, protocol)?;
    let window_handle = window.window_handle();
    let title = connection.name.clone();
    let view = cx.new(|cx| {
        RemoteDesktopView::new(
            RemoteDesktopViewConfig {
                options,
                title,
                // 恢复的标签在容器内，不能传 None（那是独立全屏窗口模式）。
                tab_index: Some(0),
            },
            window_handle,
            cx,
        )
    });
    Some(Arc::new(view))
}

/// SSH 终端要在返回后仍能把「打开 SFTP / 另存连接」事件转给宿主。
fn subscribe_terminal_events(terminal: &Entity<TerminalWorkspace>, window: &Window, cx: &mut App) {
    let window_handle = window.window_handle();
    cx.subscribe(terminal, move |_, event: &TerminalWorkspaceEvent, cx| {
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
    })
    .detach();
}

/// 从 tab id 的前缀反解连接 id，例如 `ssh-terminal-42-1700000000000` → `42`。
fn connection_id_from(tab_id: &str, prefixes: &[&str]) -> Option<i64> {
    prefixes.iter().find_map(|prefix| {
        let rest = tab_id.strip_prefix(prefix)?;
        rest.split('-').next()?.parse().ok()
    })
}

/// 按连接 id 查仓库；连接已被删除时返回 `None`。
fn lookup_connection(tab_id: &str, prefixes: &[&str], cx: &App) -> Option<StoredConnection> {
    let connection_id = connection_id_from(tab_id, prefixes)?;
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repository = storage.get::<ConnectionRepository>()?;
    repository.get(connection_id).ok().flatten()
}

fn resolve_credentials(connection: &StoredConnection, cx: &App) -> Option<StoredConnection> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repository = storage.get::<ConnectionRepository>()?;
    repository.resolve_runtime_connection(connection).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_restorer_covers_every_shell_tab_key() {
        let keys = ShellTabRestorer.content_keys();
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
    fn connection_id_is_parsed_from_tab_id() {
        assert_eq!(
            connection_id_from("ssh-terminal-42-1700000000000", &["ssh-terminal-"]),
            Some(42)
        );
        assert_eq!(connection_id_from("sftp-7-17000000", &["sftp-"]), Some(7));
        assert_eq!(connection_id_from("rdp-3", &["rdp-", "vnc-"]), Some(3));
        assert_eq!(connection_id_from("vnc-9", &["rdp-", "vnc-"]), Some(9));
    }

    #[test]
    fn unknown_or_foreign_tab_id_yields_no_connection() {
        assert_eq!(connection_id_from("known-hosts", &["ssh-terminal-"]), None);
        assert_eq!(
            connection_id_from("terminal-1700", &["ssh-terminal-"]),
            None
        );
        assert_eq!(
            connection_id_from("ssh-terminal-abc-170", &["ssh-terminal-"]),
            None
        );
    }
}
