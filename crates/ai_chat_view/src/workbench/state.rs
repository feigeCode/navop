//! 工作台外壳的纯状态：面板种类、落位、会话栏折叠。
//!
//! 本模块不依赖 GPUI 渲染，落位语义用普通单元测试锁住；渲染见 [`super::shell`]。
//!
//! # 落位模型
//!
//! 四个面板有三种承载方式：
//!
//! - **中心区**：单选，同一时刻只显示一个面板（默认「对话」）。
//! - **左侧 / 底部**：各一个单槽，放需要常驻并排的内容。
//! - **右侧**：有序标签组，可同时容纳多个面板，一次显示激活的那一个。
//!
//! 三处落位共享两条不变量：
//!
//! 1. **同一面板最多出现在一个落位。** 移到别处一定从原处摘除。
//! 2. **右侧标签组不出现重复。** 重复打开只会把它激活。
//!
//! 「对话」是中心区专属（[`WorkbenchPanelKind::is_dockable`] 为假）：它不会进标签组，
//! 也不会被挤到侧边——把别的面板切到中心区只会让对话暂时让位。

use gpui::SharedString;
use one_assets::IconName;
use one_core::settings::WorkbenchLayoutSettings;
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
    /// 固定顺序，UI 上的排列以此为准（rail 与标签组下拉都按它）。
    pub const ALL: [Self; 4] = [Self::Chat, Self::Review, Self::Files, Self::Terminal];

    /// 可停靠到侧边的面板。中心区专属的「对话」不在其中。
    pub const DOCKABLE: [Self; 3] = [Self::Review, Self::Files, Self::Terminal];

    /// 持久化用的稳定 id。改名字会让已存的布局失效，不要动。
    pub fn id(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Review => "review",
            Self::Files => "files",
            Self::Terminal => "terminal",
        }
    }

    /// 从持久化 id 还原；未知 id 返回 `None`（旧配置 / 已下线的面板）。
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
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

    /// 是否可停靠到中心区以外。只有「对话」为假。
    pub fn is_dockable(self) -> bool {
        !matches!(self, Self::Chat)
    }
}

/// 面板落位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkbenchPlacement {
    Center,
    Left,
    Right,
    Bottom,
}

impl WorkbenchPlacement {
    /// 可按落位循环的顺序，用于「移到下一处」按钮。
    pub const CYCLE: [Self; 4] = [Self::Right, Self::Bottom, Self::Left, Self::Center];

    pub fn label(self) -> SharedString {
        match self {
            Self::Center => t!("Workbench.placement_center"),
            Self::Left => t!("Workbench.placement_left"),
            Self::Right => t!("Workbench.placement_right"),
            Self::Bottom => t!("Workbench.placement_bottom"),
        }
        .into()
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Center => IconName::WindowRestore,
            Self::Left => IconName::PanelLeft,
            Self::Right => IconName::PanelRight,
            Self::Bottom => IconName::PanelBottom,
        }
    }

    /// 循环到下一个落位。中心区之后回到右侧，因此按钮永远可用。
    pub fn next(self) -> Self {
        let index = Self::CYCLE
            .iter()
            .position(|placement| *placement == self)
            .unwrap_or(0);
        Self::CYCLE[(index + 1) % Self::CYCLE.len()]
    }
}

/// 会话导航栏默认宽度（像素）。
pub const DEFAULT_NAV_WIDTH: f32 = 260.0;
/// 会话导航栏允许的宽度范围。
pub const NAV_WIDTH_RANGE: (f32, f32) = (180.0, 480.0);
/// 右侧标签组默认宽度（像素）。
pub const DEFAULT_RIGHT_WIDTH: f32 = 400.0;
/// 右侧标签组允许的宽度范围。
pub const RIGHT_WIDTH_RANGE: (f32, f32) = (240.0, 720.0);

fn clamp_width(width: f32, range: (f32, f32)) -> f32 {
    width.clamp(range.0, range.1)
}

