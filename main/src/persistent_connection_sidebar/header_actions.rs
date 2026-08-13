use gpui::{Anchor, AnyElement, Entity, IntoElement};
use gpui_component::{
    IconName,
    button::{IconButton, IconButtonRole},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
};
use rust_i18n::t;

use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

impl PersistentConnectionSidebar {
    pub(super) fn header_actions_menu(
        &self,
        view: Entity<Self>,
        palette: SidebarPalette,
    ) -> AnyElement {
        let context = HeaderActionsContext {
            view,
            home: self.home_page.clone(),
            hide_empty_workspaces: self.hide_empty_workspaces,
        };
        IconButton::new("persistent-header-actions-menu", IconName::Ellipsis)
            .role(IconButtonRole::Compact)
            .text_color(palette.foreground)
            .tooltip(t!("Common.more"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                build_header_actions_menu(menu, &context)
            })
            .into_any_element()
    }
}

struct HeaderActionsContext {
    view: Entity<PersistentConnectionSidebar>,
    home: Entity<crate::home_tab::HomePage>,
    hide_empty_workspaces: bool,
}

fn build_header_actions_menu(menu: PopupMenu, context: &HeaderActionsContext) -> PopupMenu {
    menu.item(collapse_all_item(context))
        .item(hide_empty_workspaces_item(context))
        .separator()
        .item(new_workspace_item(context))
        .item(refresh_item(context))
}

fn collapse_all_item(context: &HeaderActionsContext) -> PopupMenuItem {
    let view = context.view.clone();
    PopupMenuItem::new(t!("Connection.collapse_all").to_string())
        .icon(IconName::ChevronsUpDown)
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| this.collapse_all_groups(cx));
        })
}

fn hide_empty_workspaces_item(context: &HeaderActionsContext) -> PopupMenuItem {
    let view = context.view.clone();
    let hide_empty_workspaces = context.hide_empty_workspaces;
    PopupMenuItem::new(t!("Connection.hide_empty_workspaces").to_string())
        .icon(IconName::EyeOff)
        .checked(hide_empty_workspaces)
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.set_hide_empty_workspaces(!hide_empty_workspaces, cx);
            });
        })
}

fn new_workspace_item(context: &HeaderActionsContext) -> PopupMenuItem {
    let home = context.home.clone();
    PopupMenuItem::new(t!("Workspace.new").to_string())
        .icon(IconName::FolderOpen)
        .on_click(move |_, window, cx| {
            let sort_order = home.read(cx).workspaces.len() as i32;
            show_workspace_dialog(
                home.clone(),
                WorkspaceDialogConfig {
                    initial_sort_order: Some(sort_order),
                    ..Default::default()
                },
                window,
                cx,
            );
        })
}

fn refresh_item(context: &HeaderActionsContext) -> PopupMenuItem {
    let home = context.home.clone();
    PopupMenuItem::new(t!("Home.refresh").to_string())
        .icon(IconName::Refresh)
        .on_click(move |_, _, cx| {
            home.update(cx, |home, cx| home.refresh_local_home_data(cx));
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn overflow_menu_contains_connection_header_actions() {
        let source = include_str!("header_actions.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(implementation.contains("persistent-header-actions-menu"));
        assert!(!implementation.contains("view.read(cx)"));
        assert!(implementation.contains("Connection.collapse_all"));
        assert!(implementation.contains("Connection.hide_empty_workspaces"));
        assert!(implementation.contains("Workspace.new"));
        assert!(implementation.contains("Home.refresh"));
    }
}
