use gpui::{Anchor, AnyElement, IntoElement, ParentElement, Styled, div};
use gpui_component::{
    IconName, Sizable,
    button::Toggle,
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
            .child(batch_actions_menu_button(BatchActionsMenuContext {
                view,
                home: self.home_page.clone(),
                visible_ids,
                selected_ids,
                move_targets,
            }))
            .into_any_element()
    }

    pub(super) fn manageable_visible_connection_ids(
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

pub(super) fn batch_mode_toggle(
    view: gpui::Entity<PersistentConnectionSidebar>,
    active: bool,
    palette: SidebarPalette,
) -> Toggle {
    Toggle::new("persistent-batch-connections")
        .icon(IconName::Check)
        .checked(active)
        .xsmall()
        .text_color(palette.foreground)
        .tooltip(t!("Connection.batch_operations"))
        .on_click(move |checked, _, cx| {
            view.update(cx, |this, cx| this.set_batch_mode(*checked, cx));
        })
}

#[derive(Clone)]
struct BatchActionsMenuContext {
    view: gpui::Entity<PersistentConnectionSidebar>,
    home: gpui::Entity<crate::home_tab::HomePage>,
    visible_ids: Vec<i64>,
    selected_ids: Vec<i64>,
    move_targets: Vec<(Option<i64>, String)>,
}

fn batch_actions_menu_button(context: BatchActionsMenuContext) -> AnyElement {
    IconButton::new("persistent-batch-actions-menu", IconName::Ellipsis)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.batch_operations"))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
            let move_item = move_connections_menu_item(&context, window, cx);
            menu.item(select_visible_menu_item(&context))
                .item(move_item)
                .item(delete_connections_menu_item(&context))
                .separator()
                .item(exit_batch_mode_menu_item(&context))
        })
        .into_any_element()
}

fn select_visible_menu_item(context: &BatchActionsMenuContext) -> PopupMenuItem {
    let view = context.view.clone();
    let visible_ids = context.visible_ids.clone();
    PopupMenuItem::new(t!("Connection.batch_select_visible").to_string())
        .icon(IconName::Check)
        .disabled(visible_ids.is_empty())
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.connection_selection.select_visible(&visible_ids);
                cx.notify();
            });
        })
}

fn move_connections_menu_item(
    context: &BatchActionsMenuContext,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenuItem {
    let move_context = context.clone();
    let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
        append_move_targets(submenu, &move_context)
    });
    PopupMenuItem::submenu(t!("Connection.move_to_group").to_string(), submenu)
        .icon(IconName::Folder)
        .disabled(context.selected_ids.is_empty())
}

fn append_move_targets(menu: PopupMenu, context: &BatchActionsMenuContext) -> PopupMenu {
    context
        .move_targets
        .iter()
        .cloned()
        .fold(menu, |menu, (workspace_id, label)| {
            let view = context.view.clone();
            let home = context.home.clone();
            let selected_ids = context.selected_ids.clone();
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

fn delete_connections_menu_item(context: &BatchActionsMenuContext) -> PopupMenuItem {
    let home = context.home.clone();
    let selected_ids = context.selected_ids.clone();
    PopupMenuItem::new(t!("Common.delete").to_string())
        .icon(IconName::Delete)
        .disabled(selected_ids.is_empty())
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.confirm_delete_connections(selected_ids.clone(), window, cx);
            });
        })
}

fn exit_batch_mode_menu_item(context: &BatchActionsMenuContext) -> PopupMenuItem {
    let view = context.view.clone();
    PopupMenuItem::new(t!("Connection.batch_exit").to_string())
        .icon(IconName::Close)
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| this.set_batch_mode(false, cx));
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn toolbar_exposes_batch_actions_through_overflow_menu() {
        let source = include_str!("batch_toolbar.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(implementation.contains("persistent-batch-actions-menu"));
        assert!(implementation.contains("PopupMenuItem::submenu"));
        assert!(implementation.contains("Connection.batch_select_visible"));
        assert!(implementation.contains("Connection.move_to_group"));
        assert!(implementation.contains("Common.delete"));
        assert!(implementation.contains("Connection.batch_exit"));
    }
}
