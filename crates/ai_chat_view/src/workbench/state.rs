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
//! 1. **同一实例最多出现在一个落位。** 移到别处一定从原处摘除。
//! 2. **单例面板在标签组不重复**（重复打开只会激活）；多例面板（终端等）
//!    可通过「新建页签」并存多个实例。
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
    /// 子代理详情：回放某只子代理的完整推理过程。
    ///
    /// 由「点开子代理卡片」驱动，因此**不落盘**（见 [`WorkbenchState::to_settings`]）：
    /// 重启后没有任何卡片指向它，恢复出来的只会是一个空面板。
    Subagent,
}

impl WorkbenchPanelKind {
    /// 固定顺序，UI 上的排列以此为准（rail 与标签组下拉都按它）。
    pub const ALL: [Self; 5] = [
        Self::Chat,
        Self::Review,
        Self::Files,
        Self::Terminal,
        Self::Subagent,
    ];

    /// 可停靠到侧边的面板。中心区专属的「对话」不在其中。
    pub const DOCKABLE: [Self; 4] = [Self::Review, Self::Files, Self::Terminal, Self::Subagent];

    /// 持久化用的稳定 id。改名字会让已存的布局失效，不要动。
    pub fn id(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Review => "review",
            Self::Files => "files",
            Self::Terminal => "terminal",
            Self::Subagent => "subagent",
        }
    }

    /// 是否参与布局持久化。
    ///
    /// 子代理详情是**会话内的一次性视图**：它由某张卡片的点击开出来，重启后没有任何
    /// 东西指向它。存下来只会在下次启动时凭空恢复出一个空面板。
    pub fn persists_layout(self) -> bool {
        !matches!(self, Self::Subagent)
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
            Self::Subagent => t!("Workbench.panel_subagent"),
        }
        .into()
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Chat => IconName::AILine,
            Self::Review => IconName::GitBranch,
            Self::Files => IconName::Folder,
            Self::Terminal => IconName::SquareTerminal,
            Self::Subagent => IconName::Bot,
        }
    }

    /// 是否可停靠到中心区以外。只有「对话」为假。
    pub fn is_dockable(self) -> bool {
        !matches!(self, Self::Chat)
    }

    /// 是否支持多开：`true` 的面板（终端等）每次「新建页签」都开一个新
    /// 实例，同一面板可并存多个标签；`false` 的面板（审查、文件）重复
    /// 打开只会定位到已开的那个标签。
    pub fn multi_instance(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

/// 右侧标签组里的一个标签实例。
///
/// 单例面板（[`WorkbenchPanelKind::multi_instance`] 为假）恒用 `seq = 0`，
/// 重复打开只会激活同一个标签；多例面板每次「新建页签」分配递增的 `seq`，
/// 同一面板可并存多个实例。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorkbenchTab {
    pub kind: WorkbenchPanelKind,
    seq: u32,
}

impl WorkbenchTab {
    /// 单例面板的标签身份（`seq = 0`）。
    pub fn new(kind: WorkbenchPanelKind) -> Self {
        Self { kind, seq: 0 }
    }

    fn with_seq(kind: WorkbenchPanelKind, seq: u32) -> Self {
        Self { kind, seq }
    }

    /// 实例序号；单例面板恒为 0。
    pub fn seq(self) -> u32 {
        self.seq
    }

