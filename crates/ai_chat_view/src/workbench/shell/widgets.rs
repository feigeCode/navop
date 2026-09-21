//! 工作台外壳的无状态渲染助手与 trait 实现。

use one_assets::IconName;
use one_core::sidebar_contribution::SidebarPlacement;
use rust_i18n::t;



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
