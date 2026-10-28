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
        self.tab_container
            .update(cx, |container, cx| container.add_tab_with_mode(item, mode, window, cx));
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

    fn notify(
        &mut self,
        level: NotifyLevel,
        message: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let notification = match level {
            NotifyLevel::Info => Notification::info(message.to_string()),
            NotifyLevel::Warning => Notification::warning(message.to_string()),
            NotifyLevel::Error => Notification::error(message.to_string()),
        };
        window.push_notification(notification, cx);
    }
}
