use crate::credential_vault::CredentialVaultView;
use crate::home_tab::{HomePage, resolve_connection_credentials};
use crate::license::is_feature_enabled;
use crate::onetcli_app::{GlobalOnetCliApp, GlobalTabContainer};
use crate::session_logs::SessionLogsPage;
use crate::setting_tab::{AppSettings, DatabaseOpenMode, SettingsPanel};
use db_view::database_tab::DatabaseTabView;
use gpui::{App, AppContext, Context, Entity, Focusable, Window};
use gpui_component::{WindowExt, notification::Notification};
use json_view::JsonFormatterView;
use mongodb_view::MongoTabView;
use notes::NotesView;
use one_core::license::Feature;
use one_core::settings::{
    ConnectionSortOrder, LocalTerminalCustomProfile, LocalTerminalProfileKind,
};
use one_core::storage::{ConnectionType, StoredConnection, Workspace};
use one_core::tab_actions::next_duplicate_tab_index;
use one_core::tab_container::{TabContainer, TabItem, TabOpenMode};
use redis_view::RedisTabView;
use remote_desktop::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol};
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use rust_i18n::t;
use sftp_view::{SftpView, SftpViewEvent};
use std::collections::HashSet;
use terminal::{
    local_config_from_custom_profile, local_config_from_settings,
    local_config_from_settings_with_profile,
};
use terminal_view::{
    TerminalConnectionKind, TerminalWorkspace, TerminalWorkspaceEvent,
    current_settings as current_terminal_settings,
};