/// 工作台外壳的纯状态。
#[derive(Clone, Debug, PartialEq)]
pub struct WorkbenchState {
    nav_collapsed: bool,
    /// 中心区当前面板。`None` 只在「对话被别的面板顶掉、且用户把它关掉」的瞬间出现。
    center: Option<WorkbenchPanelKind>,
    left: Option<WorkbenchPanelKind>,
    bottom: Option<WorkbenchPanelKind>,
    /// 右侧标签组，顺序即标签顺序。
    right: Vec<WorkbenchPanelKind>,
    /// 标签组当前显示的面板。非空标签组一定有激活项。
    right_active: Option<WorkbenchPanelKind>,
    /// 会话导航栏宽度（像素），已收敛到允许范围。
    nav_width: f32,
    /// 右侧标签组宽度（像素），已收敛到允许范围。
    right_width: f32,
    /// 右侧标签组被顶栏开关收起（标签保留，仅隐藏）。
    right_collapsed: bool,
    /// 右侧标签组放大占满工作台行。
    right_maximized: bool,
}

impl WorkbenchState {
    pub fn new(center: WorkbenchPanelKind) -> Self {
        Self {
            nav_collapsed: false,
            center: Some(center),
            left: None,
            bottom: None,
            right: Vec::new(),
            right_active: None,
            nav_width: DEFAULT_NAV_WIDTH,
            right_width: DEFAULT_RIGHT_WIDTH,
            right_collapsed: false,
            right_maximized: false,
        }
    }

    pub fn nav_collapsed(&self) -> bool {
        self.nav_collapsed
    }

    /// 返回折叠后的状态，变化时才为 `true`（供调用方决定要不要重绘）。
    pub fn set_nav_collapsed(&mut self, collapsed: bool) -> bool {
        if self.nav_collapsed == collapsed {
            return false;
        }
        self.nav_collapsed = collapsed;
        true
    }

    pub fn toggle_nav(&mut self) -> bool {
        self.nav_collapsed = !self.nav_collapsed;
        self.nav_collapsed
    }

    pub fn nav_width(&self) -> f32 {
        self.nav_width
    }

    /// 设置导航栏宽度（越界收敛）。返回是否真的变化。
    pub fn set_nav_width(&mut self, width: f32) -> bool {
        let width = clamp_width(width, NAV_WIDTH_RANGE);
        if (self.nav_width - width).abs() < f32::EPSILON {
            return false;
        }
        self.nav_width = width;
        true
    }

    pub fn right_width(&self) -> f32 {
        self.right_width
    }

    /// 设置右侧标签组宽度（越界收敛）。返回是否真的变化。
    pub fn set_right_width(&mut self, width: f32) -> bool {
        let width = clamp_width(width, RIGHT_WIDTH_RANGE);
        if (self.right_width - width).abs() < f32::EPSILON {
            return false;
        }
        self.right_width = width;
        true
    }

    pub fn right_collapsed(&self) -> bool {
        self.right_collapsed
    }

    /// 收起/展开右侧标签组。放大状态下收起等于退出放大并收起。
    pub fn set_right_collapsed(&mut self, collapsed: bool) -> bool {
        if self.right_collapsed == collapsed {
            return false;
        }
        self.right_collapsed = collapsed;
        if collapsed {
            self.right_maximized = false;
        }
        true
    }

    pub fn right_maximized(&self) -> bool {
        self.right_maximized
    }

    /// 切换右侧标签组「放大占满」。放大时自动展开（不能放大一个收起的栏）。
    pub fn toggle_right_maximized(&mut self) -> bool {
        self.right_maximized = !self.right_maximized;
        if self.right_maximized {
            self.right_collapsed = false;
        }
        true
    }

    pub fn center(&self) -> Option<WorkbenchPanelKind> {
        self.center
    }

    pub fn left(&self) -> Option<WorkbenchPanelKind> {
        self.left
    }

    pub fn bottom(&self) -> Option<WorkbenchPanelKind> {
        self.bottom
    }

    pub fn right_tabs(&self) -> &[WorkbenchPanelKind] {
        &self.right
    }

    pub fn right_active(&self) -> Option<WorkbenchPanelKind> {
        self.right_active
    }

