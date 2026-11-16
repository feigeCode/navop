//! 三边工具停靠：纯状态 + 纯几何。
//!
//! 终端侧栏与 AI 工作台外壳用的是同一套停靠语义（**每边最多一个面板**），此前两侧各抄了
//! 一份数据模型和宽度计算。这里放共同的那一份。
//!
//! 各自的开关/移动策略仍留在宿主里（终端侧栏有 `TerminalSidebar` 的状态，工作台有
//! `WorkbenchState`），这里只提供它们共用的落位与几何。

use gpui::Pixels;

use crate::layout::TOOLBAR_WIDTH;
use crate::sidebar_contribution::SidebarPlacement;

/// 三边停靠布局：每边一个槽位。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDockLayout<P> {
    pub left: Option<P>,
    pub right: Option<P>,
    pub bottom: Option<P>,
}

// 手写 `Default`：derive 会带上 `P: Default`，而面板枚举通常没有 `Default`。
impl<P> Default for ToolDockLayout<P> {
    fn default() -> Self {
        Self {
            left: None,
            right: None,
            bottom: None,
        }
    }
}

impl<P: Copy + PartialEq> ToolDockLayout<P> {
    pub fn from_open_panels(open_panels: impl IntoIterator<Item = (P, SidebarPlacement)>) -> Self {
        let mut layout = Self::default();
        for (panel, placement) in open_panels {
            layout.set_placement(panel, placement);
        }
        layout
    }

    pub fn placement_of(&self, panel: P) -> Option<SidebarPlacement> {
        if self.left == Some(panel) {
            Some(SidebarPlacement::Left)
        } else if self.right == Some(panel) {
            Some(SidebarPlacement::Right)
        } else if self.bottom == Some(panel) {
            Some(SidebarPlacement::Bottom)
        } else {
            None
        }
    }

    /// 把面板放到某一边，返回该边原有的面板（同边多面板由调用方决定怎么处理）。
    pub fn set_placement(&mut self, panel: P, placement: SidebarPlacement) -> Option<P> {
        match placement {
            SidebarPlacement::Left => self.left.replace(panel),
            SidebarPlacement::Right => self.right.replace(panel),
            SidebarPlacement::Bottom => self.bottom.replace(panel),
        }
    }

    pub fn clear_placement(&mut self, placement: SidebarPlacement) -> Option<P> {
        match placement {
            SidebarPlacement::Left => self.left.take(),
            SidebarPlacement::Right => self.right.take(),
            SidebarPlacement::Bottom => self.bottom.take(),
        }
    }
}

impl<P: PartialEq> ToolDockLayout<P> {
    /// 由打开面板列表构建布局（无 `Copy` 约束版本，供 `SharedString` 等键使用）。
    pub fn from_open_panels_cloned(
        open_panels: impl IntoIterator<Item = (P, SidebarPlacement)>,
    ) -> Self
    where
        P: Clone,
    {
        let mut layout = Self::default();
        for (panel, placement) in open_panels {
            layout.set_placement_cloned(panel, placement);
        }
        layout
    }

    /// 若指定边被占用则清空该边（用于扩展面板给内建面板让位）。
    pub fn clear_if_occupied(&mut self, placement: SidebarPlacement) -> Option<P> {
        self.take_placement(placement)
    }

    /// 把面板放到某一边（无 `Copy` 约束版本）。
    pub fn set_placement_cloned(&mut self, panel: P, placement: SidebarPlacement) -> Option<P> {
        match placement {
            SidebarPlacement::Left => self.left.replace(panel),
            SidebarPlacement::Right => self.right.replace(panel),
            SidebarPlacement::Bottom => self.bottom.replace(panel),
        }
    }

    /// 面板当前停靠的边。
    pub fn placement_edge(&self, panel: &P) -> Option<SidebarPlacement> {
        if self.left.as_ref() == Some(panel) {
            Some(SidebarPlacement::Left)
        } else if self.right.as_ref() == Some(panel) {
            Some(SidebarPlacement::Right)
        } else if self.bottom.as_ref() == Some(panel) {
            Some(SidebarPlacement::Bottom)
        } else {
            None
        }
    }

    /// 清空并返回指定边的面板。
    fn take_placement(&mut self, placement: SidebarPlacement) -> Option<P> {
        match placement {
            SidebarPlacement::Left => self.left.take(),
            SidebarPlacement::Right => self.right.take(),
            SidebarPlacement::Bottom => self.bottom.take(),
        }
    }
}

