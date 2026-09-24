//! 工作台外壳的无状态渲染助手：停靠按钮的图标与提示文案。
//!
//! 这些函数不碰 GPUI 布局，只把纯状态翻译成「长什么样、说什么话」，
//! 渲染方法直接消费即可。

use rust_i18n::t;

use crate::workbench::state::{WorkbenchPanelKind, WorkbenchPlacement};

/// 面板头「移到下一处」按钮的提示。
pub(super) fn cycle_placement_tooltip(next: WorkbenchPlacement) -> String {
    t!(
        "Workbench.move_to",
        placement = next.label().to_string()
    )
    .to_string()
}

/// 工具条按钮的提示（页签模型：点击打开/激活右侧标签组）。
pub(super) fn rail_tooltip(
    kind: WorkbenchPanelKind,
    placement: Option<WorkbenchPlacement>,
) -> String {
    let panel = kind.title().to_string();
    match placement {
        None => t!("Workbench.rail_open", panel = panel).to_string(),
        Some(_) => t!("Workbench.rail_focus_center", panel = panel).to_string(),
    }
}

/// 标签条末尾「并排打开」按钮的提示。
pub(super) fn pin_tooltip(kind: WorkbenchPanelKind) -> String {
    t!("Workbench.pin_right", panel = kind.title().to_string()).to_string()
}

/// 标签的关闭按钮提示。
pub(super) fn close_tab_tooltip(kind: WorkbenchPanelKind) -> String {
    t!("Workbench.close_tab", panel = kind.title().to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_tooltip_distinguishes_open_and_focus_states() {
        let closed = rail_tooltip(WorkbenchPanelKind::Files, None);
        let open = rail_tooltip(
            WorkbenchPanelKind::Files,
            Some(WorkbenchPlacement::Right),
        );

        assert!(!closed.is_empty());
        assert!(!open.is_empty());
        assert_ne!(closed, open);
    }
}
