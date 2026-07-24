use crate::home_tab::HomePage;
use crate::license::is_feature_enabled;
use crate::onetcli_app::GlobalTabContainer;
use crate::setting_tab::{AppSettings, DatabaseOpenMode, SettingsPanel};
use db_view::database_tab::DatabaseTabView;
use gpui::{App, AppContext, Context, Entity, Window};
use gpui_component::{WindowExt, notification::Notification};
use mongodb_view::MongoTabView;
use notes::NotesView;
use one_core::license::Feature;
use one_core::settings::{LocalTerminalCustomProfile, LocalTerminalProfileKind};
use one_core::storage::{ConnectionType, ProxyConfig, ProxyType, StoredConnection, Workspace};
use one_core::tab_container::{TabContainer, TabItem, TabOpenMode};
use redis_view::RedisTabView;
use remote_desktop::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol};
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use rust_i18n::t;
use sftp_view::{SftpView, SftpViewEvent};
use terminal::{
    local_config_from_custom_profile, local_config_from_settings,
    local_config_from_settings_with_profile,
};
use terminal_view::{
    TerminalConnectionKind, TerminalWorkspace, current_settings as current_terminal_settings,
};

fn redis_tab_open_context(
    open_mode: DatabaseOpenMode,
    conn: &StoredConnection,
    workspace: Option<Workspace>,
    all_connections: &[StoredConnection],
) -> (String, Vec<StoredConnection>, Option<Workspace>) {
    let workspace_id = workspace.as_ref().and_then(|ws| ws.id);

    match (open_mode, workspace_id) {
        (DatabaseOpenMode::Workspace, Some(id)) => {
            let mut connections: Vec<StoredConnection> = all_connections
                .iter()
                .filter(|connection| connection.connection_type == ConnectionType::Redis)
                .filter(|connection| connection.workspace_id == Some(id))
                .cloned()
                .collect();
            if connections.is_empty() {
                connections.push(conn.clone());
            }
            (format!("workspace-redis-tab-{id}"), connections, workspace)
        }
        _ => {
            let conn_id = conn.id.unwrap_or(0);
            (format!("redis-{conn_id}"), vec![conn.clone()], None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{
        ProxyConfig, ProxyType, RedisMode, RedisParams, RemoteDesktopParams,
        RemoteDesktopProtocol as StoredRemoteDesktopProtocol,
    };

    fn redis_connection(id: i64, name: &str, workspace_id: Option<i64>) -> StoredConnection {
        let params = RedisParams {
            host: "localhost".to_string(),
            port: 6379,
            password: None,
            username: None,
            db_index: 0,
            mode: RedisMode::Standalone,
            use_tls: false,
            connect_timeout: None,
            sentinel: None,
            cluster: None,
            ssh_tunnel: None,
        };
        let mut connection = StoredConnection::new_redis(name.to_string(), params, workspace_id);
        connection.id = Some(id);
        connection
    }

    fn workspace(id: i64, name: &str) -> Workspace {
        let mut workspace = Workspace::new(name.to_string());
        workspace.id = Some(id);
        workspace
    }

    #[test]
    fn redis_single_mode_opens_connection_tab_without_workspace() {
        let connection = redis_connection(42, "redis-prod", Some(7));
        let all_connections = vec![connection.clone()];

        let (tab_id, connections, workspace_for_tab) = redis_tab_open_context(
            DatabaseOpenMode::Single,
            &connection,
            Some(workspace(7, "backend")),
            &all_connections,
        );

        assert_eq!("redis-42", tab_id);
        assert_eq!(
            vec![Some(42)],
            connections.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        assert!(workspace_for_tab.is_none());
    }

    #[test]
    fn notes_tab_uses_stable_identity() {
        let source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        assert!(source.contains("fn add_notes_tab"));
        assert!(source.contains("activate_or_add_tab_lazy(\n                    \"notes\""));
        assert!(source.contains("TabItem::new(\"notes\", \"home\", notes)"));
    }

    #[test]
    fn notes_sidebar_entry_precedes_extensions() {
        let source = include_str!("../persistent_connection_sidebar/rail.rs");
        let notes = source.find("\"persistent-open-notes\"").unwrap();
        let extensions = source.find("\"persistent-open-extensions\"").unwrap();
        assert!(notes < extensions);
    }

    #[test]
    fn persistent_sidebar_uses_large_icons_with_compact_spacing() {
        let source = include_str!("../persistent_connection_sidebar/rail.rs");

        assert!(source.contains("items_center().gap_1().p_1()"));
        assert!(source.matches(".large()").count() >= 2);
        assert!(!source.contains(".ghost()\n                .small()"));
    }

    #[test]
    fn persistent_sidebar_uses_line_style_rail_icons() {
        let source = include_str!("../persistent_connection_sidebar/rail.rs");
        let icons = include_str!("../../../crates/ui/src/icon.rs");

        assert!(source.contains("IconName::User"));
        assert!(source.contains("filter_line_icon"));
        assert!(icons.contains("icons/user.svg"));
        assert!(icons.contains("icons/server_line.svg"));
        assert!(icons.contains("icons/rdp_line.svg"));
    }

    #[test]
    fn ai_workbench_sidebar_entry_opens_a_closeable_regular_tab() {
        let tabs_source = include_str!("home_tabs.rs");
        let rail_source = include_str!("../persistent_connection_sidebar/rail.rs");
        let legacy_sidebar_source = include_str!("../home_tab/sidebar.rs");

        assert!(rail_source.contains("persistent-open-ai-workbench"));
        assert!(rail_source.contains("StartupDefaultPage::Home"));
        assert!(legacy_sidebar_source.contains("legacy-open-ai-workbench"));
        assert!(legacy_sidebar_source.contains("StartupDefaultPage::Home"));
        assert!(legacy_sidebar_source.contains("home.add_ai_workbench_tab(window, cx)"));
        assert!(tabs_source.contains("fn add_ai_workbench_tab"));
        assert!(tabs_source.contains("with_tab_closeable(true)"));
        assert!(
            tabs_source.contains("activate_or_add_tab_lazy(\n                    \"ai-workbench\"")
        );
    }

    #[test]
    fn redis_workspace_mode_groups_workspace_connections() {
        let active = redis_connection(1, "redis-a", Some(7));
        let peer = redis_connection(2, "redis-b", Some(7));
        let other = redis_connection(3, "redis-c", Some(8));
        let all_connections = vec![active.clone(), peer, other];

        let (tab_id, connections, workspace_for_tab) = redis_tab_open_context(
            DatabaseOpenMode::Workspace,
            &active,
            Some(workspace(7, "backend")),
            &all_connections,
        );

        assert_eq!("workspace-redis-tab-7", tab_id);
        assert_eq!(
            vec![Some(1), Some(2)],
            connections.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        assert_eq!("backend", workspace_for_tab.unwrap().name);
    }

    #[test]
    fn remote_desktop_options_maps_connection_proxy() {
        let connection = StoredConnection::new_remote_desktop(
            "rdp".to_string(),
            RemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Rdp,
                host: "10.0.0.8".to_string(),
                port: 3389,
                username: None,
                password: None,
                domain: None,
                read_only: false,
                proxy: Some(ProxyConfig {
                    proxy_type: ProxyType::Http,
                    host: "proxy.example.com".to_string(),
                    port: 8080,
                    username: Some("alice".to_string()),
                    password: Some("secret".to_string()),
                }),
            },
            None,
        );

        let options = remote_desktop_options(&connection, RemoteDesktopProtocol::Rdp).unwrap();
        let proxy = options.proxy.expect("proxy should be mapped");

        assert!(proxy.proxy_type == remote_desktop::ProxyTunnelType::Http);
        assert_eq!("proxy.example.com", proxy.host);
        assert_eq!(Some("alice".to_string()), proxy.username);
    }

    #[test]
    fn terminal_tabs_do_not_request_external_sidebar_mode() {
        let source = include_str!("home_tabs.rs");
        let external_sidebar_call = concat!(".with_", "external_sidebar");
        let lines = source.lines().collect::<Vec<_>>();

        for (index, line) in lines.iter().enumerate() {
            if !line.contains("TerminalWorkspace::new") {
                continue;
            }
            let end = (index + 8).min(lines.len());
            let nearby_source = lines[index..end].join("\n");
            assert!(
                !nearby_source.contains(external_sidebar_call),
                "terminal tab construction should not opt into TabContainer sidebar mode:\n{nearby_source}"
            );
        }
    }

    #[test]
    fn local_terminal_entry_points_use_profile_settings() {
        let source = include_str!("home_tabs.rs");
        let legacy_default = concat!(
            "TerminalWorkspace::new_with_index(",
            "LocalConfig::default()"
        );

        assert!(source.matches("local_config_from_settings").count() >= 2);
        assert!(!source.contains(legacy_default));
    }
}

impl HomePage {
    fn active_tab_container(&self, cx: &App) -> Entity<TabContainer> {
        cx.try_global::<GlobalTabContainer>()
            .map(|global| global.primary_pane())
            .unwrap_or_else(|| self.tab_container.clone())
    }

    fn terminal_sync_path_enabled(cx: &App) -> bool {
        current_terminal_settings(cx).sync_path_with_terminal
    }

    pub(crate) fn open_ssh_terminal(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_ssh_terminal_with_mode(conn, TabOpenMode::Activate, window, cx);
    }

    pub(crate) fn open_ssh_terminal_with_mode(
        &mut self,
        conn: StoredConnection,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conn_id = conn.id.unwrap_or(0);
        // 使用时间戳生成唯一 tab_id，支持同一连接打开多个 SSH 终端
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("ssh-terminal-{}-{}", conn_id, timestamp);

        // 统计同一连接的 SSH 终端数量，计算序号
        let prefix = format!("ssh-terminal-{}-", conn_id);
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };
        let sync_path = Self::terminal_sync_path_enabled(cx);

        let terminal_view = cx.new(|cx| {
            TerminalWorkspace::new_ssh_with_index(conn, tab_index, window, cx, None, sync_path)
        });
        tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, "ssh", terminal_view);
            tc.add_tab_with_mode(tab, mode, window, cx);
        });
    }

    pub(crate) fn open_serial_terminal(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_serial_terminal_with_mode(conn, TabOpenMode::Activate, window, cx);
    }

    pub(crate) fn open_serial_terminal_with_mode(
        &mut self,
        conn: StoredConnection,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conn_id = conn.id.unwrap_or(0);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("serial-terminal-{}-{}", conn_id, timestamp);

        let prefix = format!("serial-terminal-{}-", conn_id);
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        let terminal_view =
            cx.new(|cx| TerminalWorkspace::new_serial_with_index(conn, tab_index, window, cx));
        tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, "serial", terminal_view);
            tc.add_tab_with_mode(tab, mode, window, cx);
        });
    }

    pub(crate) fn open_sftp_view(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conn_id = conn.id.unwrap_or(0);
        // 使用时间戳生成唯一 tab_id，支持同一连接打开多个 SFTP 视图
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("sftp-{}-{}", conn_id, timestamp);

        // 统计同一连接的 SFTP 视图数量，计算序号
        let prefix = format!("sftp-{}-", conn_id);
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        // 创建 SftpView 并订阅终端打开事件
        let sftp_view = cx.new(|cx| SftpView::new_with_index(conn, tab_index, window, cx));
        let event_tab_container = tab_container.clone();

        let subscription = cx.subscribe_in(
            &sftp_view,
            window,
            move |_this, _sftp, event: &SftpViewEvent, window, cx| {
                match event {
                    SftpViewEvent::OpenLocalTerminal { working_dir } => {
                        // 使用时间戳生成唯一 tab_id，支持打开多个本地终端
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let config = match local_config_from_settings(
                            AppSettings::global(cx),
                            Some(working_dir.clone()),
                        ) {
                            Ok(config) => config,
                            Err(error) => {
                                push_local_terminal_config_error(window, &error, cx);
                                return;
                            }
                        };
                        let tab_id = format!("local-terminal-{}", ts);
                        // 统计已有本地终端数量
                        let existing = event_tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .filter(|t| {
                                t.id().starts_with("local-terminal-")
                                    || t.id().starts_with("terminal-")
                            })
                            .count();
                        let idx = if existing > 0 {
                            Some(existing + 1)
                        } else {
                            None
                        };
                        let terminal_view =
                            cx.new(|cx| TerminalWorkspace::new_with_index(config, idx, window, cx));
                        event_tab_container.update(cx, |tc, cx| {
                            let tab = TabItem::new(tab_id, "terminal", terminal_view);
                            tc.add_and_activate_tab_with_focus(tab, window, cx);
                        });
                    }
                    SftpViewEvent::OpenSshTerminal {
                        connection,
                        working_dir,
                    } => {
                        // 使用时间戳生成唯一 tab_id，支持打开多个 SSH 终端
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let conn_id = connection.id.unwrap_or(0);
                        let tab_id = format!("ssh-terminal-{}-{}", conn_id, ts);
                        let conn = connection.clone();
                        // 统计同一连接的 SSH 终端数量
                        let prefix = format!("ssh-terminal-{}-", conn_id);
                        let existing = event_tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .filter(|t| t.id().starts_with(&prefix))
                            .count();
                        let idx = if existing > 0 {
                            Some(existing + 1)
                        } else {
                            None
                        };
                        let sync_path = HomePage::terminal_sync_path_enabled(cx);
                        let terminal_view = cx.new(|cx| {
                            TerminalWorkspace::new_ssh_with_index(
                                conn,
                                idx,
                                window,
                                cx,
                                Some(working_dir),
                                sync_path,
                            )
                        });
                        event_tab_container.update(cx, |tc, cx| {
                            let tab = TabItem::new(tab_id, "ssh", terminal_view);
                            tc.add_and_activate_tab_with_focus(tab, window, cx);
                        });
                    }
                }
            },
        );
        self._subscriptions.push(subscription);

        // 添加标签页
        let tab = TabItem::new(tab_id, "sftp", sftp_view);
        tab_container.update(cx, |tc, cx| {
            tc.add_and_activate_tab_with_focus(tab, window, cx);
        });
    }

    pub(crate) fn open_remote_desktop_with_mode(
        &mut self,
        conn: StoredConnection,
        protocol: RemoteDesktopProtocol,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(options) = remote_desktop_options(&conn, protocol) else {
            tracing::warn!(
                connection_id = ?conn.id,
                connection_name = %conn.name,
                "failed to parse remote desktop connection params"
            );
            return;
        };
        let conn_id = conn.id.unwrap_or(0);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_kind = remote_desktop_tab_kind(protocol);
        let tab_id = format!("{tab_kind}-{conn_id}-{timestamp}");
        let prefix = format!("{tab_kind}-{conn_id}-");
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|tab| tab.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };
        let title = conn.name.clone();
        let window_handle = window.window_handle();
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
        tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, tab_kind, view);
            tc.add_tab_with_mode(tab, mode, window, cx);
        });
    }

    pub(crate) fn open_redis_tab_with_mode(
        &mut self,
        conn: StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open_mode = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).database_open_mode
        } else {
            DatabaseOpenMode::default()
        };
        let active_conn_id = conn.id;

        let (tab_id, connections, workspace_for_tab) =
            redis_tab_open_context(open_mode, &conn, workspace, &self.connections);

        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            let tab_id_for_tab = tab_id.clone();
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy_with_mode(
                    tab_id,
                    mode,
                    move |window, cx| {
                        let redis_view = cx.new(|cx| {
                            RedisTabView::new_with_active_conn(
                                workspace_for_tab,
                                connections,
                                active_conn_id,
                                window,
                                cx,
                            )
                            .with_external_sidebar()
                        });
                        TabItem::new(tab_id_for_tab, "redis", redis_view)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn open_mongodb_tab_with_mode(
        &mut self,
        conn: StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open_mode = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).database_open_mode
        } else {
            DatabaseOpenMode::default()
        };

        let workspace_id = workspace.as_ref().and_then(|ws| ws.id);
        let active_conn_id = conn.id;

        let (tab_id, connections, workspace_for_tab) = match open_mode {
            DatabaseOpenMode::Workspace if workspace_id.is_some() => {
                let connections = self
                    .connections
                    .iter()
                    .filter(|connection| connection.workspace_id == workspace_id)
                    .filter(|connection| connection.connection_type == ConnectionType::MongoDB)
                    .cloned()
                    .collect();
                let tab_id = format!("workspace-mongodb-tab-{}", workspace_id.unwrap_or(0));
                (tab_id, connections, workspace)
            }
            _ => {
                let conn_id = conn.id.unwrap_or(0);
                let tab_id = format!("mongodb-{}", conn_id);
                (tab_id, vec![conn.clone()], None)
            }
        };

        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            let tab_id_for_tab = tab_id.clone();
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy_with_mode(
                    tab_id,
                    mode,
                    move |window, cx| {
                        let mongo_view = cx.new(|cx| {
                            MongoTabView::new_with_active_conn(
                                workspace_for_tab,
                                connections,
                                active_conn_id,
                                window,
                                cx,
                            )
                            .with_external_sidebar()
                        });
                        TabItem::new(tab_id_for_tab, "mongodb", mongo_view)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new(win, cx));
                        TabItem::new("settings", "home", settings)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_team_key_settings_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !is_feature_enabled(Feature::TeamManagement, cx) {
            return;
        }

        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings-team-keys",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new_team_keys(win, cx));
                        TabItem::new("settings-team-keys", "home", settings)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_extensions_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "extensions",
                    |win, cx| {
                        let host = std::sync::Arc::new(extension_runtime::MainExtensionViewHost);
                        let extensions =
                            cx.new(|cx| extension_view::ExtensionManagerView::new(host, win, cx));
                        TabItem::new("extensions", "home", extensions)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_notes_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "notes",
                    |window, cx| {
                        let notes = cx.new(|cx| NotesView::new(window, cx));
                        TabItem::new("notes", "home", notes)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_ai_workbench_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        let (scope, catalog, mentions) =
            ai_chat_view::build_workbench_resource_state(&self.connections);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "ai-workbench",
                    |window, cx| {
                        let workbench = cx.new(|cx| {
                            ai_chat_view::DefaultAgentChatPanel::new_workbench_with_scope_and_catalog(
                                scope,
                                catalog,
                                mentions,
                                window,
                                cx,
                            )
                            .with_tab_closeable(true)
                        });
                        TabItem::new("ai-workbench", "home", workbench)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_terminal_tab_from_profile(None, window, cx);
    }

    pub(crate) fn add_terminal_tab_with_profile(
        &mut self,
        profile_kind: LocalTerminalProfileKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_terminal_tab_from_profile(Some(profile_kind), window, cx);
    }

    pub(crate) fn add_terminal_tab_with_custom_profile(
        &mut self,
        profile: LocalTerminalCustomProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = match local_config_from_custom_profile(&profile, None) {
            Ok(config) => config,
            Err(error) => {
                push_local_terminal_config_error(window, &error, cx);
                return;
            }
        };
        self.add_local_terminal_tab(config, window, cx);
    }

    fn add_terminal_tab_from_profile(
        &mut self,
        profile_kind: Option<LocalTerminalProfileKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = AppSettings::global(cx);
        let config = match profile_kind {
            Some(kind) => local_config_from_settings_with_profile(settings, kind, None),
            None => local_config_from_settings(settings, None),
        };
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                push_local_terminal_config_error(window, &error, cx);
                return;
            }
        };
        self.add_local_terminal_tab(config, window, cx);
    }

    fn add_local_terminal_tab(
        &mut self,
        config: terminal::LocalConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 使用时间戳生成唯一 tab_id，支持打开多个本地终端
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("terminal-{}", timestamp);

        // 统计已有本地终端数量，计算序号
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with("terminal-") || t.id().starts_with("local-terminal-"))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        let home = cx.entity();
        window.defer(cx, move |window, cx| {
            home.update(cx, |_this, cx| {
                let terminal_view =
                    cx.new(|cx| TerminalWorkspace::new_with_index(config, tab_index, window, cx));
                tab_container.update(cx, |tc, cx| {
                    let tab = TabItem::new(tab_id, "home", terminal_view);
                    tc.add_and_activate_tab_with_focus(tab, window, cx);
                });
            });
        });
    }

    pub(crate) fn add_item_to_tab_with_mode(
        &mut self,
        conn: &StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 根据设置中的数据库打开方式决定如何打开
        let open_mode = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).database_open_mode
        } else {
            DatabaseOpenMode::default()
        };

        // 在 defer 之前准备所有需要的数据，避免在 HomePage 更新期间
        // 触发 on_deactivate 导致双重借用 panic
        let workspace_id = workspace.as_ref().and_then(|w| w.id);
        let conn_clone = conn.clone();
        let connections: Vec<StoredConnection> = match open_mode {
            DatabaseOpenMode::Workspace if workspace_id.is_some() => self
                .connections
                .iter()
                .filter(|c| c.workspace_id == workspace_id)
                .filter(|c| c.connection_type == ConnectionType::Database)
                .cloned()
                .collect(),
            _ => vec![conn.clone()],
        };

        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| match open_mode {
                DatabaseOpenMode::Single => {
                    let tab_id = format!("database-tab-{}", conn_clone.id.unwrap_or(0));
                    tc.activate_or_add_tab_lazy_with_mode(
                        tab_id.clone(),
                        mode,
                        move |window, cx| {
                            let db_view = cx.new(|cx| {
                                DatabaseTabView::new_with_active_conn(
                                    None,
                                    vec![conn_clone.clone()],
                                    conn_clone.id,
                                    window,
                                    cx,
                                )
                                .with_external_sidebar()
                            });
                            TabItem::new(tab_id.clone(), "home", db_view)
                        },
                        window,
                        cx,
                    );
                }
                DatabaseOpenMode::Workspace => {
                    let tab_id = if workspace_id.is_some() {
                        format!("workspace-database-tab-{}", workspace_id.unwrap_or(0))
                    } else {
                        format!("database-tab-{}", conn_clone.id.unwrap_or(0))
                    };

                    let active_conn_id = conn_clone.id;
                    tc.activate_or_add_tab_lazy_with_mode(
                        tab_id.clone(),
                        mode,
                        move |window, cx| {
                            let db_view = cx.new(|cx| {
                                DatabaseTabView::new_with_active_conn(
                                    workspace,
                                    connections,
                                    active_conn_id,
                                    window,
                                    cx,
                                )
                                .with_external_sidebar()
                            });
                            TabItem::new(tab_id.clone(), "home", db_view)
                        },
                        window,
                        cx,
                    );
                }
            });
        });
    }

    /// 复制当前活动标签并打开
    pub(crate) fn duplicate_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        let tc = tab_container.read(cx);

        // pinned tab 不支持复制
        if tc.is_pinned_tab_active() {
            return;
        }

        let Some(active_tab) = tc.active_tab() else {
            return;
        };

        let content_key = active_tab.content().content_key(cx);

        match content_key {
            "Terminal" => {
                // 获取终端视图的连接信息
                let view = active_tab.content().view();
                let Ok(terminal_view) = view.downcast::<TerminalWorkspace>() else {
                    return;
                };

                let kind = terminal_view.read(cx).connection_kind(cx);
                match kind {
                    TerminalConnectionKind::Ssh => {
                        // SSH 终端：通过 connection_id 找到 StoredConnection 并打开新连接
                        let conn_id = terminal_view.read(cx).connection_id(cx);
                        if let Some(conn_id) = conn_id {
                            if let Some(conn) = self
                                .connections
                                .iter()
                                .find(|c| c.id == Some(conn_id))
                                .cloned()
                            {
                                self.open_ssh_terminal(conn, window, cx);
                            }
                        }
                    }
                    TerminalConnectionKind::Serial => {
                        let conn_id = terminal_view.read(cx).connection_id(cx);
                        if let Some(conn_id) = conn_id {
                            if let Some(conn) = self
                                .connections
                                .iter()
                                .find(|c| c.id == Some(conn_id))
                                .cloned()
                            {
                                self.open_serial_terminal(conn, window, cx);
                            }
                        }
                    }
                    TerminalConnectionKind::Local => {
                        // 本地终端：直接新建
                        self.add_terminal_tab(window, cx);
                    }
                }
            }
            _ => {
                // 其他类型暂不支持复制
            }
        }
    }
}