    /// 面板当前所在落位；未打开返回 `None`。
    pub fn placement_of(&self, kind: WorkbenchPanelKind) -> Option<WorkbenchPlacement> {
        if self.center == Some(kind) {
            Some(WorkbenchPlacement::Center)
        } else if self.left == Some(kind) {
            Some(WorkbenchPlacement::Left)
        } else if self.bottom == Some(kind) {
            Some(WorkbenchPlacement::Bottom)
        } else if self.right.contains(&kind) {
            Some(WorkbenchPlacement::Right)
        } else {
            None
        }
    }

    pub fn is_open(&self, kind: WorkbenchPanelKind) -> bool {
        self.placement_of(kind).is_some()
    }

    /// 把面板打开到指定落位。返回是否真的发生了变化。
    ///
    /// - 面板已在目标落位：只有右侧标签组会顺带把它设为激活项。
    /// - 「对话」请求停靠：直接拒绝（返回 `false`），它只能待在中心区。
    /// - 目标单槽已被占用：占用者按 [`Self::stash`] 处理，不会凭空消失。
    pub fn open(&mut self, kind: WorkbenchPanelKind, placement: WorkbenchPlacement) -> bool {
        if placement != WorkbenchPlacement::Center && !kind.is_dockable() {
            return false;
        }
        if self.placement_of(kind) == Some(placement) {
            return placement == WorkbenchPlacement::Right && self.set_right_active(kind);
        }

        self.detach(kind);
        match placement {
            WorkbenchPlacement::Center => {
                let displaced = self.center.replace(kind);
                self.stash(displaced);
            }
            WorkbenchPlacement::Left => {
                let displaced = self.left.replace(kind);
                self.stash(displaced);
            }
            WorkbenchPlacement::Bottom => {
                let displaced = self.bottom.replace(kind);
                self.stash(displaced);
            }
            WorkbenchPlacement::Right => {
                if !self.right.contains(&kind) {
                    self.right.push(kind);
                }
                self.right_active = Some(kind);
            }
        }
        true
    }

    /// 关闭面板。返回是否真的关掉了。
    ///
    /// 「对话」不可关闭；关掉中心区的可停靠面板后中心区回落到「对话」，
    /// 避免留下一个空白工作台。
    pub fn close(&mut self, kind: WorkbenchPanelKind) -> bool {
        if !kind.is_dockable() {
            return false;
        }
        let Some(placement) = self.placement_of(kind) else {
            return false;
        };
        self.detach(kind);
        if placement == WorkbenchPlacement::Center {
            self.center = Some(WorkbenchPanelKind::Chat);
        }
        true
    }

    /// 已打开则关闭，未打开则打开到指定落位。
    pub fn toggle(&mut self, kind: WorkbenchPanelKind, placement: WorkbenchPlacement) -> bool {
        if self.is_open(kind) {
            self.close(kind)
        } else {
            self.open(kind, placement)
        }
    }

    /// 选中右侧标签组里的某个面板。不在标签组里则无操作。
    pub fn set_right_active(&mut self, kind: WorkbenchPanelKind) -> bool {
        if !self.right.contains(&kind) || self.right_active == Some(kind) {
            return false;
        }
        self.right_active = Some(kind);
        true
    }

    /// 关掉右侧标签组当前显示的面板，返回被关掉的那一个。
    pub fn close_right_active(&mut self) -> Option<WorkbenchPanelKind> {
        let kind = self.right_active?;
        self.close(kind);
        Some(kind)
    }

    /// 把面板移到下一个落位；未打开的先打开到右侧。
    pub fn cycle_placement(&mut self, kind: WorkbenchPanelKind) -> bool {
        let Some(current) = self.placement_of(kind) else {
            return self.open(kind, WorkbenchPlacement::Right);
        };
        self.open(kind, current.next())
    }

    /// 从所有落位摘除面板，并修正标签组激活项。
    fn detach(&mut self, kind: WorkbenchPanelKind) {
        if self.center == Some(kind) {
            self.center = None;
        }
        if self.left == Some(kind) {
            self.left = None;
        }
        if self.bottom == Some(kind) {
            self.bottom = None;
        }
        let Some(index) = self.right.iter().position(|panel| *panel == kind) else {
            return;
        };
        self.right.remove(index);
        if self.right_active == Some(kind) {
            // 关掉激活标签后选相邻的：优先它右边那个（顺位补上），没有就取左边。
            self.right_active = self
                .right
                .get(index)
                .copied()
                .or_else(|| index.checked_sub(1).and_then(|prev| self.right.get(prev)).copied());
        }
    }

