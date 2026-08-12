use gpui::{Anchor, AnyElement, IntoElement, ParentElement, Styled, div};
use gpui_component::{
    IconName,
    button::{IconButton, IconButtonRole},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
};
use rust_i18n::t;

use super::tree_model::ConnectionTreeRow;
use super::{PersistentConnectionSidebar, SidebarPalette};

impl PersistentConnectionSidebar {
    pub(super) fn render_batch_toolbar(
        &self,
        rows: &[ConnectionTreeRow],
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let selected_count = self.connection_selection.len();
        let visible_ids = self.manageable_visible_connection_ids(rows, cx);
        let move_targets = self.move_targets(cx);
        let selected_ids = self.connection_selection.ids();
        let view = cx.entity();
        h_flex()
            .w_full()
            .h_9()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_r_1()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.muted)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_xs()
                    .text_color(palette.foreground)
                    .child(t!("Connection.batch_selected", count = selected_count).to_string()),
            )
            .child(select_visible_button(view.clone(), visible_ids))
            .child(move_connections_button(
                view.clone(),
                self.home_page.clone(),
                selected_ids.clone(),
                move_targets,
            ))
            .child(delete_connections_button(
                self.home_page.clone(),
                selected_ids,
            ))
            .child(clear_selection_button(view))
            .into_any_element()
    }

    fn manageable_visible_connection_ids(
        &self,
        rows: &[ConnectionTreeRow],
        cx: &gpui::App,
    ) -> Vec<i64> {
        let home = self.home_page.read(cx);
        rows.iter()
            .filter_map(|row| match row {
                ConnectionTreeRow::Connection { id, .. } if home.can_move_connection(*id) => {
                    Some(*id)
                }
                _ => None,
            })
            .collect()
    }

    fn move_targets(&self, cx: &gpui::App) -> Vec<(Option<i64>, String)> {
        let home = self.home_page.read(cx);
        std::iter::once((None, t!("Home.unassigned_workspace").to_string()))
            .chain(
                home.workspaces
                    .iter()
                    .filter_map(|workspace| Some((Some(workspace.id?), workspace.name.clone()))),
            )
            .collect()
    }
}

fn select_visible_button(
    view: gpui::Entity<PersistentConnectionSidebar>,
    visible_ids: Vec<i64>,
) -> IconButton {
    IconButton::new("persistent-select-visible-connections", IconName::Check)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.batch_select_visible"))
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.connection_selection.replace(visible_ids.clone());
                cx.notify();
            });
        })
}

fn move_connections_button(
    view: gpui::Entity<PersistentConnectionSidebar>,
    home: gpui::Entity<crate::home_tab::HomePage>,
    selected_ids: Vec<i64>,
    move_targets: Vec<(Option<i64>, String)>,
) -> AnyElement {
    IconButton::new("persistent-move-selected-connections", IconName::Folder)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.move_to_group"))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            append_move_targets(menu, &view, &home, &selected_ids, &move_targets)
        })
        .into_any_element()
}

fn append_move_targets(
    menu: PopupMenu,
    view: &gpui::Entity<PersistentConnectionSidebar>,
    home: &gpui::Entity<crate::home_tab::HomePage>,
    selected_ids: &[i64],
    move_targets: &[(Option<i64>, String)],
) -> PopupMenu {
    move_targets
        .iter()
        .cloned()
        .fold(menu, |menu, (workspace_id, label)| {
            let view = view.clone();
            let home = home.clone();
            let selected_ids = selected_ids.to_vec();
            menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                home.update(cx, |home, cx| {
                    home.move_connections_to_workspace(selected_ids.clone(), workspace_id, cx);
                });
                view.update(cx, |this, cx| {
                    this.connection_selection.clear();
                    cx.notify();
                });
            }))
        })
}

fn delete_connections_button(
    home: gpui::Entity<crate::home_tab::HomePage>,
    selected_ids: Vec<i64>,
) -> IconButton {
    IconButton::new("persistent-delete-selected-connections", IconName::Delete)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Common.delete"))
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.confirm_delete_connections(selected_ids.clone(), window, cx);
            });
        })
}

fn clear_selection_button(view: gpui::Entity<PersistentConnectionSidebar>) -> IconButton {
    IconButton::new("persistent-clear-connection-selection", IconName::Close)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.batch_clear_selection"))
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.connection_selection.clear();
                cx.notify();
            });
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn toolbar_exposes_move_delete_select_visible_and_clear_actions() {
        let source = include_str!("batch_toolbar.rs");
        assert!(source.contains("persistent-select-visible-connections"));
        assert!(source.contains("persistent-move-selected-connections"));
        assert!(source.contains("persistent-delete-selected-connections"));
        assert!(source.contains("persistent-clear-connection-selection"));
    }
}
