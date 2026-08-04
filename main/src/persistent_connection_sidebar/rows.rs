use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, ElementId, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div,
};
use gpui_component::{
    ActiveTheme, FunctionalIcon, IconName, IconSize, InteractiveElementExt, ObjectIcon, Sizable,
    h_flex, menu::ContextMenuExt,
};
use rust_i18n::t;

use super::drag::DragConnection;
use super::row_parts::{
    child_group_button, connection_team_indicator, delete_group_button, edit_group_button,
    tree_chevron, tree_count, tree_label,
};
use super::tree_model::ConnectionTreeRow;
use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::connection_visuals::ConnectionVisualSize;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_row(
        &self,
        row: ConnectionTreeRow,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        match row {
            row @ ConnectionTreeRow::Workspace { .. } => {
                self.render_workspace_row(row, palette, cx)
            }
            ConnectionTreeRow::Connection { id, name, depth } => {
                self.render_connection_row(id, name, depth, palette, cx)
            }
            ConnectionTreeRow::Unassigned {
                connection_count,
                expanded,
            } => self.render_unassigned_row(connection_count, expanded, palette, cx),
        }
    }

    fn render_workspace_row(
        &self,
        row: ConnectionTreeRow,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let ConnectionTreeRow::Workspace {
            id,
            name,
            depth,
            direct_connection_count,
            has_children,
            expanded,
        } = row
        else {
            return div().into_any_element();
        };
        let view = cx.entity();
        let group: SharedString = format!("persistent-workspace-{id}").into();
        let home_for_rename = self.home_page.clone();
        let rename_config = self.workspace_dialog_config(id, cx);
        let view_for_menu = view.clone();
        let tree = cx.theme().geometry.tree;
        h_flex()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .w_full()
            .h(tree.row_height)
            .border_l_2()
            .border_color(gpui::transparent_black())
            .pl(tree.base_padding + tree.indent * depth)
            .pr_1()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .hover(move |this| this.bg(palette.hover))
            .drag_over::<DragConnection>(move |this, _, _, _| {
                this.bg(palette.hover).border_color(palette.accent)
            })
            .on_drop(cx.listener(move |this, drag: &DragConnection, _, cx| {
                this.collapsed_workspaces.remove(&id);
                this.home_page.update(cx, |home, cx| {
                    home.move_connection_to_workspace(drag.connection_id, Some(id), cx);
                });
                cx.notify();
            }))
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    if !this.collapsed_workspaces.remove(&id) {
                        this.collapsed_workspaces.insert(id);
                    }
                    cx.notify();
                });
            })
            .when_some(rename_config, |this, config| {
                this.on_double_click(move |_, window, cx| {
                    cx.stop_propagation();
                    show_workspace_dialog(home_for_rename.clone(), config.clone(), window, cx);
                })
            })
            .context_menu(move |menu, window, cx| {
                Self::build_workspace_context_menu(menu, &view_for_menu, id, expanded, window, cx)
            })
            .child(tree_chevron(has_children, expanded, cx))
            .child(ObjectIcon::new(IconName::FolderOpen).with_size(IconSize::Default))
            .child(tree_label(name))
            .child(tree_count(direct_connection_count, palette))
            .child(self.render_workspace_actions(id, group, cx))
            .into_any_element()
    }

    fn workspace_dialog_config(
        &self,
        id: i64,
        cx: &gpui::Context<Self>,
    ) -> Option<WorkspaceDialogConfig> {
        self.home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(id))
            .map(|workspace| WorkspaceDialogConfig {
                workspace_id: Some(id),
                parent_id: workspace.parent_id,
                initial_name: workspace.name.clone(),
                initial_sort_order: workspace.sort_order,
            })
    }

    fn render_workspace_actions(
        &self,
        id: i64,
        group: SharedString,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let workspace = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|item| item.id == Some(id))
            .cloned();
        let Some(workspace) = workspace else {
            return div().into_any_element();
        };
        let home_for_child = self.home_page.clone();
        let home_for_edit = self.home_page.clone();
        let home_for_delete = self.home_page.clone();
        h_flex()
            .gap_0p5()
            .invisible()
            .group_hover(group, |this| this.visible())
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(child_group_button(id, home_for_child))
            .child(edit_group_button(id, workspace, home_for_edit))
            .child(delete_group_button(id, home_for_delete))
            .into_any_element()
    }

    fn render_connection_row(
        &self,
        id: i64,
        name: String,
        depth: usize,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let home = self.home_page.clone();
        let connection = home
            .read(cx)
            .connections
            .iter()
            .find(|item| item.id == Some(id))
            .cloned();
        let open_connection = connection.clone();
        let home_for_open = home.clone();
        let home_for_select = home.clone();
        let selected = home.read(cx).selected_connection_id == Some(id);
        let can_drag = home.read(cx).can_move_connection(id);
        let team_indicator = connection.as_ref().and_then(|connection| {
            connection_team_indicator(connection, home.read(cx).cached_team_options(), cx)
        });
        let icon = connection
            .as_ref()
            .map(|connection| {
                home.read(cx)
                    .connection_icon(connection, ConnectionVisualSize::Tree)
            })
            .unwrap_or_else(|| {
                FunctionalIcon::new(IconName::Apps)
                    .with_size(IconSize::Default)
                    .into_icon()
            });
        let drag = DragConnection {
            connection_id: id,
            name: name.clone(),
        };
        let view_for_menu = cx.entity();
        let tree = cx.theme().geometry.tree;
        h_flex()
            .id(SharedString::from(format!("persistent-connection-{id}")))
            .w_full()
            .h(tree.row_height)
            .border_l_2()
            .border_color(if selected {
                palette.selected_border
            } else {
                gpui::transparent_black()
            })
            .pl(tree.base_padding + tree.indent * (depth + 1))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .when(selected, |this| this.bg(palette.selected))
            .when(!selected, |this| {
                this.hover(move |this| this.bg(palette.hover))
            })
            .when(can_drag, |this| {
                this.on_drag(drag, |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                })
            })
            .on_double_click(move |_, window, cx| {
                if let Some(connection) = open_connection.as_ref() {
                    home_for_open.update(cx, |home, cx| {
                        home.open_connection_from_quick(connection, window, cx)
                    });
                }
            })
            .on_click(move |_, _, cx| {
                home_for_select.update(cx, |home, cx| {
                    home.selected_connection_id = Some(id);
                    cx.notify();
                });
            })
            .context_menu(move |menu, window, cx| {
                Self::build_connection_context_menu(menu, &view_for_menu, id, window, cx)
            })
            .child(icon)
            .child(tree_label(name))
            .when_some(team_indicator, |row, indicator| row.child(indicator))
            .into_any_element()
    }

    fn render_unassigned_row(
        &self,
        count: usize,
        expanded: bool,
        palette: SidebarPalette,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let view_for_menu = view.clone();
        let tree = cx.theme().geometry.tree;
        h_flex()
            .id("persistent-unassigned")
            .w_full()
            .h(tree.row_height)
            .border_l_2()
            .border_color(gpui::transparent_black())
            .px(tree.base_padding)
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .hover(move |this| this.bg(palette.hover))
            .drag_over::<DragConnection>(move |this, _, _, _| {
                this.bg(palette.hover).border_color(palette.accent)
            })
            .on_drop(cx.listener(|this, drag: &DragConnection, _, cx| {
                this.unassigned_collapsed = false;
                this.home_page.update(cx, |home, cx| {
                    home.move_connection_to_workspace(drag.connection_id, None, cx);
                });
                cx.notify();
            }))
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.unassigned_collapsed = !this.unassigned_collapsed;
                    cx.notify();
                })
            })
            .context_menu(move |menu, window, cx| {
                Self::build_unassigned_context_menu(menu, &view_for_menu, expanded, window, cx)
            })
            .child(tree_chevron(count > 0, expanded, cx))
            .child(ObjectIcon::new(IconName::FolderOpen).with_size(IconSize::Default))
            .child(tree_label(t!("Home.unassigned_workspace").to_string()))
            .child(tree_count(count, palette))
            .into_any_element()
    }
}