impl<P> ToolDockLayout<P> {
    pub fn has_right(&self) -> bool {
        self.right.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.bottom.is_none()
    }
}

/// 右侧停靠区占据的总宽度（面板 + 工具条）。
pub fn dock_region_width<P>(layout: &ToolDockLayout<P>, panel_size: Pixels) -> Pixels {
    if layout.has_right() {
        panel_size + TOOLBAR_WIDTH
    } else {
        TOOLBAR_WIDTH
    }
}

/// 拖动右侧分隔条时的新宽度（外层右边界 − 工具条 − 鼠标位置）。
pub fn right_sidebar_width(outer_right: Pixels, mouse_x: Pixels) -> Pixels {
    outer_right - TOOLBAR_WIDTH - mouse_x
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Panel {
        One,
        Two,
        Three,
    }

    #[test]
    fn maps_open_panels_to_edges() {
        let layout = ToolDockLayout::from_open_panels([
            (Panel::One, SidebarPlacement::Left),
            (Panel::Two, SidebarPlacement::Bottom),
            (Panel::Three, SidebarPlacement::Right),
        ]);

        assert_eq!(Some(Panel::One), layout.left);
        assert_eq!(Some(Panel::Three), layout.right);
        assert_eq!(Some(Panel::Two), layout.bottom);
        assert_eq!(
            Some(SidebarPlacement::Bottom),
            layout.placement_of(Panel::Two)
        );
        assert_eq!(
            Some(SidebarPlacement::Left),
            layout.placement_of(Panel::One)
        );
    }

    #[test]
    fn same_edge_only_keeps_the_last_panel() {
        let mut layout = ToolDockLayout::default();

        assert_eq!(
            None,
            layout.set_placement(Panel::One, SidebarPlacement::Right)
        );
        assert_eq!(
            Some(Panel::One),
            layout.set_placement(Panel::Two, SidebarPlacement::Right)
        );
        assert_eq!(Some(Panel::Two), layout.right);
        assert_eq!(None, layout.placement_of(Panel::One));
    }

    #[test]
    fn clearing_an_edge_empties_the_layout() {
        let mut layout = ToolDockLayout::from_open_panels([(Panel::One, SidebarPlacement::Left)]);

        assert_eq!(
            Some(Panel::One),
            layout.clear_placement(SidebarPlacement::Left)
        );
        assert!(layout.is_empty());
    }

    #[test]
    fn right_region_keeps_toolbar_width_without_right_panel() {
        let layout = ToolDockLayout::from_open_panels([(Panel::One, SidebarPlacement::Left)]);

        assert_eq!(TOOLBAR_WIDTH, dock_region_width(&layout, px(420.0)));
    }

    #[test]
    fn right_region_includes_panel_and_toolbar_when_right_panel_is_open() {
        let layout = ToolDockLayout::from_open_panels([(Panel::One, SidebarPlacement::Right)]);

        assert_eq!(
            px(420.0) + TOOLBAR_WIDTH,
            dock_region_width(&layout, px(420.0))
        );
    }

    #[test]
    fn right_sidebar_resize_excludes_toolbar_width() {
        assert_eq!(px(256.0), right_sidebar_width(px(1000.0), px(700.0)));
    }

    #[test]
    fn cloned_api_works_for_non_copy_keys() {
        use std::rc::Rc;

        let one = Rc::new("one");
        let two = Rc::new("two");
        let mut layout =
            ToolDockLayout::from_open_panels_cloned([(one.clone(), SidebarPlacement::Left)]);
        assert_eq!(layout.placement_edge(&one), Some(SidebarPlacement::Left));
        assert_eq!(layout.placement_edge(&two), None);

        // 后放入同一边的面板替换先前的（返回被替换者）。
        assert_eq!(
            layout.set_placement_cloned(two.clone(), SidebarPlacement::Left),
            Some(one.clone())
        );
        assert_eq!(layout.left, Some(two.clone()));

        // 清空占用的边。
        assert_eq!(
            layout.clear_if_occupied(SidebarPlacement::Left),
            Some(two.clone())
        );
        assert!(layout.left.is_none());
        assert!(layout.is_empty());
    }
}
