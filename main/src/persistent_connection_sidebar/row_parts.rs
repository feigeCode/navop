use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
};
use rust_i18n::t;

use super::SidebarPalette;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

pub(super) fn child_group_button(id: i64, home: gpui::Entity<crate::home_tab::HomePage>) -> Button {
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

pub(super) fn edit_group_button(
    id: i64,
    workspace: one_core::storage::Workspace,
    home: gpui::Entity<crate::home_tab::HomePage>,
) -> Button {
    tree_action_button("edit", id, IconName::Edit)
        .tooltip(t!("Workspace.rename"))
        .on_click(move |_, window, cx| {
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

pub(super) fn delete_group_button(
    id: i64,
    home: gpui::Entity<crate::home_tab::HomePage>,
) -> Button {
    tree_action_button("delete", id, IconName::Remove)
        .danger()
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| home.delete_workspace(id, window, cx));
        })
}

pub(super) fn tree_chevron(has_children: bool, expanded: bool) -> AnyElement {
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

pub(super) fn tree_label(label: String) -> AnyElement {
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

pub(super) fn tree_count(count: usize, palette: SidebarPalette) -> AnyElement {
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
