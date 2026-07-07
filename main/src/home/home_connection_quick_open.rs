use crate::external_driver_display::external_driver_icon_for_config_with_registry;
use crate::home_tab::HomePage;
use db::ipc::IpcDriverRegistry;
use gpui::{
    App, Context, Entity, FontWeight, ParentElement, SharedString, Styled, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IndexPath, Sizable, Size, WindowExt, h_flex,
    list::{ListDelegate, ListItem, ListState},
};
use one_core::storage::{ConnectionType, StoredConnection};

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
            .filter(|conn| {
                conn.name.to_lowercase().contains(&query)
                    || conn.connection_type.label().to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
    }
}

fn connection_icon(connection: &StoredConnection, registry: &IpcDriverRegistry) -> Icon {
    match connection.connection_type {
        ConnectionType::Database => connection
            .to_db_connection()
            .map(|config| {
                external_driver_icon_for_config_with_registry(&config, Size::Small, registry)
                    .unwrap_or_else(|| config.database_type.as_icon())
            })
            .unwrap_or_else(|_| Icon::new(ConnectionType::Database.icon()).color())
            .with_size(Size::Small),
        _ => Icon::new(connection.connection_type.icon())
            .color()
            .with_size(Size::Small),
    }
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
        let icon = connection_icon(&connection, &self.external_driver_registry);
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