    /// 被单槽挤走的面板：能停靠的回到右侧标签组并激活，否则视为关闭。
    fn stash(&mut self, displaced: Option<WorkbenchPanelKind>) {
        let Some(panel) = displaced else {
            return;
        };
        if !panel.is_dockable() {
            return;
        }
        if !self.right.contains(&panel) {
            self.right.push(panel);
        }
        self.right_active = Some(panel);
    }

    /// 导出可持久化的布局。面板按稳定 id 记录。
    pub fn to_settings(&self) -> WorkbenchLayoutSettings {
        WorkbenchLayoutSettings {
            nav_collapsed: self.nav_collapsed,
            center: self.center.map(|kind| kind.id().to_string()),
            left: self.left.map(|kind| kind.id().to_string()),
            bottom: self.bottom.map(|kind| kind.id().to_string()),
            right: self
                .right
                .iter()
                .map(|kind| kind.id().to_string())
                .collect(),
            right_active: self.right_active.map(|kind| kind.id().to_string()),
            nav_width: Some(self.nav_width),
            right_width: Some(self.right_width),
            right_collapsed: self.right_collapsed,
            right_maximized: self.right_maximized,
        }
    }

    /// 从持久化布局还原。
    ///
    /// 配置文件是外部输入，这里按「能还原多少算多少」处理，绝不因为一个坏字段
    /// 就丢掉整份布局：未知 id 忽略、重复面板只保留第一次出现、不可停靠的面板
    /// 出现在侧边位一律忽略、激活项不在标签组里则回落到第一个标签。
    pub fn from_settings(settings: &WorkbenchLayoutSettings) -> Self {
        let mut state = Self::new(WorkbenchPanelKind::Chat);
        state.nav_collapsed = settings.nav_collapsed;

        if let Some(kind) = settings.center.as_deref().and_then(WorkbenchPanelKind::from_id) {
            state.center = Some(kind);
        }
        if let Some(kind) = dockable_from_settings(settings.left.as_deref()) {
            if state.placement_of(kind).is_none() {
                state.left = Some(kind);
            }
        }
        if let Some(kind) = dockable_from_settings(settings.bottom.as_deref()) {
            if state.placement_of(kind).is_none() {
                state.bottom = Some(kind);
            }
        }
        for id in &settings.right {
            let Some(kind) = dockable_from_settings(Some(id.as_str())) else {
                continue;
            };
            if state.placement_of(kind).is_none() {
                state.right.push(kind);
            }
        }
        state.right_active = settings
            .right_active
            .as_deref()
            .and_then(WorkbenchPanelKind::from_id)
            .filter(|kind| state.right.contains(kind))
            .or_else(|| state.right.first().copied());
        state.nav_width = settings
            .nav_width
            .map(|width| clamp_width(width, NAV_WIDTH_RANGE))
            .unwrap_or(DEFAULT_NAV_WIDTH);
        state.right_width = settings
            .right_width
            .map(|width| clamp_width(width, RIGHT_WIDTH_RANGE))
            .unwrap_or(DEFAULT_RIGHT_WIDTH);
        state.right_collapsed = settings.right_collapsed;
        state.right_maximized = settings.right_maximized && !state.right_collapsed;
        state
    }
}

