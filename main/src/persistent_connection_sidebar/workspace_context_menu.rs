use gpui::{Entity, Window};
use gpui_component::{
    IconName,
    menu::{PopupMenu, PopupMenuItem},
};
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

impl PersistentConnectionSidebar {
    pub(super) fn build_workspace_context_menu(
        menu: PopupMenu,
        view: &Entity<Self>,
        workspace_id: i64,
        expanded: bool,
        window: &mut Window,
        cx: &mut gpui::Context<PopupMenu>,
    ) -> PopupMenu {
        let home = view.read(cx).home_page.clone();
        let Some(workspace) = home
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .cloned()
        else {
            return menu;
        };
        let sort_order = home.read(cx).workspaces.len() as i32;
        let toggle_view = view.clone();
        let child_home = home.clone();
        let new_home = home.clone();
        let rename_home = home.clone();
        let delete_home = home.clone();

        menu.item(new_child_item(child_home, workspace_id, sort_order))
            .item(new_connection_item(new_home))
            .separator()
            .item(
                PopupMenuItem::new(expansion_label(expanded))
                    .icon(expansion_icon(expanded))
                    .on_click(window.listener_for(&toggle_view, move |this, _, _, cx| {
                        if !this.collapsed_workspaces.remove(&workspace_id) {
                            this.collapsed_workspaces.insert(workspace_id);
                        }
                        cx.notify();
                    })),
            )
            .item(
                PopupMenuItem::new(t!("Workspace.rename").to_string())
                    .icon(IconName::Edit)
                    .on_click(move |_, window, cx| {
                        show_workspace_dialog(
                            rename_home.clone(),
                            WorkspaceDialogConfig {
                                workspace_id: Some(workspace_id),
                                parent_id: workspace.parent_id,
                                initial_name: workspace.name.clone(),
                                initial_sort_order: workspace.sort_order,
                            },
                            window,
                            cx,
                        );
                    }),
            )
            .item(
                PopupMenuItem::new(t!("Workspace.delete").to_string())
                    .icon(IconName::Remove)
                    .on_click(move |_, window, cx| {
                        delete_home.update(cx, |home, cx| {
                            home.delete_workspace(workspace_id, window, cx);
                        });
                    }),
            )
            .separator()
            .item(refresh_item(home))
    }

    pub(super) fn build_unassigned_context_menu(
        menu: PopupMenu,
        view: &Entity<Self>,
        expanded: bool,
        window: &mut Window,
        cx: &mut gpui::Context<PopupMenu>,
    ) -> PopupMenu {
        let home = view.read(cx).home_page.clone();
        let sort_order = home.read(cx).workspaces.len() as i32;
        let toggle_view = view.clone();
        let new_home = home.clone();
        let group_home = home.clone();

        menu.item(new_connection_item(new_home))
            .item(
                PopupMenuItem::new(t!("Workspace.new").to_string())
                    .icon(IconName::FolderOpen)
                    .on_click(move |_, window, cx| {
                        show_workspace_dialog(
                            group_home.clone(),
                            WorkspaceDialogConfig {
                                initial_sort_order: Some(sort_order),
                                ..Default::default()
                            },
                            window,
                            cx,
                        );
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new(expansion_label(expanded))
                    .icon(expansion_icon(expanded))
                    .on_click(window.listener_for(&toggle_view, |this, _, _, cx| {
                        this.unassigned_collapsed = !this.unassigned_collapsed;
                        cx.notify();
                    })),
            )
            .separator()
            .item(refresh_item(home))
    }
}

fn new_child_item(
    home: Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
    sort_order: i32,
) -> PopupMenuItem {
    PopupMenuItem::new(t!("Workspace.new_child").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            show_workspace_dialog(
                home.clone(),
                WorkspaceDialogConfig {
                    parent_id: Some(workspace_id),
                    initial_sort_order: Some(sort_order),
                    ..Default::default()
                },
                window,
                cx,
            );
        })
}

fn new_connection_item(home: Entity<crate::home_tab::HomePage>) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.new_connection").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.show_new_connection_dialog(window, cx);
            });
        })
}

fn refresh_item(home: Entity<crate::home_tab::HomePage>) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.refresh").to_string())
        .icon(IconName::Refresh)
        .on_click(move |_, _, cx| {
            home.update(cx, |home, cx| home.refresh_local_home_data(cx));
        })
}

fn expansion_label(expanded: bool) -> String {
    if expanded {
        t!("Workspace.collapse")
    } else {
        t!("Workspace.expand")
    }
    .to_string()
}

fn expansion_icon(expanded: bool) -> IconName {
    if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    }
}
