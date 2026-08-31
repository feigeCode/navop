use gpui::{
    App, Context, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, Selectable,
    checkbox::Checkbox,
    h_flex,
    list::{ListDelegate, ListState},
};

use crate::db_tree_view::DbTreeView;

// ============================================================================
// DatabaseListItem - 数据库筛选列表项
// ============================================================================

#[derive(IntoElement)]
pub struct DatabaseListItem {
    db_id: String,
    db_name: String,
    db_selected: bool,
    selected: bool,
    view: Entity<DbTreeView>,
    connection_id: String,
}

impl DatabaseListItem {
    pub fn new(
        db_id: String,
        db_name: String,
        is_selected: bool,
        selected: bool,
        view: Entity<DbTreeView>,
        connection_id: String,
    ) -> Self {
        Self {
            db_id,
            db_name,
            db_selected: is_selected,
            selected,
            view,
            connection_id,
        }
    }
}

impl Selectable for DatabaseListItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for DatabaseListItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view_item = self.view.clone();
        let conn_item = self.connection_id.clone();
        let db_name_item = self.db_name.clone();
        let db_name_display = self.db_name.clone();
        let is_selected = self.db_selected;

        h_flex()
            .id(SharedString::from(format!("db-item-{}", self.db_id)))
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .rounded(px(4.0))
            .when(self.selected, |el| el.bg(cx.theme().list_active))
            .when(!self.selected, |el| {
                el.hover(|style| style.bg(cx.theme().list_hover))
            })
            .on_click(move |_, _, cx| {
                view_item.update(cx, |this, cx| {
                    this.toggle_database_selection(&conn_item, &db_name_item, cx);
                });
            })
            .child(
                Checkbox::new(SharedString::from(format!("db-check-{}", self.db_id)))
                    .checked(is_selected),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(db_name_display),
            )
    }
}

// ============================================================================
// DatabaseListDelegate - 数据库筛选列表代理
// ============================================================================

pub struct DatabaseListDelegate {
    view: Entity<DbTreeView>,
    connection_id: String,
    pub(super) databases: Vec<(String, String)>,
    pub(super) filtered_databases: Vec<(String, String)>,
    selected_index: Option<IndexPath>,
}

impl DatabaseListDelegate {
    pub fn new(
        view: Entity<DbTreeView>,
        connection_id: String,
        databases: Vec<(String, String)>,
    ) -> Self {
        let filtered_databases = databases.clone();
        Self {
            view,
            connection_id,
            databases,
            filtered_databases,
            selected_index: None,
        }
    }
}

impl ListDelegate for DatabaseListDelegate {
    type Item = DatabaseListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> gpui::Task<()> {
        if query.is_empty() {
            self.filtered_databases = self.databases.clone();
        } else {
            let query_lower = query.to_lowercase();
            self.filtered_databases = self
                .databases
                .iter()
                .filter(|(_, name)| name.to_lowercase().contains(&query_lower))
                .cloned()
                .collect();
        }
        cx.notify();
        gpui::Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_databases.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let (db_id, db_name) = self.filtered_databases.get(ix.row)?.clone();
        let is_selected = self
            .view
            .read(cx)
            .is_database_selected(&self.connection_id, &db_name);
        let selected = Some(ix) == self.selected_index;

        Some(DatabaseListItem::new(
            db_id,
            db_name,
            is_selected,
            selected,
            self.view.clone(),
            self.connection_id.clone(),
        ))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }
}