    /// 显示名：多例面板的第 N 个实例（N > 1）带序号（如「终端 2」），
    /// 其余用面板原名。`ordinal` 为该实例在同 kind 标签中的下标（0 起）。
    pub fn display_title(self, ordinal: usize) -> SharedString {
        if self.kind.multi_instance() && ordinal > 0 {
            t!(
                "Workbench.tab_instance",
                panel = self.kind.title(),
                ordinal = ordinal + 1
            )
            .to_string()
            .into()
        } else {
            self.kind.title()
        }
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
    center: Option<WorkbenchTab>,
    left: Option<WorkbenchTab>,
    bottom: Option<WorkbenchTab>,
    /// 右侧标签组，顺序即标签顺序。多例面板可有多个实例标签。
    right: Vec<WorkbenchTab>,
    /// 标签组当前显示的面板。非空标签组一定有激活项。
    right_active: Option<WorkbenchTab>,
    /// 会话导航栏宽度（像素），已收敛到允许范围。
    nav_width: f32,
    /// 右侧标签组宽度（像素），已收敛到允许范围。
    right_width: f32,
    /// 右侧标签组被顶栏开关收起（标签保留，仅隐藏）。
    right_collapsed: bool,
    /// 右侧标签组放大占满工作台行。
    right_maximized: bool,
    /// 侧栏里被用户「移除」的工作区根目录。会话归属工作区是持久事实，
    /// 分组由它派生，所以隐藏必须记名单（落盘），否则下次渲染就回来。
    hidden_workspaces: Vec<String>,
}

impl WorkbenchState {
    pub fn new(center: WorkbenchPanelKind) -> Self {
        Self {
            nav_collapsed: false,
            center: Some(WorkbenchTab::new(center)),
            left: None,
            bottom: None,
            right: Vec::new(),
            right_active: None,
            nav_width: DEFAULT_NAV_WIDTH,
            right_width: DEFAULT_RIGHT_WIDTH,
            right_collapsed: false,
            right_maximized: false,
            hidden_workspaces: Vec::new(),
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

    /// 右侧标签组是否处于「展开」态：有标签且未被收起，放大视为展开。
    /// 工具条注入外壳的开关图标与外壳自身的切换逻辑共用这一个判据，
    /// 避免两处口径漂移。
    pub fn right_sidebar_open(&self) -> bool {
        self.right_maximized() || (!self.right_collapsed() && !self.right_tabs().is_empty())
    }

    /// 切换右侧标签组「放大占满」。放大时自动展开（不能放大一个收起的栏）。
    pub fn toggle_right_maximized(&mut self) -> bool {
        self.right_maximized = !self.right_maximized;
        if self.right_maximized {
            self.right_collapsed = false;
        }
        true
    }

    /// 该工作区是否被用户从侧栏移除过。
    pub fn is_workspace_hidden(&self, root: &str) -> bool {
        self.hidden_workspaces.iter().any(|hidden| hidden == root)
    }

    /// 把工作区从侧栏移除（记名单）。已在名单里则无变化。
    pub fn hide_workspace(&mut self, root: &str) -> bool {
        if self.is_workspace_hidden(root) {
            return false;
        }
        self.hidden_workspaces.push(root.to_string());
        true
    }

    /// 恢复工作区分组（用户显式切回该工作区时调用）。
    pub fn unhide_workspace(&mut self, root: &str) -> bool {
        let Some(index) = self
            .hidden_workspaces
            .iter()
            .position(|hidden| hidden == root)
        else {
            return false;
        };
        self.hidden_workspaces.remove(index);
        true
    }

    pub fn center(&self) -> Option<WorkbenchTab> {
        self.center
    }

    pub fn left(&self) -> Option<WorkbenchTab> {
        self.left
    }

    pub fn bottom(&self) -> Option<WorkbenchTab> {
        self.bottom
    }

    pub fn right_tabs(&self) -> &[WorkbenchTab] {
        &self.right
    }

    pub fn right_active(&self) -> Option<WorkbenchTab> {
        self.right_active
    }

    /// 面板当前所在落位（取它的第一个实例）；未打开返回 `None`。
    pub fn placement_of(&self, kind: WorkbenchPanelKind) -> Option<WorkbenchPlacement> {
        if self.center.map(|tab| tab.kind) == Some(kind) {
            Some(WorkbenchPlacement::Center)
        } else if self.left.map(|tab| tab.kind) == Some(kind) {
            Some(WorkbenchPlacement::Left)
        } else if self.bottom.map(|tab| tab.kind) == Some(kind) {
            Some(WorkbenchPlacement::Bottom)
        } else if self.right.iter().any(|tab| tab.kind == kind) {
            Some(WorkbenchPlacement::Right)
        } else {
            None
        }
    }

    pub fn is_open(&self, kind: WorkbenchPanelKind) -> bool {
        self.placement_of(kind).is_some()
    }

    /// 把面板打开到指定落位（定位语义）。返回是否真的发生了变化。
    ///
    /// - 面板已在目标落位：只有右侧标签组会顺带把它设为激活项。
    /// - 「对话」请求停靠：直接拒绝（返回 `false`），它只能待在中心区。
    /// - 目标单槽已被占用：占用者按 [`Self::stash`] 处理，不会凭空消失。
    ///
    /// 多例面板（终端等）走定位语义时也只定位到第一个实例；
    /// 「新建页签」请用 [`Self::open_new_tab`]。
    pub fn open(&mut self, kind: WorkbenchPanelKind, placement: WorkbenchPlacement) -> bool {
        if placement != WorkbenchPlacement::Center && !kind.is_dockable() {
            return false;
        }
        if self.placement_of(kind) == Some(placement) {
            return placement == WorkbenchPlacement::Right && self.activate_first_tab(kind);
        }

        let tab = self.take_tab(kind);
        match placement {
            WorkbenchPlacement::Center => {
                let displaced = self.center.replace(tab);
                self.stash(displaced);
            }
            WorkbenchPlacement::Left => {
                let displaced = self.left.replace(tab);
                self.stash(displaced);
            }
            WorkbenchPlacement::Bottom => {
                let displaced = self.bottom.replace(tab);
                self.stash(displaced);
            }
            WorkbenchPlacement::Right => {
                self.right.push(tab);
                self.right_active = Some(tab);
            }
        }
        true
    }

    /// 多例面板「新建页签」：无论是否已打开，都向右侧标签组追加一个新
    /// 实例并激活。单例面板退化为定位语义。返回新标签身份；
    /// 面板不可停靠时返回 `None`。
    pub fn open_new_tab(&mut self, kind: WorkbenchPanelKind) -> Option<WorkbenchTab> {
        if !kind.is_dockable() {
            return None;
        }
        if !kind.multi_instance() {
            self.open(kind, WorkbenchPlacement::Right);
            return self.first_tab_of(kind);
        }
        let tab = self.fresh_tab(kind);
        self.right.push(tab);
        self.right_active = Some(tab);
        Some(tab)
    }

    /// 关闭面板的第一个实例。返回是否真的关掉了。
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
        self.take_tab(kind);
        if placement == WorkbenchPlacement::Center {
            self.center = Some(WorkbenchTab::new(WorkbenchPanelKind::Chat));
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

    /// 选中右侧标签组里的某个面板（定位到它的第一个实例）。不在标签组里则无操作。
    pub fn set_right_active(&mut self, kind: WorkbenchPanelKind) -> bool {
        self.activate_first_tab(kind)
    }

    /// 选中右侧标签组里的某个具体实例。
    pub fn set_right_active_tab(&mut self, tab: WorkbenchTab) -> bool {
        if !self.right.contains(&tab) || self.right_active == Some(tab) {
            return false;
        }
        self.right_active = Some(tab);
        true
    }

    /// 关掉右侧标签组当前显示的实例，返回被关掉的那一个。
    pub fn close_right_active(&mut self) -> Option<WorkbenchTab> {
        let tab = self.right_active?;
        self.close_tab(tab);
        Some(tab)
    }

    /// 把面板移到下一个落位；未打开的先打开到右侧。
    pub fn cycle_placement(&mut self, kind: WorkbenchPanelKind) -> bool {
        let Some(current) = self.placement_of(kind) else {
            return self.open(kind, WorkbenchPlacement::Right);
        };
        self.open(kind, current.next())
    }

    /// 该面板在标签组里的第一个实例标签。
    fn first_tab_of(&self, kind: WorkbenchPanelKind) -> Option<WorkbenchTab> {
        self.right.iter().copied().find(|tab| tab.kind == kind)
    }

    /// 定位语义：激活面板已有的第一个标签（若有），返回是否变化。
    fn activate_first_tab(&mut self, kind: WorkbenchPanelKind) -> bool {
        match self.first_tab_of(kind) {
            Some(tab) => self.set_right_active_tab(tab),
            None => false,
        }
    }

    /// 为面板分配一个新标签身份：多例面板用递增 `seq`，单例恒为 0。
    fn fresh_tab(&self, kind: WorkbenchPanelKind) -> WorkbenchTab {
        let seq = if kind.multi_instance() {
            self.right
                .iter()
                .filter(|tab| tab.kind == kind)
                .map(|tab| tab.seq + 1)
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        WorkbenchTab::with_seq(kind, seq)
    }

    /// 把面板的第一个实例从所有落位摘下，返回它的标签身份；
    /// 面板原本未打开则分配一个新身份。右侧组里的后继实例会顺位补激活。
    fn take_tab(&mut self, kind: WorkbenchPanelKind) -> WorkbenchTab {
        if let Some(tab) = self.center.filter(|tab| tab.kind == kind) {
            self.center = None;
            return tab;
        }
        if let Some(tab) = self.left.filter(|tab| tab.kind == kind) {
            self.left = None;
            return tab;
        }
        if let Some(tab) = self.bottom.filter(|tab| tab.kind == kind) {
            self.bottom = None;
            return tab;
        }
        match self.first_tab_of(kind) {
            Some(tab) => {
                self.close_tab(tab);
                tab
            }
            None => self.fresh_tab(kind),
        }
    }

    /// 关闭右侧标签组里的一个具体实例。返回是否真的关掉了。
    pub fn close_tab(&mut self, tab: WorkbenchTab) -> bool {
        let Some(index) = self.right.iter().position(|open| *open == tab) else {
            return false;
        };
        let removed = self.right.remove(index);
        if self.right_active == Some(removed) {
            // 关掉激活标签后选相邻的：优先它右边那个（顺位补上），没有就取左边。
            self.right_active = self.right.get(index).copied().or_else(|| {
                index
                    .checked_sub(1)
                    .and_then(|prev| self.right.get(prev))
                    .copied()
            });
        }
        true
    }

    /// 被单槽挤走的面板实例：能停靠的回到右侧标签组并激活，否则视为关闭。
    fn stash(&mut self, displaced: Option<WorkbenchTab>) {
        let Some(tab) = displaced else {
            return;
        };
        if !tab.kind.is_dockable() {
            return;
        }
        if !self.right.contains(&tab) {
            self.right.push(tab);
        }
        self.right_active = Some(tab);
    }

    /// 导出可持久化的布局。面板按稳定 id 记录。
    ///
    /// 多例面板的多个实例只落盘第一个（终端会话本就活不过进程，恢复时
    /// 只还原「这个面板开着」这一事实）；激活项若是非首个实例则回落到
    /// 该面板的第一个实例。
    ///
    /// 另外只落盘 [`WorkbenchPanelKind::persists_layout`] 为真的面板：子代理详情由
    /// 卡片点击开出来，存下来只会在下次启动时恢复出一个没有任何内容的空面板。
    pub fn to_settings(&self) -> WorkbenchLayoutSettings {
        let persisted = |tab: &WorkbenchTab| tab.kind.persists_layout();
        WorkbenchLayoutSettings {
            nav_collapsed: self.nav_collapsed,
            center: self
                .center
                .filter(persisted)
                .map(|tab| tab.kind.id().to_string()),
            left: self
                .left
                .filter(persisted)
                .map(|tab| tab.kind.id().to_string()),
            bottom: self
                .bottom
                .filter(persisted)
                .map(|tab| tab.kind.id().to_string()),
            right: {
                let mut seen = std::collections::HashSet::new();
                self.right
                    .iter()
                    .filter(|tab| tab.kind.persists_layout() && seen.insert(tab.kind.id()))
                    .map(|tab| tab.kind.id().to_string())
                    .collect()
            },
            right_active: self
                .right_active
                .filter(|tab| tab.kind.persists_layout())
                .filter(|tab| self.first_tab_of(tab.kind) == Some(*tab))
                .map(|tab| tab.kind.id().to_string()),
            nav_width: Some(self.nav_width),
            right_width: Some(self.right_width),
            right_collapsed: self.right_collapsed,
            right_maximized: self.right_maximized,
            hidden_workspaces: self.hidden_workspaces.clone(),
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

        // 中心区同样要过一遍持久化守卫：配置文件是外部输入，手改过的文件不该能把
        // 一个「没有内容可显示」的面板摆到中心。
        if let Some(kind) = settings
            .center
            .as_deref()
            .and_then(WorkbenchPanelKind::from_id)
            .filter(|kind| kind.persists_layout())
        {
            state.center = Some(WorkbenchTab::new(kind));
        }
        if let Some(kind) = dockable_from_settings(settings.left.as_deref()) {
            if state.placement_of(kind).is_none() {
                state.left = Some(WorkbenchTab::new(kind));
            }
        }
        if let Some(kind) = dockable_from_settings(settings.bottom.as_deref()) {
            if state.placement_of(kind).is_none() {
                state.bottom = Some(WorkbenchTab::new(kind));
            }
        }
        for id in &settings.right {
            let Some(kind) = dockable_from_settings(Some(id.as_str())) else {
                continue;
            };
            if state.placement_of(kind).is_none() {
                state.right.push(WorkbenchTab::new(kind));
            }
        }
        state.right_active = settings
            .right_active
            .as_deref()
            .and_then(WorkbenchPanelKind::from_id)
            .and_then(|kind| state.first_tab_of(kind))
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
        state.hidden_workspaces = settings.hidden_workspaces.clone();
        state
    }
}

/// 解析一个「侧边停靠位」的 id：未知 id、不可停靠、不参与持久化的面板都返回 `None`。
///
/// 不参与持久化这一条是双保险：正常路径上 [`WorkbenchState::to_settings`] 就不会写出
/// 这类面板，但配置文件是外部输入，手改过的旧文件不该能把一个空面板恢复出来。
fn dockable_from_settings(id: Option<&str>) -> Option<WorkbenchPanelKind> {
    let kind = WorkbenchPanelKind::from_id(id?)?;
    (kind.is_dockable() && kind.persists_layout()).then_some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_right(state: &mut WorkbenchState, kind: WorkbenchPanelKind) {
        assert!(state.open(kind, WorkbenchPlacement::Right));
    }

    fn tab(kind: WorkbenchPanelKind) -> WorkbenchTab {
        WorkbenchTab::new(kind)
    }

    #[test]
    fn new_state_shows_chat_in_center_without_side_panels() {
        let state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert_eq!(
            Some(WorkbenchPanelKind::Chat),
            state.center().map(|t| t.kind)
        );
        assert_eq!(None, state.left());
        assert_eq!(None, state.bottom());
        assert!(state.right_tabs().is_empty());
        assert_eq!(None, state.right_active());
        assert!(!state.nav_collapsed());
    }

    #[test]
    fn hidden_workspaces_round_trip_and_deduplicate() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(state.hide_workspace("/tmp/a"));
        assert!(!state.hide_workspace("/tmp/a"), "重复移除不应产生重复名单");
        assert!(state.is_workspace_hidden("/tmp/a"));
        assert!(!state.is_workspace_hidden("/tmp/b"));

        let settings = state.to_settings();
        let mut restored = WorkbenchState::from_settings(&settings);
        assert!(restored.is_workspace_hidden("/tmp/a"));

        assert!(restored.unhide_workspace("/tmp/a"));
        assert!(!restored.unhide_workspace("/tmp/a"), "名单里没有时无变化");
        assert!(!restored.is_workspace_hidden("/tmp/a"));
    }

    #[test]
    fn right_sidebar_open_tracks_tabs_collapse_and_maximize() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        assert!(!state.right_sidebar_open(), "无标签时右侧是关的");

        open_right(&mut state, WorkbenchPanelKind::Files);
        assert!(state.right_sidebar_open(), "有标签且未收起即视为展开");

        assert!(state.set_right_collapsed(true));
        assert!(!state.right_sidebar_open(), "收起后视为关闭");

        assert!(state.toggle_right_maximized());
        assert!(
            state.right_sidebar_open(),
            "放大视为展开（放大自动解除收起）"
        );

        assert!(state.set_right_collapsed(true));
        assert!(
            !state.right_sidebar_open() && !state.right_maximized(),
            "收起优先级高于放大"
        );
    }

    #[test]
    fn opening_second_panel_on_right_keeps_both_tabs_and_activates_the_newest() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        open_right(&mut state, WorkbenchPanelKind::Review);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert_eq!(
            &[
                tab(WorkbenchPanelKind::Review),
                tab(WorkbenchPanelKind::Files)
            ],
            state.right_tabs()
        );
        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.right_active().map(|t| t.kind)
        );
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

        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.right_active().map(|t| t.kind)
        );
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
            &[
                tab(WorkbenchPanelKind::Review),
                tab(WorkbenchPanelKind::Terminal)
            ],
            state.right_tabs()
        );
        assert_eq!(
            Some(WorkbenchPanelKind::Terminal),
            state.right_active().map(|t| t.kind)
        );
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

        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn chat_cannot_be_docked() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Right));
        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Left));
        assert!(!state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Bottom));
        assert!(!state.close(WorkbenchPanelKind::Chat));

        assert!(state.right_tabs().is_empty());
        assert_eq!(
            Some(WorkbenchPanelKind::Chat),
            state.center().map(|t| t.kind)
        );
    }

    #[test]
    fn switching_center_keeps_the_displaced_panel_alive_in_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Review);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Center));

        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.center().map(|t| t.kind)
        );
        // 被顶掉的 Review 不能凭空消失，它落进右侧标签组并激活。
        assert_eq!(&[tab(WorkbenchPanelKind::Review)], state.right_tabs());
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn chatting_again_leaves_the_previous_center_panel_in_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Files);

        assert!(state.open(WorkbenchPanelKind::Chat, WorkbenchPlacement::Center));

        assert_eq!(
            Some(WorkbenchPanelKind::Chat),
            state.center().map(|t| t.kind)
        );
        assert_eq!(&[tab(WorkbenchPanelKind::Files)], state.right_tabs());
    }

    #[test]
    fn opening_a_right_tab_into_the_center_removes_it_from_the_group() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        open_right(&mut state, WorkbenchPanelKind::Files);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Center));

        assert!(state.right_tabs().is_empty());
        assert_eq!(None, state.right_active());
        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.center().map(|t| t.kind)
        );
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

        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.left().map(|t| t.kind)
        );
        assert_eq!(&[tab(WorkbenchPanelKind::Terminal)], state.right_tabs());
        assert_eq!(
            Some(WorkbenchPanelKind::Terminal),
            state.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn a_single_side_slot_displaces_its_previous_occupant_into_right_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Left);

        assert!(state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left));

        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.left().map(|t| t.kind)
        );
        assert_eq!(&[tab(WorkbenchPanelKind::Review)], state.right_tabs());
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn side_slots_are_independent_of_each_other() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left);
        state.open(WorkbenchPanelKind::Terminal, WorkbenchPlacement::Bottom);

        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.left().map(|t| t.kind)
        );
        assert_eq!(
            Some(WorkbenchPanelKind::Terminal),
            state.bottom().map(|t| t.kind)
        );
        assert!(state.right_tabs().is_empty());
    }

    #[test]
    fn opening_a_panel_twice_at_the_same_placement_is_a_noop() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left);

        assert!(!state.open(WorkbenchPanelKind::Files, WorkbenchPlacement::Left));
        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.left().map(|t| t.kind)
        );
    }

    #[test]
    fn closing_a_center_dock_panel_falls_back_to_chat() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Center);

        assert!(state.close(WorkbenchPanelKind::Review));

        assert_eq!(
            Some(WorkbenchPanelKind::Chat),
            state.center().map(|t| t.kind)
        );
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
            state.close_right_active().map(|t| t.kind)
        );
        // 激活项会顺位补位，所以还能再关一个；组空之后才是 None。
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.close_right_active().map(|t| t.kind)
        );
        assert_eq!(None, state.close_right_active());
    }

    #[test]
    fn cycling_placement_rotates_through_all_four_slots() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open(WorkbenchPanelKind::Review, WorkbenchPlacement::Right);

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.bottom().map(|t| t.kind)
        );

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.left().map(|t| t.kind)
        );

        assert!(state.cycle_placement(WorkbenchPanelKind::Review));
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.center().map(|t| t.kind)
        );

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
        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn panel_ids_round_trip() {
        for kind in WorkbenchPanelKind::ALL {
            assert_eq!(Some(kind), WorkbenchPanelKind::from_id(kind.id()));
        }
        assert_eq!(None, WorkbenchPanelKind::from_id("nope"));
    }

    #[test]
    fn only_terminal_is_multi_instance() {
        assert!(!WorkbenchPanelKind::Chat.multi_instance());
        assert!(!WorkbenchPanelKind::Review.multi_instance());
        assert!(!WorkbenchPanelKind::Files.multi_instance());
        assert!(WorkbenchPanelKind::Terminal.multi_instance());
    }

    #[test]
    fn new_tab_appends_a_fresh_terminal_instance_each_time() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        let first = state.open_new_tab(WorkbenchPanelKind::Terminal);
        let second = state.open_new_tab(WorkbenchPanelKind::Terminal);

        assert_eq!(
            Some(WorkbenchTab::with_seq(WorkbenchPanelKind::Terminal, 0)),
            first
        );
        assert_eq!(
            Some(WorkbenchTab::with_seq(WorkbenchPanelKind::Terminal, 1)),
            second
        );
        assert_eq!(2, state.right_tabs().len(), "多例面板每次都是新标签");
        assert_eq!(second, state.right_active(), "新标签自动激活");
    }

    #[test]
    fn new_tab_on_a_singleton_panel_locates_instead_of_duplicating() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);

        let first = state.open_new_tab(WorkbenchPanelKind::Files);
        let second = state.open_new_tab(WorkbenchPanelKind::Files);

        assert_eq!(first, second, "单例面板重复点选只定位");
        assert_eq!(1, state.right_tabs().len());
    }

    #[test]
    fn locate_semantics_pick_the_first_terminal_instance() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_new_tab(WorkbenchPanelKind::Terminal);
        state.open_new_tab(WorkbenchPanelKind::Terminal);
        state.set_right_active_tab(state.right_tabs()[0]);

        // 定位语义（rail / 拖拽等既有路径）只激活第一个实例，不再开新的；
        // 已是激活项时无变化，返回 false。
        assert!(!state.open(WorkbenchPanelKind::Terminal, WorkbenchPlacement::Right));
        assert_eq!(2, state.right_tabs().len());
        assert_eq!(
            Some(WorkbenchTab::with_seq(WorkbenchPanelKind::Terminal, 0)),
            state.right_active()
        );
    }

    #[test]
    fn closing_one_terminal_instance_keeps_the_other() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_new_tab(WorkbenchPanelKind::Terminal);
        let second = state
            .open_new_tab(WorkbenchPanelKind::Terminal)
            .expect("终端可多开");

        assert!(state.close_tab(second));

        assert_eq!(1, state.right_tabs().len());
        assert_eq!(
            Some(WorkbenchTab::with_seq(WorkbenchPanelKind::Terminal, 0)),
            state.right_active()
        );
        // 关掉第一个后，再「新建页签」从 0 重新计号不冲突（唯一活跃实例）。
        let reopened = state.open_new_tab(WorkbenchPanelKind::Terminal);
        assert_eq!(
            Some(WorkbenchTab::with_seq(WorkbenchPanelKind::Terminal, 1)),
            reopened
        );
    }

    #[test]
    fn settings_round_trip_dedupes_multi_instance_tabs() {
        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        state.open_new_tab(WorkbenchPanelKind::Terminal);
        state.open_new_tab(WorkbenchPanelKind::Terminal);
        state.open_new_tab(WorkbenchPanelKind::Review);

        let restored = WorkbenchState::from_settings(&state.to_settings());

        // 多实例只还原「面板开着」：终端与审查各一个标签。
        let kinds: Vec<WorkbenchPanelKind> = restored.right_tabs().iter().map(|t| t.kind).collect();
        assert_eq!(
            &[WorkbenchPanelKind::Terminal, WorkbenchPanelKind::Review],
            kinds.as_slice()
        );
        // 激活项是审查（最后打开的单例，首个实例可落盘）。
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            restored.right_active().map(|t| t.kind)
        );
    }

    #[test]
    fn tab_display_title_only_numbers_second_and_later_instances() {
        let terminal = WorkbenchPanelKind::Terminal;
        assert_eq!(
            terminal.title(),
            WorkbenchTab::with_seq(terminal, 0).display_title(0)
        );
        assert_ne!(
            terminal.title(),
            WorkbenchTab::with_seq(terminal, 1).display_title(1),
            "第二个实例应带序号"
        );
        // 单例面板永远不带序号。
        assert_eq!(
            WorkbenchPanelKind::Files.title(),
            WorkbenchTab::new(WorkbenchPanelKind::Files).display_title(1)
        );
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

        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.left().map(|t| t.kind)
        );
        assert_eq!(None, state.bottom(), "未知 id 应被忽略");
        assert_eq!(
            &[tab(WorkbenchPanelKind::Review)],
            state.right_tabs(),
            "已在左侧的面板不应重复出现在标签组"
        );
        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.right_active().map(|t| t.kind)
        );
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

        assert_eq!(
            Some(WorkbenchPanelKind::Review),
            state.center().map(|t| t.kind)
        );
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
            WorkbenchState::from_settings(&settings)
                .center()
                .map(|t| t.kind)
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
            WorkbenchState::from_settings(&settings)
                .right_active()
                .map(|t| t.kind)
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

    /// 子代理详情面板是「点开卡片才出现」的临时视图：可以停靠、但绝不落盘。
    ///
    /// 它显示的是某一次子代理调用的回放，重启后没有任何卡片指向它。落盘只会让下次
    /// 启动凭空多出一个永远空着的右栏标签。
    #[test]
    fn the_subagent_panel_opens_like_any_other_but_is_never_persisted() {
        assert!(WorkbenchPanelKind::Subagent.is_dockable());
        assert!(!WorkbenchPanelKind::Subagent.persists_layout());

        let mut state = WorkbenchState::new(WorkbenchPanelKind::Chat);
        assert!(state.open(WorkbenchPanelKind::Subagent, WorkbenchPlacement::Right));
        assert!(state.is_open(WorkbenchPanelKind::Subagent));
        assert_eq!(
            Some(WorkbenchPanelKind::Subagent),
            state.right_active().map(|tab| tab.kind)
        );

        let settings = state.to_settings();
        assert!(
            settings.right.is_empty(),
            "临时面板不落盘，实际写出：{:?}",
            settings.right
        );
        assert_eq!(None, settings.right_active);

        // 同一份状态里别的面板该存还得存：过滤不能误伤。
        open_right(&mut state, WorkbenchPanelKind::Files);
        let settings = state.to_settings();
        assert_eq!(vec!["files".to_string()], settings.right);
        assert_eq!(Some("files".to_string()), settings.right_active);
    }

    /// 配置文件是外部输入：手改过的旧文件也不该把空的详情面板恢复出来。
    #[test]
    fn a_hand_edited_layout_cannot_restore_the_subagent_panel() {
        for slot in ["center", "left", "bottom"] {
            let mut settings = WorkbenchLayoutSettings::default();
            match slot {
                "center" => settings.center = Some("subagent".into()),
                "left" => settings.left = Some("subagent".into()),
                _ => settings.bottom = Some("subagent".into()),
            }

            let state = WorkbenchState::from_settings(&settings);

            assert!(
                !state.is_open(WorkbenchPanelKind::Subagent),
                "{slot} 位上的 subagent 不该被还原"
            );
        }

        let settings = WorkbenchLayoutSettings {
            right: vec!["subagent".into(), "files".into()],
            right_active: Some("subagent".into()),
            ..Default::default()
        };

        let state = WorkbenchState::from_settings(&settings);

        assert_eq!(
            vec![WorkbenchPanelKind::Files],
            state
                .right_tabs()
                .iter()
                .map(|tab| tab.kind)
                .collect::<Vec<_>>(),
            "右侧组里的 subagent 应被丢掉，其余面板照常还原"
        );
        assert_eq!(
            Some(WorkbenchPanelKind::Files),
            state.right_active().map(|tab| tab.kind),
            "激活项指向被丢掉的面板时要回落到第一个标签"
        );
    }
}
