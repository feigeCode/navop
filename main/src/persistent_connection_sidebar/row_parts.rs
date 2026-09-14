use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Hsla, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Icon, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use one_assets::IconName;
use one_core::storage::{ConnectionType, StoredConnection};
use rust_i18n::t;

use super::SidebarPalette;
use crate::connection_visuals::connection_type_label;
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

pub(super) fn tree_chevron(has_children: bool, expanded: bool, _cx: &gpui::App) -> AnyElement {
    let disclosure_size = one_ui::theme_geometry().tree.disclosure_size;
    div()
        .w(disclosure_size)
        .h(disclosure_size)
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

/// 名称 + 类型 tag 同一行：名称可截断，tag 紧跟其后，不会被推到行尾。
pub(super) fn tree_connection_name_with_type_tag(
    name: String,
    type_tag: Option<AnyElement>,
) -> AnyElement {
    h_flex()
        .flex_1()
        .min_w_0()
        .gap_1p5()
        .items_center()
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_sm()
                .child(name),
        )
        .when_some(type_tag, |row, tag| row.child(tag))
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

pub(super) fn tree_connection_icon_slot(icon: Icon, palette: SidebarPalette) -> AnyElement {
    div()
        .size(px(24.0))
        .flex_shrink_0()
        .rounded(px(6.0))
        .border_1()
        .border_color(palette.border)
        .bg(palette.muted)
        .flex()
        .items_center()
        .justify_center()
        .child(icon.with_size(one_ui::IconSize::Default))
        .into_any_element()
}

/// 连接类型小标签：类型色圆点 + 前景色文案 + muted 底。
/// 文案始终用 foreground，避免 accent/灰字在侧栏里看不清。
pub(super) fn connection_type_tag(connection: &StoredConnection, cx: &gpui::App) -> AnyElement {
    let kind = connection.connection_type;
    if kind == ConnectionType::All {
        return div().into_any_element();
    }
    let dot = connection_type_tag_color(kind, cx);
    h_flex()
        .flex_shrink_0()
        .max_w(px(96.0))
        .gap_1()
        .items_center()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(cx.theme().muted)
        .child(
            div()
                .size(px(6.0))
                .flex_shrink_0()
                .rounded_full()
                .bg(dot),
        )
        .child(
            div()
                .min_w_0()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(cx.theme().foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(connection_type_tag_label(kind)),
        )
        .into_any_element()
}

fn connection_type_tag_label(kind: ConnectionType) -> String {
    match kind {
        ConnectionType::SshSftp => t!("ConnectionType.server").to_string(),
        _ => connection_type_label(kind),
    }
}

fn connection_type_tag_color(kind: ConnectionType, cx: &gpui::App) -> Hsla {
    match kind {
        ConnectionType::Database => cx.theme().blue,
        ConnectionType::SshSftp => cx.theme().success,
        ConnectionType::Redis => cx.theme().warning,
        ConnectionType::MongoDB => cx.theme().info,
        ConnectionType::Mqtt => cx.theme().accent,
        ConnectionType::Serial => cx.theme().info,
        ConnectionType::Telnet => cx.theme().info,
        ConnectionType::PortForwarding => cx.theme().warning,
        ConnectionType::Rdp => cx.theme().blue,
        ConnectionType::Vnc => cx.theme().accent,
        ConnectionType::Extension => cx.theme().muted_foreground,
        ConnectionType::All => cx.theme().muted_foreground,
    }
}

pub(super) fn connection_team_indicator(
    connection: &StoredConnection,
    teams: &[one_core::cloud_sync::TeamOption],
    cx: &gpui::App,
) -> Option<AnyElement> {
    let badge = connection_team_badge(connection.team_id.as_deref(), teams)?;
    // 与主页卡片团队徽标同一中性样式（redesign §5.4）：muted 底、
    // active 用前景色、departed/unknown 用弱化前景色；不带独立 hitbox，
    // 避免截获行 hover。完整名称与状态放 tooltip 之外不展示，与卡片一致。
    Some(
        div()
            .flex_shrink_0()
            .max_w(px(92.0))
            .px_1p5()
            .py_0p5()
            .rounded(px(4.0))
            .bg(cx.theme().muted)
            .text_color(if badge.active {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            })
            .text_xs()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
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