pub(crate) fn remote_desktop_options(
    conn: &StoredConnection,
    protocol: RemoteDesktopProtocol,
) -> Option<RemoteDesktopConnectionOptions> {
    let params = conn.to_remote_desktop_params().ok()?;
    Some(RemoteDesktopConnectionOptions {
        protocol,
        destination: format!("{}:{}", params.host, params.port),
        username: params.username,
        password: params.password,
        domain: params.domain,
        read_only: params.read_only,
        proxy: params.proxy.map(remote_desktop_proxy_config),
    })
}

fn remote_desktop_proxy_config(proxy: ProxyConfig) -> remote_desktop::ProxyTunnelConfig {
    remote_desktop::ProxyTunnelConfig {
        proxy_type: match proxy.proxy_type {
            ProxyType::Socks5 => remote_desktop::ProxyTunnelType::Socks5,
            ProxyType::Http => remote_desktop::ProxyTunnelType::Http,
        },
        host: proxy.host.trim().to_string(),
        port: proxy.port,
        username: normalized_optional(proxy.username),
        password: preserved_secret(proxy.password),
    }
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn preserved_secret(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn push_local_terminal_config_error<T>(
    window: &mut Window,
    error: &dyn std::fmt::Display,
    cx: &mut Context<T>,
) {
    window.push_notification(
        Notification::error(t!("Home.local_terminal_invalid_config", error = error).to_string()),
        cx,
    );
}

fn remote_desktop_tab_kind(protocol: RemoteDesktopProtocol) -> &'static str {
    match protocol {
        RemoteDesktopProtocol::Rdp => "rdp",
        RemoteDesktopProtocol::Vnc => "vnc",
    }
}
