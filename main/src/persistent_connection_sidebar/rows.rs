use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use rust_i18n::t;

use super::tree_model::ConnectionTreeRow;
use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

const TREE_INDENT: f32 = 16.0;
const TREE_BASE_PADDING: f32 = 8.0;
const TREE_ROW_HEIGHT: f32 = 32.0;

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
        h_flex()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .w_full()
            .h(px(TREE_ROW_HEIGHT))
            .border_l_2()
            .border_color(gpui::transparent_black())
            .pl(px(TREE_BASE_PADDING + depth as f32 * TREE_INDENT))
            .pr_1()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .hover(move |this| this.bg(palette.muted))
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    if !this.collapsed_workspaces.remove(&id) {
                        this.collapsed_workspaces.insert(id);
                    }
                    cx.notify();
                });
            })
            .child(tree_chevron(has_children, expanded))
            .child(Icon::new(IconName::FolderOpen).with_size(Size::Small))
            .child(tree_label(name))
            .child(tree_count(direct_connection_count, palette))
            .child(self.render_workspace_actions(id, group, cx))
            .into_any_element()
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
        let selected = home.read(cx).selected_connection_id == Some(id);
        let icon = connection
            .as_ref()
            .map(|connection| home.read(cx).connection_icon(connection, px(16.0)))
            .unwrap_or_else(|| Icon::new(IconName::Apps).with_size(Size::Small));
        h_flex()
            .id(SharedString::from(format!("persistent-connection-{id}")))
            .w_full()
            .h(px(TREE_ROW_HEIGHT))
            .border_l_2()
            .border_color(if selected {
                palette.accent
            } else {
                gpui::transparent_black()
            })
            .pl(px(TREE_BASE_PADDING
                + depth as f32 * TREE_INDENT
                + TREE_INDENT))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .when(selected, |this| this.bg(palette.muted))
            .hover(move |this| this.bg(palette.muted))
            .on_click(move |_, window, cx| {
                if let Some(connection) = connection.as_ref() {
                    home.update(cx, |home, cx| {
                        home.open_connection_from_quick(connection, window, cx)
                    });
                }
            })
            .child(icon)
            .child(tree_label(name))
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
        h_flex()
            .id("persistent-unassigned")
            .w_full()
            .h(px(TREE_ROW_HEIGHT))
            .border_l_2()
            .border_color(gpui::transparent_black())
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_color(palette.foreground)
            .hover(move |this| this.bg(palette.muted))
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.unassigned_collapsed = !this.unassigned_collapsed;
                    cx.notify();
                })
            })
            .child(tree_chevron(true, expanded))
            .child(Icon::new(IconName::FolderOpen).with_size(Size::Small))
            .child(tree_label(t!("Home.unassigned_workspace").to_string()))
            .child(tree_count(count, palette))
            .into_any_element()
    }
}

fn child_group_button(id: i64, home: gpui::Entity<crate::home_tab::HomePage>) -> Button {
    tree_action_button("child", id, IconName::Plus).on_click(move |_, window, cx| {
        let initial_sort_order = home.read(cx).workspaces.len() as i32;
        show_workspace_dialog(
            home.clone(),
            WorkspaceDialogConfig {
                parent_id: Some(id),
                initial_sort_order: Some(initial_sort_order),
                ..Default::default()
            },
            window,
            cx,
        );
    })
}

fn edit_group_button(
    id: i64,
    workspace: one_core::storage::Workspace,
    home: gpui::Entity<crate::home_tab::HomePage>,
) -> Button {
    tree_action_button("edit", id, IconName::Edit).on_click(move |_, window, cx| {
        show_workspace_dialog(
            home.clone(),
            WorkspaceDialogConfig {
                workspace_id: Some(id),
                parent_id: workspace.parent_id,
                initial_name: workspace.name.clone(),
                initial_sort_order: workspace.sort_order,
            },
            window,
            cx,
        );
    })
}

fn delete_group_button(id: i64, home: gpui::Entity<crate::home_tab::HomePage>) -> Button {
    tree_action_button("delete", id, IconName::Remove)
        .danger()
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| home.delete_workspace(id, window, cx));
        })
}

fn tree_chevron(has_children: bool, expanded: bool) -> AnyElement {
    div()
        .w(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .when(has_children, |this| {
            this.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(Size::XSmall),
            )
        })
        .into_any_element()
}

fn tree_label(label: String) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_sm()
        .child(label)
        .into_any_element()
}

fn tree_count(count: usize, palette: SidebarPalette) -> AnyElement {
    div()
        .px_1p5()
        .rounded_full()
        .bg(palette.muted)
        .text_xs()
        .text_color(palette.muted_foreground)
        .child(count.to_string())
        .into_any_element()
}

fn tree_action_button(action: &'static str, id: i64, icon: IconName) -> Button {
    Button::new(format!("persistent-{action}-{id}"))
        .icon(icon)
        .ghost()
        .xsmall()
}
