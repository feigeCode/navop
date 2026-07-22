use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    tooltip::Tooltip,
};
use rust_i18n::t;

use super::SidebarPalette;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};
use crate::home_tab::connection_team_badge;

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

pub(super) fn connection_team_indicator(
    connection: &one_core::storage::StoredConnection,
    teams: &[one_core::cloud_sync::TeamOption],
    cx: &gpui::App,
) -> Option<AnyElement> {
    let badge = connection_team_badge(connection.team_id.as_deref(), teams)?;
    let tooltip: SharedString = badge.tooltip.into();
    Some(
        div()
            .id(format!(
                "persistent-team-{}",
                connection.id.unwrap_or_default()
            ))
            .flex_shrink_0()
            .max_w(px(92.0))
            .px_1p5()
            .py_0p5()
            .rounded(px(4.0))
            .bg(if badge.active {
                cx.theme().primary
            } else {
                cx.theme().muted
            })
            .text_color(if badge.active {
                cx.theme().primary_foreground
            } else {
                cx.theme().muted_foreground
            })
            .text_xs()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(badge.name)
            .into_any_element(),
    )
}

fn tree_action_button(action: &'static str, id: i64, icon: IconName) -> Button {
    Button::new(format!("persistent-{action}-{id}"))
        .icon(icon)
        .ghost()
        .xsmall()
}
