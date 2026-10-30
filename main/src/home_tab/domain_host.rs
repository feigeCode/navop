use crate::home_tab::HomePage;
use domain_api::{HomeContext, NotifyLevel, TabHost};
use gpui::{App, Subscription, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use one_core::storage::{StoredConnection, Workspace};
use one_core::tab_container::{TabItem, TabOpenMode};
use std::sync::Arc;

impl TabHost for HomePage {
    fn open_tab(
        &mut self,
        item: TabItem,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let tab_container = self.tab_container.clone();
        let tab_id = item.id();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                if !container.activate_tab_by_id(tab_id.as_ref(), window, cx) {
                    container.add_tab_with_mode(item, mode, window, cx);
                }
            });
        });
        true
    }

    fn open_tab_with_focus(&mut self, item: TabItem, window: &mut Window, cx: &mut App) -> bool {
        let tab_container = self.tab_container.clone();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                container.add_and_activate_tab_with_focus(item, window, cx)
            });
        });
        true
    }

    fn open_tab_lazy(
        &mut self,
        id: String,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
        build: Box<dyn FnOnce(&mut Window, &mut App) -> TabItem>,
    ) -> bool {
        let tab_container = self.tab_container.clone();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                container.activate_or_add_tab_lazy_with_mode(
                    id,
                    mode,
                    move |window, cx| build(window, cx),
                    window,
                    cx,
                );
            });
        });
        true
    }
}

impl HomeContext for HomePage {
    fn connections(&self) -> &[StoredConnection] {
        &self.connections
    }

    fn workspace(&self, id: i64) -> Option<Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == Some(id))
            .cloned()
    }

    fn sort_connections(&self, connections: &mut [StoredConnection], cx: &App) {
        let sort_order = if cx.has_global::<crate::setting_tab::AppSettings>() {
            crate::setting_tab::AppSettings::global(cx).connection_sort_order
        } else {
            Default::default()
        };
        crate::connection_sort::sort_connections(connections, sort_order);
    }

    fn database_open_mode(&self, cx: &App) -> domain_api::DatabaseTabMode {
        let mode = if cx.has_global::<crate::setting_tab::AppSettings>() {
            crate::setting_tab::AppSettings::global(cx).database_open_mode
        } else {
            one_core::settings::DatabaseOpenMode::default()
        };
        match mode {
            one_core::settings::DatabaseOpenMode::Single => domain_api::DatabaseTabMode::Single,
            one_core::settings::DatabaseOpenMode::Workspace => {
                domain_api::DatabaseTabMode::Workspace
            }
        }
    }

    fn next_tab_index(&self, base_title: &str, cx: &App) -> Option<usize> {
        self.next_available_tab_index(base_title, cx)
    }

    fn keep_subscription(&mut self, subscription: Subscription) {
        self._subscriptions.push(subscription);
    }

    fn resolve_connection_credentials(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<StoredConnection> {
        crate::home_tab::connection_open::resolve_connection_credentials(connection, window, cx)
    }

    fn port_forwarding_runtime(
        &self,
    ) -> Option<Arc<tokio::sync::Mutex<domain_api::PortForwardingRuntime>>> {
        // 运行时由 shell 领域持有，公共壳只向注册表询问。
        app_kit::registry()
            .shell_extras()
            .and_then(|extras| extras.port_forwarding_runtime())
    }

    fn notify(&mut self, level: NotifyLevel, message: &str, window: &mut Window, cx: &mut App) {
        let notification = match level {
            NotifyLevel::Info => Notification::info(message.to_string()),
            NotifyLevel::Warning => Notification::warning(message.to_string()),
            NotifyLevel::Error => Notification::error(message.to_string()),
        };
        window.push_notification(notification, cx);
    }
}

/// 全量产品的 shell 宿主：领域事件回到当前 `HomePage`。
#[cfg(feature = "shell")]
pub(crate) struct NavopShellHost;

#[cfg(feature = "shell")]
impl app_kit::ShellHost for NavopShellHost {
    fn open_sftp(&self, connection: StoredConnection, window: &mut Window, cx: &mut App) {
        let Some(home) = cx
            .try_global::<crate::navop_app::GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home.update(cx, |home, cx| {
            home.open_sftp_view(connection, window, cx);
        });
    }

    fn save_temporary_connection(
        &self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(home) = cx
            .try_global::<crate::navop_app::GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home.update(cx, |home, cx| {
            home.show_save_temporary_connection_form(connection, window, cx);
        });
    }

    fn open_local_terminal(&self, working_dir: String, window: &mut Window, cx: &mut App) {
        let Some(home) = cx
            .try_global::<crate::navop_app::GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home.update(cx, |home, cx| {
            home.open_sftp_local_terminal(working_dir, window, cx);
        });
    }

    fn open_ssh_terminal(
        &self,
        connection: StoredConnection,
        working_dir: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(home) = cx
            .try_global::<crate::navop_app::GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home.update(cx, |home, cx| {
            home.open_sftp_ssh_terminal(connection, working_dir, window, cx);
        });
    }

    fn open_session_log_file(&self, path: std::path::PathBuf, window: &mut Window, cx: &mut App) {
        if let Some(extras) = app_kit::registry().shell_extras() {
            extras.open_session_log(path, window, cx);
        }
    }
}
