//! 工作台外壳的纯状态：面板种类、三边停靠布局、会话栏折叠。
//!
//! 本模块不依赖 GPUI 渲染，停靠语义用普通单元测试锁住；渲染见 [`super::shell`]。
//! 语义照搬 `terminal_view::sidebar::tool_dock`：**三边各最多一个面板**。

use gpui::SharedString;
use one_assets::IconName;
use one_core::dock::ToolDockLayout;
use one_core::sidebar_contribution::SidebarPlacement;
use rust_i18n::t;

/// 工作台可承载的面板种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkbenchPanelKind {
    Chat,
    Review,
    Files,
    Terminal,
}

impl WorkbenchPanelKind {
    /// 固定顺序，UI 上的排列以此为准。
    pub const ALL: [Self; 4] = [Self::Chat, Self::Review, Self::Files, Self::Terminal];

    pub fn id(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Review => "review",
            Self::Files => "files",
            Self::Terminal => "terminal",
        }
    }

    pub fn title(self) -> SharedString {
        match self {
            Self::Chat => t!("Workbench.panel_chat"),
            Self::Review => t!("Workbench.panel_review"),
            Self::Files => t!("Workbench.panel_files"),
            Self::Terminal => t!("Workbench.panel_terminal"),
        }
        .into()
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Chat => IconName::AILine,
            Self::Review => IconName::GitBranch,
            Self::Files => IconName::Folder,
            Self::Terminal => IconName::SquareTerminal,
        }
    }
}

/// 三边停靠布局：每边最多一个面板。
///
/// 模型与几何共用 [`one_core::dock`]，与终端侧栏同一套语义。
pub type WorkbenchDockLayout = ToolDockLayout<WorkbenchPanelKind>;

pub use one_core::dock::dock_region_width;

/// 工作台外壳的纯状态。
#[derive(Clone, Debug)]
pub struct WorkbenchState {
    nav_collapsed: bool,
    active: WorkbenchPanelKind,
    dock: WorkbenchDockLayout,
}

impl WorkbenchState {
    pub fn new(active: WorkbenchPanelKind) -> Self {
        Self {
            nav_collapsed: false,
            active,
            dock: WorkbenchDockLayout::default(),
        }
    }

    pub fn nav_collapsed(&self) -> bool {
        self.nav_collapsed
    }

    pub fn toggle_nav(&mut self) -> bool {
        self.nav_collapsed = !self.nav_collapsed;
        self.nav_collapsed
    }

    pub fn set_nav_collapsed(&mut self, collapsed: bool) {
        self.nav_collapsed = collapsed;
    }

    pub fn active(&self) -> WorkbenchPanelKind {
        self.active
    }

    pub fn set_active(&mut self, panel: WorkbenchPanelKind) {
        self.active = panel;
    }

    pub fn layout(&self) -> &WorkbenchDockLayout {
        &self.dock
    }

    pub fn is_panel_open(&self, panel: WorkbenchPanelKind) -> bool {
        self.dock.placement_of(panel).is_some()
    }

    pub fn placement_of(&self, panel: WorkbenchPanelKind) -> Option<SidebarPlacement> {
        self.dock.placement_of(panel)
    }

    /// 在指定边打开面板；同边已占用的面板会被替换。
    /// 返回 `true` 表示布局发生变化。
    pub fn open_panel(&mut self, panel: WorkbenchPanelKind, placement: SidebarPlacement) -> bool {
        if self.dock.placement_of(panel) == Some(placement) {
            return false;
        }
        self.close_panel_everywhere(panel);
        self.dock.set_placement(panel, placement);
        true
    }

    pub fn close_panel(&mut self, placement: SidebarPlacement) -> bool {
        self.dock.clear_placement(placement).is_some()
    }

    pub fn toggle_panel(&mut self, panel: WorkbenchPanelKind, placement: SidebarPlacement) -> bool {
        if self.dock.placement_of(panel) == Some(placement) {
            self.close_panel(placement)
        } else {
            self.open_panel(panel, placement)
        }
    }

    /// 移动面板到另一边；当前所在边会被清空。
    pub fn move_panel(&mut self, panel: WorkbenchPanelKind, placement: SidebarPlacement) -> bool {
        let Some(current) = self.dock.placement_of(panel) else {
            return self.open_panel(panel, placement);
        };
        if current == placement {
            return false;
        }
        self.close_panel(current);
        self.open_panel(panel, placement)
    }

