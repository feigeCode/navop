use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use super::tree_model::ConnectionTreeRow;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

const TREE_INDENT: f32 = 16.0;
const TREE_BASE_PADDING: f32 = 8.0;

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_row(
        &self,
        row: ConnectionTreeRow,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        match row {
            row @ ConnectionTreeRow::Workspace { .. } => self.render_workspace_row(row, cx),
            ConnectionTreeRow::Connection { id, name, depth } => {
                self.render_connection_row(id, name, depth, cx)
            }
            ConnectionTreeRow::Unassigned {
                connection_count,
                expanded,
            } => self.render_unassigned_row(connection_count, expanded, cx),
        }
    }

    fn render_workspace_row(&self, row: ConnectionTreeRow, cx: &gpui::Context<Self>) -> AnyElement {
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
            .h(px(30.0))
            .pl(px(TREE_BASE_PADDING + depth as f32 * TREE_INDENT))
            .pr_1()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .hover(|this| this.bg(cx.theme().sidebar_accent))
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
            .child(tree_count(direct_connection_count, cx))
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
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let home = self.home_page.clone();
        let connection = home
            .read(cx)
            .connections
            .iter()
            .find(|item| item.id == Some(id))
            .cloned();
        let icon = connection
            .as_ref()
            .map(|item| item.connection_type.icon())
            .unwrap_or(IconName::Apps);
        h_flex()
            .id(SharedString::from(format!("persistent-connection-{id}")))
            .w_full()
            .h(px(30.0))
            .pl(px(TREE_BASE_PADDING
                + depth as f32 * TREE_INDENT
                + TREE_INDENT))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .hover(|this| this.bg(cx.theme().sidebar_accent))
            .on_click(move |_, window, cx| {
                if let Some(connection) = connection.as_ref() {
                    home.update(cx, |home, cx| {
                        home.open_connection_from_quick(connection, window, cx)
                    });
                }
            })
            .child(Icon::new(icon).color().with_size(Size::Small))
            .child(tree_label(name))
            .into_any_element()
    }

    fn render_unassigned_row(
        &self,
        count: usize,
        expanded: bool,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        h_flex()
            .id("persistent-unassigned")
            .w_full()
            .h(px(30.0))
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .hover(|this| this.bg(cx.theme().sidebar_accent))
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.unassigned_collapsed = !this.unassigned_collapsed;
                    cx.notify();
                })
            })
            .child(tree_chevron(true, expanded))
            .child(Icon::new(IconName::FolderOpen).with_size(Size::Small))
            .child(tree_label(t!("Home.unassigned_workspace").to_string()))
            .child(tree_count(count, cx))
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

fn tree_count(count: usize, cx: &gpui::App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(count.to_string())
        .into_any_element()
}

fn tree_action_button(action: &'static str, id: i64, icon: IconName) -> Button {
    Button::new(format!("persistent-{action}-{id}"))
        .icon(icon)
        .ghost()
        .xsmall()
}
