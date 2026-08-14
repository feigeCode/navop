use crate::connection_visuals::{ConnectionVisualSize, stored_connection_icon};
use crate::home_tab::{HomePage, connection_matches_query};
use db::ipc::IpcDriverRegistry;
use gpui::{
    App, Context, Entity, FontWeight, ParentElement, SharedString, Styled, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, WindowExt, h_flex,
    list::{ListDelegate, ListItem, ListState},
};
use one_core::storage::StoredConnection;

pub(crate) struct ConnectionQuickOpenDelegate {
    parent: Entity<HomePage>,
    external_driver_registry: IpcDriverRegistry,
    items: Vec<StoredConnection>,
    filtered_items: Vec<StoredConnection>,
    selected_index: Option<IndexPath>,
    search_query: String,
}

impl ConnectionQuickOpenDelegate {
    pub(crate) fn new(
        parent: Entity<HomePage>,
        external_driver_registry: IpcDriverRegistry,
    ) -> Self {
        Self {
            parent,
            external_driver_registry,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected_index: None,
            search_query: String::new(),
        }
    }

    pub(crate) fn update_items(&mut self, connections: &[StoredConnection]) {
        self.items = connections.to_vec();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_items = self.items.clone();
            return;
        }
        let query = self.search_query.to_lowercase();
        self.filtered_items = self
            .items
            .iter()
            .filter(|conn| quick_open_matches_connection(conn, &query))
            .cloned()
            .collect();
    }
}

fn quick_open_matches_connection(connection: &StoredConnection, query: &str) -> bool {
    connection_matches_query(connection, query)
        || connection
            .connection_type
            .label()
            .to_lowercase()
            .contains(query)
}

impl ListDelegate for ConnectionQuickOpenDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.search_query = query.to_string();
        self.apply_filter();
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let connection = self.filtered_items.get(ix.row)?.clone();
        let parent = self.parent.clone();
        let name = connection.name.clone();
        let connection_type = connection.connection_type;
        let icon = stored_connection_icon(
            &connection,
            ConnectionVisualSize::Tree,
            &self.external_driver_registry,
        );
        let connection_for_open = connection.clone();

        Some(
            ListItem::new(ix)
                .mx_2()
                .h(px(44.0))
                .px_3()
                .rounded(px(6.0))
                .on_click(move |_, window, cx| {
                    parent.update(cx, |this, cx| {
                        this.open_connection_from_quick(&connection_for_open, window, cx);
                    });
                    window.close_dialog(cx);
                })
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(div().flex_shrink_0().flex().items_center().child(icon))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(SharedString::from(connection_type.label())),
                        ),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        if let Some(ix) = self.selected_index {
            if let Some(connection) = self.filtered_items.get(ix.row).cloned() {
                let parent = self.parent.clone();
                parent.update(cx, |this, cx| {
                    this.open_connection_from_quick(&connection, window, cx);
                });
                window.close_dialog(cx);
            }
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{
        DatabaseType, DbConnectionConfig, RemoteDesktopParams, RemoteDesktopProtocol,
    };

    #[test]
    fn quick_open_matches_database_connection_by_ip() {
        let connection = StoredConnection::new_database(
            "Production".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: "Production".to_string(),
                host: "192.168.10.42".to_string(),
                port: 3306,
                username: "root".to_string(),
                password: String::new(),
                credential_reference: None,
                database: Some("app".to_string()),
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: std::collections::HashMap::new(),
            },
            None,
        );

        assert!(quick_open_matches_connection(&connection, "192.168.10.42"));
        assert!(quick_open_matches_connection(&connection, "168.10"));
        assert!(!quick_open_matches_connection(&connection, "10.0.0.1"));
    }

    #[test]
    fn quick_open_matches_remote_desktop_connections_by_ip() {
        let rdp = remote_desktop_connection(
            RemoteDesktopProtocol::Rdp,
            "rdp-production",
            "10.0.0.8",
            Some("administrator"),
        );
        let vnc = remote_desktop_connection(
            RemoteDesktopProtocol::Vnc,
            "vnc-production",
            "10.0.0.9",
            None,
        );

        assert!(quick_open_matches_connection(&rdp, "10.0.0.8"));
        assert!(quick_open_matches_connection(
            &rdp,
            "administrator@10.0.0.8"
        ));
        assert!(quick_open_matches_connection(&vnc, "10.0.0.9"));
        assert!(quick_open_matches_connection(&vnc, "10.0.0.9:5900"));
    }

    fn remote_desktop_connection(
        protocol: RemoteDesktopProtocol,
        name: &str,
        host: &str,
        username: Option<&str>,
    ) -> StoredConnection {
        StoredConnection::new_remote_desktop(
            name.to_string(),
            RemoteDesktopParams {
                protocol,
                host: host.to_string(),
                port: protocol.default_port(),
                username: username.map(str::to_string),
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
            },
            None,
        )
    }
}