    fn close_panel_everywhere(&mut self, panel: WorkbenchPanelKind) -> bool {
        match self.dock.placement_of(panel) {
            Some(placement) => self.dock.clear_placement(placement).is_some(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::layout::TOOLBAR_WIDTH;
    use gpui::px;

    #[test]
    fn layout_maps_open_panels_to_edges() {
        let layout = WorkbenchDockLayout::from_open_panels([
            (WorkbenchPanelKind::Files, SidebarPlacement::Left),
            (WorkbenchPanelKind::Chat, SidebarPlacement::Bottom),
            (WorkbenchPanelKind::Review, SidebarPlacement::Right),
        ]);

        assert_eq!(Some(WorkbenchPanelKind::Files), layout.left);
        assert_eq!(Some(WorkbenchPanelKind::Review), layout.right);
        assert_eq!(Some(WorkbenchPanelKind::Chat), layout.bottom);
    }

    #[test]
    fn right_region_keeps_toolbar_width_without_right_panel() {
        let layout = WorkbenchDockLayout::from_open_panels([(
            WorkbenchPanelKind::Files,
            SidebarPlacement::Left,
        )]);

        assert_eq!(TOOLBAR_WIDTH, dock_region_width(&layout, px(400.0)));
    }

    #[test]
    fn right_region_includes_panel_and_toolbar_when_right_panel_is_open() {
        let layout = WorkbenchDockLayout::from_open_panels([(
            WorkbenchPanelKind::Review,
            SidebarPlacement::Right,
        )]);

        assert_eq!(
            px(400.0) + TOOLBAR_WIDTH,
            dock_region_width(&layout, px(400.0))
        );
    }

    #[test]
    fn opening_second_panel_on_same_edge_replaces_the_first() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.open_panel(WorkbenchPanelKind::Files, SidebarPlacement::Right));
        assert!(state.open_panel(WorkbenchPanelKind::Review, SidebarPlacement::Right));

        assert_eq!(Some(WorkbenchPanelKind::Review), state.layout().right);
        assert!(!state.is_panel_open(WorkbenchPanelKind::Files));
    }

    #[test]
    fn moving_panel_clears_previous_edge() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_panel(WorkbenchPanelKind::Review, SidebarPlacement::Right);

        assert!(state.move_panel(WorkbenchPanelKind::Review, SidebarPlacement::Bottom));

        assert_eq!(None, state.layout().right);
        assert_eq!(Some(WorkbenchPanelKind::Review), state.layout().bottom);
        assert_eq!(
            Some(SidebarPlacement::Bottom),
            state.placement_of(WorkbenchPanelKind::Review)
        );
    }

    #[test]
    fn moving_panel_to_current_edge_is_a_noop() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_panel(WorkbenchPanelKind::Review, SidebarPlacement::Right);

        assert!(!state.move_panel(WorkbenchPanelKind::Review, SidebarPlacement::Right));
        assert_eq!(Some(WorkbenchPanelKind::Review), state.layout().right);
    }

    #[test]
    fn toggling_open_panel_closes_it() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_panel(WorkbenchPanelKind::Files, SidebarPlacement::Right);

        assert!(state.toggle_panel(WorkbenchPanelKind::Files, SidebarPlacement::Right));

        assert!(!state.is_panel_open(WorkbenchPanelKind::Files));
        assert!(state.layout().is_empty());
    }

    #[test]
    fn opening_panel_twice_on_same_edge_keeps_layout_unchanged() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_panel(WorkbenchPanelKind::Terminal, SidebarPlacement::Bottom);

        assert!(!state.open_panel(WorkbenchPanelKind::Terminal, SidebarPlacement::Bottom));
    }

    #[test]
    fn nav_collapse_toggles_independently_of_active_panel() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(!state.nav_collapsed());
        assert!(state.toggle_nav());
        assert!(state.nav_collapsed());
        assert_eq!(WorkbenchPanelKind::Chat, state.active());
        assert!(!state.toggle_nav());
        assert!(!state.nav_collapsed());
    }
}
