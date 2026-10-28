use crate::home_tab::HomePage;
use domain_api::{HomeContext, NotifyLevel, TabHost};
use gpui::{App, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use one_core::storage::{StoredConnection, Workspace};
use one_core::tab_container::{TabItem, TabOpenMode};

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

    fn notify(&mut self, level: NotifyLevel, message: &str, window: &mut Window, cx: &mut App) {
        let notification = match level {
            NotifyLevel::Info => Notification::info(message.to_string()),
            NotifyLevel::Warning => Notification::warning(message.to_string()),
            NotifyLevel::Error => Notification::error(message.to_string()),
        };
        window.push_notification(notification, cx);
    }
}