fn redis_tab_open_context(
    open_mode: DatabaseOpenMode,
    conn: &StoredConnection,
    workspace: Option<Workspace>,
    all_connections: &[StoredConnection],
    sort_order: ConnectionSortOrder,
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
            crate::connection_sort::sort_connections(&mut connections, sort_order);
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

fn database_tab_connection_context(
    open_mode: DatabaseOpenMode,
    active_connection: &StoredConnection,
    workspace_id: Option<i64>,
    all_connections: &[StoredConnection],
    mut resolve: impl FnMut(&StoredConnection) -> Option<StoredConnection>,
) -> Option<(StoredConnection, Vec<StoredConnection>)> {
    let active_connection_id = active_connection.id;
    let active_connection = resolve(active_connection)?;
    let connections = match (open_mode, workspace_id) {
        (DatabaseOpenMode::Workspace, Some(workspace_id)) => {
            let mut connections = Vec::new();
            let mut active_connection_included = false;
            let mut seen_connection_ids = HashSet::new();

            for candidate in all_connections
                .iter()
                .filter(|candidate| candidate.workspace_id == Some(workspace_id))
                .filter(|candidate| candidate.connection_type == ConnectionType::Database)
            {
                // Workspace connections come from persistent storage and should have stable IDs.
                // Treating multiple `None` IDs as equal would incorrectly duplicate the active
                // runtime connection and could leak an unresolved candidate into the tab.
                let Some(candidate_id) = candidate.id else {
                    continue;
                };

                if active_connection_id == Some(candidate_id) {
                    if !active_connection_included {
                        seen_connection_ids.insert(candidate_id);
                        connections.push(active_connection.clone());
                        active_connection_included = true;
                    }
                    continue;
                }

                if seen_connection_ids.contains(&candidate_id) {
                    continue;
                }

                if let Some(connection) = resolve(candidate) {
                    seen_connection_ids.insert(candidate_id);
                    connections.push(connection);
                }
            }

            if !active_connection_included {
                connections.push(active_connection.clone());
            }
            connections
        }
        _ => vec![active_connection.clone()],
    };

    Some((active_connection, connections))
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{
        DatabaseType, DbConnectionConfig, ProxyConfig, ProxyType, RedisMode, RedisParams,
        RemoteDesktopBackendPreference, RemoteDesktopParams,
        RemoteDesktopProtocol as StoredRemoteDesktopProtocol,
    };

    fn redis_connection(id: i64, name: &str, workspace_id: Option<i64>) -> StoredConnection {
        let params = RedisParams {
            host: "localhost".to_string(),
            port: 6379,
            password: None,
            username: None,
            credential_reference: None,
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

    fn database_connection(
        id: Option<i64>,
        name: &str,
        workspace_id: Option<i64>,
    ) -> StoredConnection {
        let mut connection = StoredConnection::new_database(
            name.to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: name.to_string(),
                host: "localhost".to_string(),
                port: 3306,
                username: "stored-user".to_string(),
                password: "stored-password".to_string(),
                credential_reference: None,
                database: None,
                service_name: None,
                sid: None,
                workspace_id,
                proxy: None,
                extra_params: Default::default(),
            },
            workspace_id,
        );
        connection.id = id;
        connection
    }

    #[test]
    fn database_single_mode_resolves_the_active_connection() {
        let active = database_connection(Some(1), "active", Some(7));
        let mut resolved_ids = Vec::new();

        let (resolved_active, connections) = database_tab_connection_context(
            DatabaseOpenMode::Single,
            &active,
            Some(7),
            &[active.clone()],
            |connection| {
                resolved_ids.push(connection.id);
                let mut connection = connection.clone();
                connection.name = format!("resolved-{}", connection.name);
                Some(connection)
            },
        )
        .expect("active connection should resolve");

        assert_eq!(vec![Some(1)], resolved_ids);
        assert_eq!("resolved-active", resolved_active.name);
        assert_eq!(vec!["resolved-active"], connection_names(&connections));
    }

    #[test]
    fn database_workspace_mode_resolves_active_once_and_skips_failed_or_duplicate_peers() {
        let active = database_connection(Some(1), "active", Some(7));
        let failed_peer = database_connection(Some(2), "failed-peer", Some(7));
        let peer = database_connection(Some(3), "peer", Some(7));
        let duplicate_peer = database_connection(Some(3), "duplicate-peer", Some(7));
        let all_connections = vec![
            active.clone(),
            active.clone(),
            failed_peer,
            peer,
            duplicate_peer,
        ];
        let mut resolved_ids = Vec::new();

        let (resolved_active, connections) = database_tab_connection_context(
            DatabaseOpenMode::Workspace,
            &active,
            Some(7),
            &all_connections,
            |connection| {
                resolved_ids.push(connection.id);
                if connection.id == Some(2) {
                    return None;
                }
                let mut connection = connection.clone();
                connection.name = format!("resolved-{}", connection.name);
                Some(connection)
            },
        )
        .expect("active connection should resolve");

        assert_eq!(vec![Some(1), Some(2), Some(3)], resolved_ids);
        assert_eq!("resolved-active", resolved_active.name);
        assert_eq!(
            vec![Some(1), Some(3)],
            connections
                .iter()
                .map(|connection| connection.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec!["resolved-active", "resolved-peer"],
            connection_names(&connections)
        );
    }

    #[test]
    fn database_workspace_mode_does_not_treat_none_ids_as_the_active_connection() {
        let active = database_connection(None, "active", Some(7));
        let anonymous_peer = database_connection(None, "anonymous-peer", Some(7));
        let persisted_peer = database_connection(Some(2), "persisted-peer", Some(7));
        let all_connections = vec![anonymous_peer, persisted_peer];
        let mut resolved_names = Vec::new();

        let (resolved_active, connections) = database_tab_connection_context(
            DatabaseOpenMode::Workspace,
            &active,
            Some(7),
            &all_connections,
            |connection| {
                resolved_names.push(connection.name.clone());
                let mut connection = connection.clone();
                connection.name = format!("resolved-{}", connection.name);
                Some(connection)
            },
        )
        .expect("active connection should resolve");

        assert_eq!("resolved-active", resolved_active.name);
        assert_eq!(
            vec![Some(2), None],
            connections
                .iter()
                .map(|connection| connection.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec!["resolved-persisted-peer", "resolved-active"],
            connection_names(&connections)
        );
        assert_eq!(vec!["active", "persisted-peer"], resolved_names);
    }

    #[test]
    fn database_workspace_mode_without_workspace_id_uses_only_the_resolved_active_connection() {
        let active = database_connection(Some(1), "active", None);
        let peer = database_connection(Some(2), "peer", Some(7));
        let mut resolved_ids = Vec::new();

        let (resolved_active, connections) = database_tab_connection_context(
            DatabaseOpenMode::Workspace,
            &active,
            None,
            &[peer],
            |connection| {
                resolved_ids.push(connection.id);
                let mut connection = connection.clone();
                connection.name = format!("resolved-{}", connection.name);
                Some(connection)
            },
        )
        .expect("active connection should resolve");

        assert_eq!(vec![Some(1)], resolved_ids);
        assert_eq!("resolved-active", resolved_active.name);
        assert_eq!(vec!["resolved-active"], connection_names(&connections));
    }

    #[test]
    fn database_workspace_mode_filters_other_workspaces_and_connection_types() {
        let active = database_connection(Some(1), "active", Some(7));
        let peer = database_connection(Some(2), "peer", Some(7));
        let other_workspace = database_connection(Some(3), "other-workspace", Some(8));
        let redis = redis_connection(4, "redis", Some(7));
        let mut resolved_ids = Vec::new();

        let (_, connections) = database_tab_connection_context(
            DatabaseOpenMode::Workspace,
            &active,
            Some(7),
            &[active.clone(), peer, other_workspace, redis],
            |connection| {
                resolved_ids.push(connection.id);
                Some(connection.clone())
            },
        )
        .expect("active connection should resolve");

        assert_eq!(vec![Some(1), Some(2)], resolved_ids);
        assert_eq!(
            vec![Some(1), Some(2)],
            connections
                .iter()
                .map(|connection| connection.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn database_tab_open_fails_closed_when_active_resolution_fails() {
        let active = database_connection(Some(1), "active", Some(7));

        let context = database_tab_connection_context(
            DatabaseOpenMode::Workspace,
            &active,
            Some(7),
            &[active.clone()],
            |_| None,
        );

        assert!(context.is_none());
    }

    fn connection_names(connections: &[StoredConnection]) -> Vec<&str> {
        connections
            .iter()
            .map(|connection| connection.name.as_str())
            .collect()
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
            ConnectionSortOrder::Natural,
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
    fn session_logs_open_from_both_home_sidebars_as_a_stable_tab() {
        let tabs_source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let legacy_source = include_str!("../home_tab/sidebar_navigation.rs");
        let modern_home_source = include_str!("../home_tab/modern_home.rs");
        let navigation_source = include_str!("../home_tab/navigation.rs");

        assert!(tabs_source.contains("fn add_session_logs_tab"));
        assert!(
            tabs_source.contains("activate_or_add_tab_lazy(\n                    \"session-logs\"")
        );
        assert!(tabs_source.contains("TabItem::new(\"session-logs\", \"home\", page)"));
        assert!(tabs_source.contains("window.defer(cx, move |window, cx|"));
        assert!(legacy_source.contains("\"legacy-open-session-logs\""));
        assert!(
            legacy_source.contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(modern_home_source.contains("\"home-app-session-logs\""));
        assert!(
            modern_home_source
                .contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(navigation_source.contains(
            "NavigationApplication::SessionLogs => self.add_session_logs_tab(window, cx)"
        ));
    }

    #[test]
    fn credential_vault_opens_from_both_home_sidebars_as_a_stable_tab() {
        let tabs_source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let toolbar_source = include_str!("../home_tab/toolbar.rs");
        let legacy_sidebar_source = include_str!("../home_tab/sidebar_navigation.rs");
        let modern_home_source = include_str!("../home_tab/modern_home.rs");
        let navigation_source = include_str!("../home_tab/navigation.rs");
        let settings_source = include_str!("../setting_tab.rs");
        let actions_source = include_str!("../credential_vault/actions.rs");

        assert!(tabs_source.contains("fn add_credential_vault_tab"));
        assert!(
            tabs_source
                .contains("activate_or_add_tab_lazy(\n                    \"credential-vault\"")
        );
        assert!(tabs_source.contains("TabItem::new(\"credential-vault\", \"home\", vault)"));
        assert!(legacy_sidebar_source.contains("\"legacy-open-credential-vault\""));
        assert!(
            legacy_sidebar_source
                .contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(modern_home_source.contains("\"home-app-credential-vault\""));
        assert!(
            modern_home_source
                .contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(navigation_source.contains("NavigationApplication::CredentialVault =>"));
        assert!(navigation_source.contains("self.add_credential_vault_tab(window, cx)"));
        assert!(!toolbar_source.contains("\"credential-vault-button\""));
        assert!(!toolbar_source.contains("add_credential_vault_tab"));
        assert!(!settings_source.contains("SettingPage::new(\"钥匙串\")"));
        assert!(actions_source.contains("open_popup_window("));
        assert!(!actions_source.contains("open_dialog("));
    }

    #[test]
    fn more_applications_places_json_formatter_after_credential_vault() {
        use crate::navigation_quick_open::{
            NavigationApplication, overflow_navigation_applications,
        };

        assert_eq!(
            overflow_navigation_applications(),
            vec![
                NavigationApplication::SessionLogs,
                NavigationApplication::CredentialVault,
                NavigationApplication::JsonFormatter,
            ]
        );
    }

    #[test]
    fn both_home_layouts_place_credential_vault_with_their_application_entries() {
        let legacy_source = include_str!("../home_tab/sidebar_navigation.rs");
        let modern_home_source = include_str!("../home_tab/modern_home.rs");

        for id in [
            "\"legacy-open-notes\"",
            "\"legacy-open-session-logs\"",
            "\"legacy-open-credential-vault\"",
            "\"legacy-open-extensions\"",
        ] {
            assert!(legacy_source.contains(id));
        }
        for id in [
            "\"home-app-notes\"",
            "\"home-app-session-logs\"",
            "\"home-app-credential-vault\"",
            "\"home-app-extensions\"",
        ] {
            assert!(modern_home_source.contains(id));
        }
        assert!(
            !modern_home_source.contains("home-app-settings"),
            "设置入口已迁移到全局标签栏，不再作为现代主页应用磁贴"
        );
        assert!(legacy_source.contains("show_application_navigation_quick_open"));
        assert!(modern_home_source.contains("all_navigation_applications("));
    }

    #[test]
    fn persistent_sidebar_tree_toggle_leads_and_home_follows_in_the_tab_bar() {
        // 常驻 rail 已移除：Home 与连接树折叠按钮都收进顶部标签栏。
        // 折叠按钮贴着窗口边缘（与 macOS 红绿灯对齐的导航位），Home 紧随其后。
        let source = include_str!("../../../crates/core/src/tab_container.rs");
        let home = source.find("\"tab-bar-home\"").unwrap();
        let tree_toggle = source.find("\"navigation-sidebar-toggle\"").unwrap();

        assert!(tree_toggle < home);
        assert!(source.contains("IconName::Home"));
        assert!(source.contains("set_home_button_active("));
    }

    #[test]
    fn persistent_sidebar_home_entry_avoids_reentrant_home_page_updates() {
        let tabs_source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let tabs_impl = tabs_source.split_once("\nimpl HomePage {\n").unwrap().1;
        let app_source = include_str!("../onetcli_app.rs").replace("\r\n", "\n");
        let show_home_start = tabs_impl
            .find(
                "pub(crate) fn show_home(home_page: &Entity<Self>, window: &mut Window, cx: &mut App)",
            )
            .unwrap();
        let show_home_end = tabs_impl[show_home_start..]
            .find("\n    fn terminal_sync_path_enabled")
            .map(|offset| show_home_start + offset)
            .unwrap();
        let show_home_source = &tabs_impl[show_home_start..show_home_end];

        assert!(show_home_source.contains("window.defer(cx, move |window, cx|"));
        assert!(show_home_source.contains("try_global::<GlobalOnetCliApp>()"));
        assert!(show_home_source.contains("app.show_home(window, cx)"));
        assert!(!show_home_source.contains("activate_base_content"));
        assert!(
            app_source.contains("HomePage::show_home(&home_page, window, cx);"),
            "顶部 Home 按钮应走与旧 rail 相同的 show_home 入口"
        );
    }

    #[test]
    fn home_page_connection_openers_defer_active_tab_changes() {
        let tabs_source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let tabs_impl = tabs_source.split_once("\nimpl HomePage {\n").unwrap().1;
        let forwarding_source = include_str!("../home_tab/forwarding.rs").replace("\r\n", "\n");
        let forwarding_impl = forwarding_source
            .split_once("\nimpl HomePage {\n")
            .unwrap()
            .1;

        for (method, next_method) in [
            (
                "pub(crate) fn open_ssh_terminal_with_mode",
                "pub(crate) fn open_serial_terminal",
            ),
            (
                "pub(crate) fn open_serial_terminal_with_mode",
                "pub(crate) fn open_sftp_view",
            ),
            (
                "pub(crate) fn open_sftp_view",
                "pub(crate) fn open_remote_desktop_with_mode",
            ),
            (
                "pub(crate) fn open_remote_desktop_with_mode",
                "pub(crate) fn open_redis_tab_with_mode",
            ),
        ] {
            let start = tabs_impl.find(method).unwrap();
            let end = tabs_impl[start..]
                .find(next_method)
                .map(|offset| start + offset)
                .unwrap();
            let method_source = &tabs_impl[start..end];

            assert!(
                method_source.contains("window.defer(cx, move |window, cx|"),
                "{method} must defer TabContainer activation to avoid re-entering HomePage"
            );
        }

        let start = forwarding_impl
            .find("pub(crate) fn open_port_forwarding_tab")
            .unwrap();
        let end = forwarding_impl[start..]
            .find("pub(super) fn port_forwarding_tab_config")
            .map(|offset| start + offset)
            .unwrap();
        let method_source = &forwarding_impl[start..end];
        assert!(method_source.contains("window.defer(cx, move |window, cx|"));
    }

    #[test]
    fn persistent_filter_button_uses_the_shared_rail_icon_geometry() {
        let filter_bar =
            include_str!("../persistent_connection_sidebar/filter_bar.rs").replace("\r\n", "\n");
        let visuals = include_str!("../connection_visuals.rs");

        assert!(filter_bar.contains("persistent-filter-button"));
        assert!(filter_bar.contains("IconName::Filter"));
        assert!(filter_bar.contains("IconButtonRole::Compact"));
        assert!(filter_bar.contains("connection_type_rail_icon(filter)"));
        assert!(visuals.contains("Self::Inline | Self::Rail => IconSize::Medium"));
        assert!(filter_bar.contains("ConnectionType::all()"));
        assert!(filter_bar.contains(".checked(selected_filter == filter)"));
    }

    #[test]
    fn persistent_sidebar_uses_line_style_rail_icons() {
        let filter_bar = include_str!("../persistent_connection_sidebar/filter_bar.rs");
        let modern_home = include_str!("../home_tab/modern_home.rs");
        let user_avatar = include_str!("../user_avatar.rs");
        let visuals = include_str!("../connection_visuals.rs");
        let remote_render = include_str!("../../../crates/remote_desktop_view/src/view/render.rs");

        assert!(filter_bar.contains("connection_type_rail_icon"));
        assert!(modern_home.contains("Icon::new(application.icon())"));
        assert!(modern_home.contains("render_user_avatar("));
        assert!(user_avatar.contains("IconName::User"));
        assert!(visuals.contains("ConnectionType::All => IconName::ServerLine"));
        assert!(visuals.contains("ConnectionType::SshSftp => IconName::TerminalLine"));
        assert!(visuals.contains("ConnectionType::Rdp => IconName::RdpLine"));
        assert!(visuals.contains("ConnectionType::Vnc => IconName::VncLine"));
        assert!(visuals.contains(".mono()"));
        assert!(visuals.contains("ConnectionType::Rdp => IconName::Rdp"));
        assert!(visuals.contains("ConnectionType::Vnc => IconName::Vnc"));
        assert!(visuals.contains(".color()"));
        assert_eq!(
            gpui_component::IconNamed::path(gpui_component::IconName::User),
            "icons/user.svg"
        );
        assert_eq!(
            gpui_component::IconNamed::path(gpui_component::IconName::ServerLine),
            "icons/server_line.svg"
        );
        assert_eq!(
            gpui_component::IconNamed::path(gpui_component::IconName::RdpLine),
            "icons/rdp_line.svg"
        );
        assert!(remote_render.contains("RemoteDesktopProtocol::Rdp => IconName::Rdp.color()"));
        assert!(remote_render.contains("RemoteDesktopProtocol::Vnc => IconName::Vnc.color()"));
    }

    #[test]
    fn ai_workbench_sidebar_entry_opens_a_closeable_regular_tab() {
        let tabs_source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let modern_home_source = include_str!("../home_tab/modern_home.rs");
        let legacy_sidebar_source = include_str!("../home_tab/sidebar_navigation.rs");
        let legacy_sidebar_layout_source = include_str!("../home_tab/sidebar.rs");
        let navigation_source = include_str!("../home_tab/navigation.rs");

        assert!(modern_home_source.contains("home-app-ai-workbench"));
        assert!(modern_home_source.contains("StartupDefaultPage::Home"));
        assert!(legacy_sidebar_source.contains("legacy-open-ai-workbench"));
        assert!(legacy_sidebar_layout_source.contains("StartupDefaultPage::Home"));
        assert!(
            legacy_sidebar_source
                .contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(
            modern_home_source
                .contains("home.activate_navigation_application(application, window, cx)")
        );
        assert!(navigation_source.contains(
            "NavigationApplication::AiWorkbench => self.add_ai_workbench_tab(window, cx)"
        ));
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
            ConnectionSortOrder::Natural,
        );

        assert_eq!("workspace-redis-tab-7", tab_id);
        assert_eq!(
            vec![Some(1), Some(2)],
            connections.iter().map(|c| c.id).collect::<Vec<_>>()
        );
        assert_eq!("backend", workspace_for_tab.unwrap().name);
    }

    #[test]
    fn remote_desktop_options_maps_connection_proxy_and_rdp_audio() {
        let connection = StoredConnection::new_remote_desktop(
            "rdp".to_string(),
            RemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Rdp,
                host: "10.0.0.8".to_string(),
                port: 3389,
                username: None,
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: true,
                proxy: Some(ProxyConfig {
                    proxy_type: ProxyType::Http,
                    host: "proxy.example.com".to_string(),
                    port: 8080,
                    username: Some("alice".to_string()),
                    password: Some("secret".to_string()),
                    credential_reference: None,
                }),
                backend_preference: RemoteDesktopBackendPreference::WindowsNative,
                rdp: None,
            },
            None,
        );

        let options = remote_desktop_options(&connection, RemoteDesktopProtocol::Rdp).unwrap();
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            options.backend_preference
        );
        let proxy = options.proxy.expect("proxy should be mapped");

        assert!(options.audio_playback);
        assert!(proxy.proxy_type == remote_desktop::ProxyTunnelType::Http);
        assert_eq!("proxy.example.com", proxy.host);
        assert_eq!(Some("alice".to_string()), proxy.username);
    }

    #[test]
    fn remote_desktop_options_never_enable_vnc_audio() {
        let connection = StoredConnection::new_remote_desktop(
            "vnc".to_string(),
            RemoteDesktopParams {
                protocol: StoredRemoteDesktopProtocol::Vnc,
                host: "10.0.0.9".to_string(),
                port: 5900,
                username: None,
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: true,
                proxy: None,
                backend_preference: RemoteDesktopBackendPreference::WindowsNative,
                rdp: None,
            },
            None,
        );

        let options = remote_desktop_options(&connection, RemoteDesktopProtocol::Vnc).unwrap();

        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            options.backend_preference
        );
        assert!(!options.audio_playback);
    }

    #[test]
    fn remote_desktop_tab_resolves_credentials_before_building_options() {
        let source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let implementation = source
            .split_once("\nimpl HomePage {\n")
            .expect("HomePage implementation")
            .1;
        let method_start = implementation
            .find("pub(crate) fn open_remote_desktop_with_mode")
            .expect("remote desktop open method");
        let method_end = implementation[method_start..]
            .find("pub(crate) fn open_redis_tab_with_mode")
            .map(|offset| method_start + offset)
            .expect("next method");
        let method = &implementation[method_start..method_end];
        let resolve = method
            .find("resolve_connection_credentials(")
            .and_then(|offset| {
                method[offset..]
                    .find("window, cx)")
                    .map(|next| offset + next + "window, cx)".len())
            })
            .expect("credential resolution");
        let options = method
            .find("remote_desktop_options(&conn, protocol)")
            .expect("remote desktop options");

        assert!(
            resolve < options,
            "RDP/VNC credentials must be resolved before runtime options are built"
        );
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
    fn ssh_terminal_workspace_can_request_an_sftp_tab() {
        let source = include_str!("home_tabs.rs").replace("\r\n", "\n");
        let method = source
            .split("\n    pub(crate) fn open_ssh_terminal_with_mode")
            .nth(1)
            .and_then(|source| {
                source
                    .split("\n    pub(crate) fn open_serial_terminal")
                    .next()
            })
            .expect("SSH terminal open method");

        assert!(method.contains("TerminalWorkspaceEvent::OpenSftp(connection)"));
        assert!(method.contains("this.open_sftp_view(connection.clone(), window, cx)"));
        assert!(method.contains("self._subscriptions.push(subscription)"));
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

    pub(crate) fn set_home_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.home_active == active {
            return;
        }
        self.home_active = active;
        cx.notify();
    }

    pub(crate) fn show_home(home_page: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let app = cx
            .try_global::<GlobalOnetCliApp>()
            .map(|global| global.app.clone());
        let home_page = home_page.clone();
        window.defer(cx, move |window, cx| {
            if let Some(app) = app {
                app.update(cx, |app, cx| app.show_home(window, cx));
            } else {
                home_page.update(cx, |home, cx| {
                    home.focus_handle(cx).focus(window, cx);
                    cx.notify();
                });
            }
        });
    }

    /// 计算同基础名称的下一个可用标签序号；没有任何同名标签时返回 None（首标签不加序号）。
    fn next_available_tab_index(&self, base_title: &str, cx: &App) -> Option<usize> {
        let tab_container = self.active_tab_container(cx);
        let titles: Vec<String> = tab_container
            .read(cx)
            .tabs()
            .iter()
            .map(|tab| tab.title(cx).to_string())
            .collect();
        next_duplicate_tab_index(base_title, titles.iter().map(String::as_str))
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

        // 统计同一连接的 SSH 终端数量，计算序号（从 (1) 开始，复用已释放序号）
        let prefix = format!("ssh-terminal-{}-", conn_id);
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let base_title = conn.name.clone();
        let tab_index = self
            .next_available_tab_index(&base_title, cx)
            .or_else(|| (existing_count > 0).then_some(existing_count));
        let sync_path = Self::terminal_sync_path_enabled(cx);

        let terminal_view = cx.new(|cx| {
            TerminalWorkspace::new_ssh_with_index(conn, tab_index, window, cx, None, sync_path)
        });
        let subscription = cx.subscribe_in(
            &terminal_view,
            window,
            |this, _terminal, event: &TerminalWorkspaceEvent, window, cx| match event {
                TerminalWorkspaceEvent::OpenSftp(connection) => {
                    this.open_sftp_view(connection.clone(), window, cx);
                }
            },
        );
        self._subscriptions.push(subscription);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                let tab = TabItem::new(tab_id, "ssh", terminal_view);
                tc.add_tab_with_mode(tab, mode, window, cx);
            });
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
        let base_title = conn.name.clone();
        let tab_index = self
            .next_available_tab_index(&base_title, cx)
            .or_else(|| (existing_count > 0).then_some(existing_count));

        let terminal_view =
            cx.new(|cx| TerminalWorkspace::new_serial_with_index(conn, tab_index, window, cx));
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                let tab = TabItem::new(tab_id, "serial", terminal_view);
                tc.add_tab_with_mode(tab, mode, window, cx);
            });
        });
    }

    pub(crate) fn open_telnet_terminal(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_telnet_terminal_with_mode(conn, TabOpenMode::Activate, window, cx);
    }

    pub(crate) fn open_telnet_terminal_with_mode(
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
        let tab_id = format!("telnet-terminal-{}-{}", conn_id, timestamp);

        let prefix = format!("telnet-terminal-{}-", conn_id);
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
            cx.new(|cx| TerminalWorkspace::new_telnet_with_index(conn, tab_index, window, cx));
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                let tab = TabItem::new(tab_id, "telnet", terminal_view);
                tc.add_tab_with_mode(tab, mode, window, cx);
            });
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

        // 统计同一连接的 SFTP 视图数量，计算序号（从 (1) 开始，复用已释放序号）
        let prefix = format!("sftp-{}-", conn_id);
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let base_title = conn.name.clone();
        let tab_index = self
            .next_available_tab_index(&base_title, cx)
            .or_else(|| (existing_count > 0).then_some(existing_count));

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
                        // 统计同一连接的 SSH 终端数量，计算序号（从 (1) 开始，复用已释放序号）
                        let prefix = format!("ssh-terminal-{}-", conn_id);
                        let existing = event_tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .filter(|t| t.id().starts_with(&prefix))
                            .count();
                        let base_title = conn.name.clone();
                        let titles: Vec<String> = event_tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .map(|tab| tab.title(cx).to_string())
                            .collect();
                        let idx = next_duplicate_tab_index(
                            &base_title,
                            titles.iter().map(String::as_str),
                        )
                        .or_else(|| (existing > 0).then_some(existing));
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
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.add_and_activate_tab_with_focus(tab, window, cx);
            });
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
        let Some(conn) = resolve_connection_credentials(&conn, window, cx) else {
            return;
        };
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
        let base_title = conn.name.clone();
        let tab_index = self
            .next_available_tab_index(&base_title, cx)
            .or_else(|| (existing_count > 0).then_some(existing_count));
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
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                let tab = TabItem::new(tab_id, tab_kind, view);
                tc.add_tab_with_mode(tab, mode, window, cx);
            });
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

        let sort_order = AppSettings::global(cx).connection_sort_order;
        let (tab_id, connections, workspace_for_tab) =
            redis_tab_open_context(open_mode, &conn, workspace, &self.connections, sort_order);

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
        let connection_sort_order = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).connection_sort_order
        } else {
            ConnectionSortOrder::default()
        };

        let workspace_id = workspace.as_ref().and_then(|ws| ws.id);
        let active_conn_id = conn.id;

        let (tab_id, connections, workspace_for_tab) = match open_mode {
            DatabaseOpenMode::Workspace if workspace_id.is_some() => {
                let mut connections: Vec<StoredConnection> = self
                    .connections
                    .iter()
                    .filter(|connection| connection.workspace_id == workspace_id)
                    .filter(|connection| connection.connection_type == ConnectionType::MongoDB)
                    .cloned()
                    .collect();
                crate::connection_sort::sort_connections(&mut connections, connection_sort_order);
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

    pub(crate) fn open_mqtt_tab_with_mode(
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
        let connection_sort_order = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).connection_sort_order
        } else {
            ConnectionSortOrder::default()
        };

        let workspace_id = workspace.as_ref().and_then(|ws| ws.id);
        let active_conn_id = conn.id;

        let (tab_id, connections, workspace_for_tab) = match open_mode {
            DatabaseOpenMode::Workspace if workspace_id.is_some() => {
                let mut connections: Vec<StoredConnection> = self
                    .connections
                    .iter()
                    .filter(|connection| connection.workspace_id == workspace_id)
                    .filter(|connection| connection.connection_type == ConnectionType::Mqtt)
                    .cloned()
                    .collect();
                crate::connection_sort::sort_connections(&mut connections, connection_sort_order);
                let tab_id = format!("workspace-mqtt-tab-{}", workspace_id.unwrap_or(0));
                (tab_id, connections, workspace)
            }
            _ => {
                let conn_id = conn.id.unwrap_or(0);
                let tab_id = format!("mqtt-{conn_id}");
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
                        let mqtt_view = cx.new(|cx| {
                            mqtt_view::MqttTabView::new_with_active_conn(
                                workspace_for_tab,
                                connections,
                                active_conn_id,
                                window,
                                cx,
                            )
                        });
                        TabItem::new(tab_id_for_tab, "mqtt", mqtt_view)
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

    pub(crate) fn add_sync_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings-sync",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new_sync(win, cx));
                        TabItem::new("settings-sync", "home", settings)
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

    pub(crate) fn add_json_formatter_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "json-formatter",
                    |window, cx| {
                        let view = cx.new(|cx| JsonFormatterView::new(window, cx));
                        TabItem::new("json-formatter", "home", view)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_session_logs_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "session-logs",
                    |window, cx| {
                        let page = cx.new(|cx| SessionLogsPage::new(window, cx));
                        TabItem::new("session-logs", "home", page)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_credential_vault_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.active_tab_container(cx);
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy(
                    "credential-vault",
                    |window, cx| {
                        let vault = cx.new(|cx| CredentialVaultView::new(window, cx));
                        TabItem::new("credential-vault", "home", vault)
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

        // 统计已有本地终端数量，计算序号（从 (1) 开始，复用已释放序号）
        let tab_container = self.active_tab_container(cx);
        let existing_count = tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with("terminal-") || t.id().starts_with("local-terminal-"))
            .count();
        let tab_index = self
            .next_available_tab_index("Terminal", cx)
            .or_else(|| (existing_count > 0).then_some(existing_count));

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
        let Some((conn_clone, mut connections)) = database_tab_connection_context(
            open_mode,
            conn,
            workspace_id,
            &self.connections,
            |connection| resolve_connection_credentials(connection, window, cx),
        ) else {
            return;
        };
        // 分组内的连接按设置中的排序方式排列
        let connection_sort_order = if cx.has_global::<AppSettings>() {
            AppSettings::global(cx).connection_sort_order
        } else {
            ConnectionSortOrder::default()
        };
        crate::connection_sort::sort_connections(&mut connections, connection_sort_order);

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
                    TerminalConnectionKind::Telnet => {
                        let conn_id = terminal_view.read(cx).connection_id(cx);
                        if let Some(conn_id) = conn_id {
                            if let Some(conn) = self
                                .connections
                                .iter()
                                .find(|c| c.id == Some(conn_id))
                                .cloned()
                            {
                                self.open_telnet_terminal(conn, window, cx);
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
    let mut params = conn.to_remote_desktop_params().ok()?;
    params.protocol = match protocol {
        RemoteDesktopProtocol::Rdp => one_core::storage::RemoteDesktopProtocol::Rdp,
        RemoteDesktopProtocol::Vnc => one_core::storage::RemoteDesktopProtocol::Vnc,
    };
    Some(RemoteDesktopConnectionOptions::from_storage_params(params))
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