/// 解析一个「侧边停靠位」的 id：未知 id 与不可停靠的面板都返回 `None`。
fn dockable_from_settings(id: Option<&str>) -> Option<WorkbenchPanelKind> {
    let kind = WorkbenchPanelKind::from_id(id?)?;
    kind.is_dockable().then_some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_right(state: &mut WorkbenchState, kind: WorkbenchPanelKind) {
        assert!(state.open(kind, WorkbenchPlacement::Right));
    }

    #[test]
    fn new_state_shows_chat_in_center_without_side_panels() {
        let state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert_eq!(Some(WorkbenchPanelKind::Chat), state.center());
        assert_eq!(None, state.left());
        assert_eq!(None, state.bottom());
        assert!(state.right_tabs().is_empty());
        assert_eq!(None, state.right_active());
        assert!(!state.nav_collapsed());
    }

    #[test]
    fn opening_second_panel_on_right_keeps_both_tabs_and_activates_the_newest() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert_eq!(
            &[WorkbenchPanelKind::Review, WorkbenchPanelKind::Files],
            state.right_tabs()
        );
        assert_eq!(Some(WorkbenchPanelKind::Files), state.right_active());
        assert_eq!(
            Some(WorkbenchPlacement::Right),
            state.placement_of(WorkbenchPanelKind::Review)
        );
    }

    #[test]
    fn reopening_a_right_tab_only_activates_it() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert!(state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Right));

        assert_eq!(Some(WorkbenchPanelKind::Review), state.right_active());
        assert_eq!(2, state.right_tabs().len(), "重复打开不应新增标签");
    }

    #[test]
    fn closing_active_tab_falls_back_to_the_next_one() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Files);
        open_right(&mut state, WorkbenchPanelKind::Terminal);
        // 激活的是 Terminal（最后一个），先切回中间那个再关掉。
        state.set_right_active(WorkbenchPanelKind::Files);

        assert!(state.close(WorkbenchPanelKind::Files));

        assert_eq!(
            &[WorkbenchPanelKind::Review, WorkbenchPanelKind::Terminal],
            state.right_tabs()
        );
        assert_eq!(Some(WorkbenchPanelKind::Terminal), state.right_active());
    }

    #[test]
    fn closing_last_tab_empties_the_group() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Terminal);

        assert!(state.close(WorkbenchPanelKind::Terminal));

        assert!(state.right_tabs().is_empty());
        assert_eq!(None, state.right_active());
        assert!(!state.is_open(WorkbenchPanelKind::Terminal));
    }

    #[test]
    fn closing_active_tab_at_the_end_falls_back_to_previous() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Terminal);

        // 激活项是 Terminal（队尾），右边没有标签，应回落到左边那个。
        assert!(state.close(WorkbenchPanelKind::Terminal));

        assert_eq!(Some(WorkbenchPanelKind::Review), state.right_active());
    }

    #[test]
    fn chat_cannot_be_docked() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Right));
        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Left));
        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Bottom));
        assert!(!state.close(WorkbenchPanelKind::Chat));

        assert!(state.right_tabs().is_empty());
        assert_eq!(Some(WorkbenchPanelKind::Chat), state.center());
    }

    #[test]
    fn switching_center_keeps_the_displaced_panel_alive_in_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Review);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Center));

        assert_eq!(Some(WorkbenchPanelKind::Files), state.center());
        // 被顶掉的 Review 不能凭空消失，它落进右侧标签组并激活。
        assert_eq!(&[WorkbenchPanelKind::Review], state.right_tabs());
        assert_eq!(Some(WorkbenchPanelKind::Review), state.right_active());
    }

    #[test]
    fn chatting_again_leaves_the_previous_center_panel_in_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Files);

        assert!(state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Center));

        assert_eq!(Some(WorkbenchPanelKind::Chat), state.center());
        assert_eq!(&[WorkbenchPanelKind::Files], state.right_tabs());
    }

    #[test]
    fn opening_a_right_tab_into_the_center_removes_it_from_the_group() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Center));

        assert!(state.right_tabs().is_empty());
        assert_eq!(None, state.right_active());
        assert_eq!(Some(WorkbenchPanelKind::Files), state.center());
        assert_eq!(
            Some(WorkbenchPlacement::Center),
            state.placement_of(WorkbenchPanelKind::Files)
        );
    }

    #[test]
    fn moving_a_panel_to_left_clears_its_right_tab() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Terminal);

        assert!(state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Left));

        assert_eq!(Some(WorkbenchPanelKind::Review), state.left());
        assert_eq!(&[WorkbenchPanelKind::Terminal], state.right_tabs());
        assert_eq!(Some(WorkbenchPanelKind::Terminal), state.right_active());
    }

    #[test]
    fn a_single_side_slot_displaces_its_previous_occupant_into_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Left);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left));

        assert_eq!(Some(WorkbenchPanelKind::Files), state.left());
        assert_eq!(&[WorkbenchPanelKind::Review], state.right_tabs());
        assert_eq!(Some(WorkbenchPanelKind::Review), state.right_active());
    }

    #[test]
    fn side_slots_are_independent_of_each_other() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left);
        state.open(WorkbenchPanelKind::Terminal, WorkbenchPlacement::Bottom);

        assert_eq!(Some(WorkbenchPanelKind::Files), state.left());
        assert_eq!(Some(WorkbenchPanelKind::Terminal), state.bottom());
        assert!(state.right_tabs().is_empty());
    }

    #[test]
    fn opening_a_panel_twice_at_the_same_placement_is_a_noop() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left);

        assert!(!state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left));
        assert_eq!(Some(WorkbenchPanelKind::Files), state.left());
    }

    #[test]
    fn closing_a_center_dock_panel_falls_back_to_chat() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Center);

        assert!(state.close(WorkbenchPanelKind::Review));

        assert_eq!(Some(WorkbenchPanelKind::Chat), state.center());
        assert!(!state.is_open(WorkbenchPanelKind::Review));
    }

    #[test]
    fn closing_a_panel_that_is_not_open_reports_no_change() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(!state.close(WorkbenchPanelKind::Terminal));
    }

    #[test]
    fn toggling_opens_then_closes() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.toggle(WorkbenchPanelKind::Files, WorkbenchPlacement::Right));
        assert!(state.is_open(WorkbenchPanelKind::Files));

        assert!(state.toggle(WorkbenchPanelKind::Files, WorkbenchPlacement::Right));
        assert!(!state.is_open(WorkbenchPanelKind::Files));
    }

    #[test]
    fn closing_right_active_reports_what_it_closed() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.close_right_active()
        );
        // 激活项会顺位补位，所以还能再关一个；组空之后才是 None。
        assert_eq!(Some(WorkbenchPanelKind::Review), state.close_right_active());
        assert_eq!(None, state.close_right_active());
    }

    #[test]
    fn cycling_placement_rotates_through_all_four_slots() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Right);

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(Some(WorkbenchPanelKind::Review), state.bottom());

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(Some(WorkbenchPanelKind::Review), state.left());

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(Some(WorkbenchPanelKind::Review), state.center());

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(
            Some(WorkbenchPlacement::Right),
            state.placement_of(WorkbenchPanelKind::Review)
        );
    }

    #[test]
    fn cycling_a_closed_panel_opens_it_on_the_right() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.cycle_placement(WorkbenchPanelKind::Terminal));

        assert_eq!(
            Some(WorkbenchPlacement::Right),
            state.placement_of(WorkbenchPanelKind::Terminal)
        );
    }

    #[test]
    fn setting_nav_collapsed_reports_only_real_changes() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.set_nav_collapsed(true));
        assert!(!state.set_nav_collapsed(true));
        assert!(state.nav_collapsed());
        assert!(state.set_nav_collapsed(false));
        assert!(!state.nav_collapsed());
    }

    #[test]
    fn nav_collapse_toggles_independently_of_panels() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        state.toggle_nav();
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert!(state.nav_collapsed());
        assert_eq!(Some(WorkbenchPanelKind::Files), state.right_active());
    }

    #[test]
    fn panel_ids_round_trip() {
        for kind in WorkbenchPanelKind::ALL {
            assert_eq!(Some(kind), WorkbenchPanelKind::from_id(kind.id()));
        }
        assert_eq!(None, WorkbenchPanelKind::from_id("nope"));
    }

    #[test]
    fn layout_round_trips_through_settings() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.set_nav_collapsed(true);
        state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left);
        state.open(WorkbenchPanelKind::Terminal, WorkbenchPlacement::Bottom);
        open_right(&mut state, WorkbenchPanelKind::Review);

        let restored = WorkbenchState::from_settings(&state.to_settings());

        assert_eq!(state, restored);
    }

    #[test]
    fn restoring_ignores_unknown_and_duplicate_ids() {
        let settings = WorkbenchLayoutSettings {
            nav_collapsed: false,
            center: Some("chat".into()),
            left: Some("files".into()),
            bottom: Some("unreleased-panel".into()),
            right: vec!["files".into(), "review".into(), "review".into()],
            right_active: None,
            ..Default::default()
        };

        let state = WorkbenchState::from_settings(&settings);

        assert_eq!(Some(WorkbenchPanelKind::Files), state.left());
        assert_eq!(None, state.bottom(), "未知 id 应被忽略");
        assert_eq!(
            &[WorkbenchPanelKind::Review],
            state.right_tabs(),
            "已在左侧的面板不应重复出现在标签组"
        );
        assert_eq!(Some(WorkbenchPanelKind::Review), state.right_active());
    }

    #[test]
    fn restoring_rejects_chat_in_a_side_slot() {
        let settings = WorkbenchLayoutSettings {
            center: Some("review".into()),
            left: Some("chat".into()),
            bottom: Some("chat".into()),
            right: vec!["chat".into()],
            ..Default::default()
        };

        let state = WorkbenchState::from_settings(&settings);

        assert_eq!(Some(WorkbenchPanelKind::Review), state.center());
        assert_eq!(None, state.left());
        assert_eq!(None, state.bottom());
        assert!(state.right_tabs().is_empty());
    }

    #[test]
    fn restoring_falls_back_to_chat_when_the_center_id_is_bogus() {
        let settings = WorkbenchLayoutSettings {
            center: Some("gone".into()),
            ..Default::default()
        };

        assert_eq!(
            Some(WorkbenchPanelKind::Chat),
            WorkbenchState::from_settings(&settings).center()
        );
    }

    #[test]
    fn restoring_picks_the_first_tab_when_the_active_one_is_missing() {
        let settings = WorkbenchLayoutSettings {
            right: vec!["review".into(), "files".into()],
            right_active: Some("terminal".into()),
            ..Default::default()
        };

        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            WorkbenchState::from_settings(&settings).right_active()
        );
    }

    #[test]
    fn sidebar_widths_clamp_and_persist() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.set_nav_width(50.0), "低于下限应收敛到下限");
        assert_eq!(NAV_WIDTH_RANGE.0, state.nav_width());
        assert!(state.set_right_width(10_000.0));
        assert_eq!(RIGHT_WIDTH_RANGE.1, state.right_width());
        assert!(!state.set_nav_width(state.nav_width()), "同值不报变化");

        let restored = WorkbenchState::from_settings(&state.to_settings());
        assert_eq!(state.nav_width(), restored.nav_width());
        assert_eq!(state.right_width(), restored.right_width());
    }

    #[test]
    fn restoring_widths_from_bad_or_missing_values_falls_back() {
        let mut settings = WorkbenchLayoutSettings::default();
        settings.nav_width = Some(-3.0);
        settings.right_width = Some(99_999.0);

        let state = WorkbenchState::from_settings(&settings);

        assert_eq!(NAV_WIDTH_RANGE.0, state.nav_width());
        assert_eq!(RIGHT_WIDTH_RANGE.1, state.right_width());

        let state = WorkbenchState::from_settings(&WorkbenchLayoutSettings::default());
        assert_eq!(DEFAULT_NAV_WIDTH, state.nav_width());
        assert_eq!(DEFAULT_RIGHT_WIDTH, state.right_width());
    }

    #[test]
    fn right_collapse_and_maximize_interplay() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.toggle_right_maximized());
        assert!(state.right_maximized());
        // 收起会退出放大。
        assert!(state.set_right_collapsed(true));
        assert!(!state.right_maximized());
        assert!(state.right_collapsed());

        // 放大收起中的栏会自动展开。
        assert!(state.toggle_right_maximized());
        assert!(state.right_maximized());
        assert!(!state.right_collapsed());

        let restored = WorkbenchState::from_settings(&state.to_settings());
        assert!(restored.right_maximized());
        assert!(!restored.right_collapsed());
    }

    #[test]
    fn restoring_maximized_and_collapsed_prefers_collapsed() {
        let settings = WorkbenchLayoutSettings {
            right_collapsed: true,
            right_maximized: true,
            ..Default::default()
        };

        let state = WorkbenchState::from_settings(&settings);

        assert!(state.right_collapsed());
        assert!(!state.right_maximized(), "收起优先于放大");
    }
}
