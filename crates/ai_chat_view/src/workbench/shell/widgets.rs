//! 工作台外壳的无状态渲染助手与 trait 实现。

use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, Sizable as _, Size, h_flex};
use one_assets::IconName;
use one_core::sidebar_contribution::SidebarPlacement;
use rust_i18n::t;

use super::super::state::WorkbenchPanelKind;
use crate::theme::AgentChatTheme;


pub(super) fn seg_button(
    kind: WorkbenchPanelKind,
    active: bool,
    theme: &AgentChatTheme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let hover = theme.hover_background();
    h_flex()
        .id(SharedString::from(format!("workbench-switch-{}", kind.id())))
        .h(px(26.0))
        .px_2()
        .gap_1p5()
        .rounded(theme.surface_radius)
        .cursor_pointer()
        .bg(if active {
            theme.panel_hover
        } else {
            theme.background
        })
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .hover(move |style| style.bg(hover))
        .child(Icon::new(kind.icon()).with_size(Size::Small))
        .child(div().text_sm().child(kind.title()))
        .on_click(on_click)
}

pub(super) fn next_placement(current: SidebarPlacement) -> SidebarPlacement {
    match current {
        SidebarPlacement::Left => SidebarPlacement::Right,
        SidebarPlacement::Right => SidebarPlacement::Bottom,
        SidebarPlacement::Bottom => SidebarPlacement::Left,
    }
}

pub(super) fn placement_icon(placement: SidebarPlacement) -> IconName {
    match placement {
        SidebarPlacement::Left => IconName::PanelLeft,
        SidebarPlacement::Right => IconName::PanelRight,
        SidebarPlacement::Bottom => IconName::PanelBottom,
    }
}

pub(super) fn placement_tooltip(placement: SidebarPlacement) -> String {
    let label = match placement {
        SidebarPlacement::Left => t!("Workbench.placement_left"),
        SidebarPlacement::Right => t!("Workbench.placement_right"),
        SidebarPlacement::Bottom => t!("Workbench.placement_bottom"),
    };
    t!("Workbench.move_to", placement = label).to_string()
}
