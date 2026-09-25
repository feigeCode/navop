//! 工作台外壳：会话导航 + 中心区 + 左右底三处停靠 + 工具条。
//!
//! 外壳不持有业务会话：面板以 [`WorkbenchPanelEntry`] 注入，外壳只负责切换、
//! 停靠、折叠与布局落盘；面板之间不互相持有。落位语义在 [`super::state`]，
//! 本模块只做渲染与状态编排。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{AnyView, AppContext as _, Context, Entity, FocusHandle, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};
use one_core::settings::AppSettings;

use super::state::{WorkbenchPanelKind, WorkbenchPlacement, WorkbenchState, WorkbenchTab};
use crate::DefaultAgentChatPanel;
use crate::theme::AgentChatTheme;

mod impls;
mod nav;
mod widgets;

/// 侧边停靠面板的默认宽度。
const DOCK_PANEL_WIDTH: f32 = 400.0;
/// 底部停靠面板的默认高度。
const DOCK_PANEL_HEIGHT: f32 = 260.0;
const PANEL_HEADER_HEIGHT: f32 = 36.0;
/// 未分组会话在折叠表里的键（与 `WorkspaceGroup` 的 `root: None` 对应）。
pub(super) const GROUP_KEY_UNGROUPED: &str = "ungrouped";

/// 一次注入给外壳的面板。
pub struct WorkbenchPanelEntry {
    pub kind: WorkbenchPanelKind,
    pub view: AnyView,
}

impl WorkbenchPanelEntry {
    pub fn new(kind: WorkbenchPanelKind, view: impl Into<AnyView>) -> Self {
        Self {
            kind,
            view: view.into(),
        }
    }
}

/// 外壳构造参数。
pub struct WorkbenchShellConfig {
    pub panels: Vec<WorkbenchPanelEntry>,
    /// 外部自绘会话导航；`None` 时改用 `session_source` 的内建列表。
    pub session_nav: Option<AnyView>,
    /// 会话来源面板；提供后外壳直接渲染会话列表并订阅其刷新。
    pub session_source: Option<Entity<DefaultAgentChatPanel>>,
    /// 初始布局（含面板落位与会话栏折叠）。宿主通常由持久化设置还原。
    pub initial_state: WorkbenchState,
    /// 主题覆盖；`None` 时取 `cx.theme()`。
    pub theme: Option<AgentChatTheme>,
    pub subscriptions: Vec<Subscription>,
    /// 当前工作区根目录；仅用于在顶部常显，切换由宿主驱动。
    pub workspace_root: Option<std::path::PathBuf>,
}

pub struct WorkbenchShell {
    pub(super) state: WorkbenchState,
    /// 面板视图，按标签实例身份存放。宿主注入的初始面板用 `seq = 0`；
    /// 多例面板（终端）的新实例由 `panel_factory` 创建后以新标签身份入表。
    pub(super) panels: HashMap<WorkbenchTab, AnyView>,
    pub(super) nav: Option<AnyView>,
    pub(super) session_source: Option<Entity<DefaultAgentChatPanel>>,
    /// 会话搜索框；仅内建会话列表（`session_source`）存在时创建。
    pub(super) search_input: Option<Entity<InputState>>,
    /// 搜索串（已同步自 `search_input`）。
    pub(super) search_query: String,
    pub(super) theme: Option<AgentChatTheme>,
    pub(super) tab_closeable: bool,
    pub(super) focus_handle: FocusHandle,
    /// 当前工作区根目录；`None` 表示宿主未提供。
    pub(super) workspace_root: Option<std::path::PathBuf>,
    /// 宿主注入的工作区选择动作（弹目录选择器并切换根）。
    pub(super) workspace_picker: Option<Box<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    /// 宿主注入的工作区切换动作（把 explorer 等宿主侧视图切到指定根，
    /// 经 `RootChanged` 级联回外壳与聊天面板）。侧栏分组的新建/选择依赖它。
    pub(super) workspace_switcher: Option<Box<dyn Fn(&std::path::Path, &mut gpui::App) + 'static>>,
    /// 宿主注入的多例面板工厂：为「新建页签」创建新实例视图（如新终端）。
    /// 入参为当前工作区根目录。未注入时多例面板的新建页签退化为定位。
    pub(super) panel_factory:
        Option<Box<dyn Fn(&std::path::Path, &mut Window, &mut gpui::App) -> AnyView + 'static>>,
    /// 宽度拖拽时落盘的节流时间戳（拖拽每帧都会触发，不能每帧写盘）。
    pub(super) last_width_persist: Option<Instant>,
    /// 收起的工作区分组（键为组根目录或 `ungrouped`）。仅会话内记忆：
    /// 折叠是浏览姿势，不值得落盘，也不该跨工作区串味。
    pub(super) collapsed_groups: HashSet<String>,
    /// 「新建页签」选择面板是否打开。虚拟页签：不进 `WorkbenchState`、
    /// 不落盘，点选面板后即关闭。
    pub(super) picker_open: bool,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl WorkbenchShell {
    pub fn new(
        config: WorkbenchShellConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let WorkbenchShellConfig {
            panels,
            session_nav,
            session_source,
            initial_state,
            theme,
            subscriptions: mut external_subscriptions,
            workspace_root,
        } = config;
        let mut subscriptions = Vec::new();
        subscriptions.append(&mut external_subscriptions);
        if let Some(source) = session_source.as_ref() {
            subscriptions.push(cx.observe(source, |_this, _, cx| cx.notify()));
        }

        // 左右侧栏开关下沉到会话面板工具条（agent 切换器两侧），外壳不再有
        // 独立顶栏。动作经外壳弱引用回调，状态由外壳实时提供。
        if let Some(source) = session_source.as_ref() {
            let shell = cx.entity().downgrade();
            let shell_nav_state = shell.clone();
            let shell_nav_action = shell.clone();
            let shell_right_state = shell.clone();
            let shell_right_action = shell.clone();
            source.update(cx, |panel, cx| {
                panel.set_workbench_toggles(
                    crate::agent_view::WorkbenchSidebarToggles {
                        nav_collapsed: std::sync::Arc::new(move |cx| {
                            shell_nav_state
                                .upgrade()
                                .map(|shell| shell.read(cx).state().nav_collapsed())
                                .unwrap_or(true)
                        }),
                        toggle_nav: std::sync::Arc::new(move |_, cx| {
                            if let Some(shell) = shell_nav_action.upgrade() {
                                shell.update(cx, |shell, cx| shell.toggle_session_nav(cx));
                            }
                        }),
                        right_open: std::sync::Arc::new(move |cx| {
                            shell_right_state
                                .upgrade()
                                .map(|shell| shell.read(cx).state().right_sidebar_open())
                                .unwrap_or(false)
                        }),
                        toggle_right: std::sync::Arc::new(move |_, cx| {
                            if let Some(shell) = shell_right_action.upgrade() {
                                shell.update(cx, |shell, cx| shell.toggle_right_sidebar(cx));
                            }
                        }),
                    },
                    cx,
                );
            });
        }

        // 内建会话列表才有搜索：外部注入的导航视图自带交互，外壳不掺和。
        let (search_input, search_query) = match session_source.as_ref() {
            Some(_) => {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |this, input, event: &InputEvent, _window, cx| {
                        if let InputEvent::Change = event {
                            this.search_query = input.read(cx).text().to_string();
                            cx.notify();
                        }
                    },
                ));
                (Some(input), String::new())
            }
            None => (None, String::new()),
        };

        Self {
            state: initial_state,
            panels: panels
                .into_iter()
                .map(|entry| (WorkbenchTab::new(entry.kind), entry.view))
                .collect(),
            nav: session_nav,
            session_source,
            search_input,
            search_query,
            theme,
            tab_closeable: true,
            focus_handle: cx.focus_handle(),
            workspace_root,
            workspace_picker: None,
            workspace_switcher: None,
            panel_factory: None,
            last_width_persist: None,
            collapsed_groups: HashSet::new(),
            picker_open: false,
            _subscriptions: subscriptions,
        }
    }

    /// 宿主在根目录变化后同步显示；不参与任何面板状态。
    pub fn set_workspace_root(&mut self, root: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.workspace_root.as_ref() == Some(&root) {
            return;
        }
        self.workspace_root = Some(root);
        cx.notify();
    }

    /// 注入「选择工作区」动作；点击顶部工作区标签时触发。
    pub fn set_workspace_picker(
        &mut self,
        picker: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.workspace_picker = Some(Box::new(picker));
        cx.notify();
    }

    /// 注入「切换到指定工作区」动作。侧栏分组的新建对话 / 选择跨工作区会话
    /// 时调用；宿主应把 explorer 等切到该根（`RootChanged` 会级联回外壳）。
    pub fn set_workspace_switcher(
        &mut self,
        switcher: impl Fn(&std::path::Path, &mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.workspace_switcher = Some(Box::new(switcher));
        cx.notify();
    }

    /// 切换到指定工作区（经宿主级联）。返回是否发出了切换。
    pub fn switch_to_workspace(
        &mut self,
        root: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_root.as_deref() == Some(root) {
            return false;
        }
        match self.workspace_switcher.as_ref() {
            Some(switcher) => {
                switcher(root, cx);
                true
            }
            None => false,
        }
    }

    /// 在指定工作区新建对话：先经宿主切根（级联同步），再开新会话。
    /// 新会话的归属在首次落盘时定格为该工作区。
    pub fn create_session_in_workspace(
        &mut self,
        root: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        self.switch_to_workspace(root, cx);
        if let Some(panel) = self.session_source.clone() {
            panel.update(cx, |panel, cx| panel.create_session(cx));
        }
    }

    /// 打开侧栏列表里的会话；若它属于另一个工作区，先经宿主切根。
    pub fn open_session_from_list(
        &mut self,
        uid: &str,
        workspace_root: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = workspace_root
            && self.workspace_root.as_deref() != Some(std::path::Path::new(root))
        {
            self.switch_to_workspace(std::path::Path::new(root), cx);
        }
        if let Some(panel) = self.session_source.clone() {
            panel.update(cx, |panel, cx| panel.select_session(uid, cx));
        }
    }

    /// 顶栏右侧栏开关：无标签时展开即打开第一个可用面板；有标签时在
    /// 收起/展开间切换；放大中先退出放大。
    pub fn toggle_right_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.state.right_maximized() {
            let changed = self.state.toggle_right_maximized();
            self.commit(changed, cx);
        } else if self.state.right_tabs().is_empty() && !self.state.right_collapsed() {
            let kind = WorkbenchPanelKind::DOCKABLE
                .into_iter()
                .find(|kind| self.has_panel(*kind));
            if let Some(kind) = kind {
                self.open_panel(kind, WorkbenchPlacement::Right, cx);
            }
        } else {
            let collapsed = !self.state.right_collapsed();
            let changed = self.state.set_right_collapsed(collapsed);
            self.commit(changed, cx);
        }
    }

    /// 右侧标签组「放大占满」开关。
    pub fn toggle_right_maximized(&mut self, cx: &mut Context<Self>) {
        let changed = self.state.toggle_right_maximized();
        self.commit(changed, cx);
    }

    /// 追加一个由宿主在构造之后创建的订阅（外壳创建早于其依赖时使用）。
    pub fn add_subscription(&mut self, subscription: Subscription, cx: &mut Context<Self>) {
        self._subscriptions.push(subscription);
        cx.notify();
    }

    /// 作为标签页内容时是否允许关闭。
    pub fn with_tab_closeable(mut self, closeable: bool) -> Self {
        self.tab_closeable = closeable;
        self
    }

    fn has_nav(&self) -> bool {
        self.nav.is_some() || self.session_source.is_some()
    }

    pub fn state(&self) -> &WorkbenchState {
        &self.state
    }

    pub fn has_panel(&self, kind: WorkbenchPanelKind) -> bool {
        self.panels.keys().any(|tab| tab.kind == kind)
    }

    pub fn has_session_nav(&self) -> bool {
        self.nav.is_some()
    }

    pub fn set_panel(&mut self, kind: WorkbenchPanelKind, view: AnyView, cx: &mut Context<Self>) {
        self.panels.insert(WorkbenchTab::new(kind), view);
        cx.notify();
    }

    /// 注入多例面板工厂：为「新建页签」创建新实例视图（如新终端）。
    /// 入参为外壳当前的工作区根目录。
    pub fn set_panel_factory(
        &mut self,
        factory: impl Fn(&std::path::Path, &mut Window, &mut gpui::App) -> AnyView + 'static,
        cx: &mut Context<Self>,
    ) {
        self.panel_factory = Some(Box::new(factory));
        cx.notify();
    }

    /// 工具条与面板头共用的唯一入口。
    ///
    /// 页签模型：rail / 工具条打开的面板一律进右侧标签组并激活；已在其他
    /// 落位（左/底/中心）的面板会被摘下来放进标签组。单例面板重复打开
    /// 只会定位；多例面板走定位语义（新建实例见 [`Self::pick_panel`]）。
    pub fn activate_panel(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = if self.state.placement_of(kind) == Some(WorkbenchPlacement::Right)
            && self
                .state
                .right_active()
                .map(|tab| tab.kind)
                == Some(kind)
        {
            false
        } else {
            self.state.open(kind, WorkbenchPlacement::Right)
        };
        self.commit(changed, cx);
    }

    /// 把面板打开到指定落位（右侧即为加入标签组）。
    pub fn open_panel(
        &mut self,
        kind: WorkbenchPanelKind,
        placement: WorkbenchPlacement,
        cx: &mut Context<Self>,
    ) {
        let changed = self.state.open(kind, placement);
        self.commit(changed, cx);
    }

    /// 从所有落位关闭面板。
    pub fn close_panel(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = self.state.close(kind);
        self.commit(changed, cx);
    }

    /// 选中右侧标签组里的某个面板（定位到它的第一个实例）。
    pub fn select_right_tab(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = self.state.set_right_active(kind);
        self.commit(changed, cx);
    }

    /// 选中右侧标签组里的某个具体实例（点击标签时用）。
    pub fn select_right_tab_instance(&mut self, tab: WorkbenchTab, cx: &mut Context<Self>) {
        let changed = self.state.set_right_active_tab(tab);
        self.commit(changed, cx);
    }

    /// 关闭右侧标签组里的一个具体实例（标签上的 X）。
    pub fn close_right_tab_instance(&mut self, tab: WorkbenchTab, cx: &mut Context<Self>) {
        let changed = self.state.close_tab(tab);
        self.commit(changed, cx);
    }

    /// 把面板移到下一个落位；顺序与 [`WorkbenchPlacement::CYCLE`] 一致。
    pub fn cycle_panel_placement(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = self.state.cycle_placement(kind);
        self.commit(changed, cx);
    }

    pub fn toggle_session_nav(&mut self, cx: &mut Context<Self>) {
        let changed = self.state.toggle_nav();
        self.commit(changed, cx);
    }

    /// 切换工作区分组的展开/收起。`key` 为组根目录字符串，未分组传
    /// [`GROUP_KEY_UNGROUPED`]。
    pub fn toggle_workspace_group(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed_groups.remove(key) {
            self.collapsed_groups.insert(key.to_string());
        }
        cx.notify();
    }

    /// 该工作区分组当前是否被收起。
    pub fn group_collapsed(&self, key: &str) -> bool {
        self.collapsed_groups.contains(key)
    }

    /// 打开/关闭「新建页签」选择面板（虚拟页签）。
    pub fn toggle_tab_picker(&mut self, cx: &mut Context<Self>) {
        self.picker_open = !self.picker_open;
        if self.picker_open && self.state.right_collapsed() {
            // 从收起态点 +：先展开右侧组，否则选择面板看不见。
            let changed = self.state.set_right_collapsed(false);
            self.commit(changed, cx);
        }
        cx.notify();
    }

    /// 关闭「新建页签」选择面板（点选面板 / 点 X 时调用）。
    pub fn close_tab_picker(&mut self, cx: &mut Context<Self>) {
        if !self.picker_open {
            return;
        }
        self.picker_open = false;
        cx.notify();
    }

    /// 从选择面板打开一个面板：当前页签即变为该功能，选择面板随之关闭。
    ///
    /// 多例面板（终端等）不走定位：每次点选都经工厂创建一个新实例并作为
    /// 新标签打开。工厂未注入时退化为定位语义。
    pub fn pick_panel(
        &mut self,
        kind: WorkbenchPanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker_open = false;
        if kind.multi_instance() {
            self.open_new_instance_tab(kind, window, cx);
        } else {
            self.activate_panel(kind, cx);
        }
    }

    /// 为多例面板创建一个新实例标签。
    fn open_new_instance_tab(
        &mut self,
        kind: WorkbenchPanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(factory) = self.panel_factory.as_ref() else {
            self.activate_panel(kind, cx);
            return;
        };
        let root = self
            .workspace_root
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let Some(tab) = self.state.open_new_tab(kind) else {
            return;
        };
        let view = factory(&root, window, cx);
        self.panels.insert(tab, view);
        self.commit(true, cx);
    }

    /// 拖拽调宽：立即重绘；落盘按 [`WIDTH_PERSIST_THROTTLE`] 节流，
    /// 拖拽中的中间值不值得每帧写盘。
    pub fn set_nav_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if self.state.set_nav_width(width) {
            self.persist_width_throttled(cx);
            cx.notify();
        }
    }

    pub fn set_right_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if self.state.set_right_width(width) {
            self.persist_width_throttled(cx);
            cx.notify();
        }
    }

    const fn width_persist_throttle() -> Duration {
        Duration::from_millis(250)
    }

    fn persist_width_throttled(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let due = self
            .last_width_persist
            .is_none_or(|last| now.duration_since(last) >= Self::width_persist_throttle());
        if due {
            self.last_width_persist = Some(now);
            self.persist_layout(cx);
        }
    }

    /// 状态变更的统一出口：落盘 + 重绘。
    fn commit(&mut self, changed: bool, cx: &mut Context<Self>) {
        if !changed {
            return;
        }
        self.persist_layout(cx);
        // 工具条上的侧栏开关图标由外壳状态驱动，面板得跟着重绘一次，
        // 否则点外壳侧的「放大」后工具条图标会停在旧状态。
        if let Some(panel) = self.session_source.clone() {
            panel.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// 布局变化立刻落盘。
    ///
    /// 没有全局设置时直接跳过：单元测试不会因为一次点击就去改用户真实的配置文件。
    fn persist_layout(&self, cx: &mut Context<Self>) {
        if cx.try_global::<AppSettings>().is_none() {
            return;
        }
        let layout = self.state.to_settings();
        AppSettings::update_and_save(cx, |settings| {
            settings.ai_chat.workbench_layout = layout;
        });
    }

    fn snapshot(&self, cx: &gpui::App) -> AgentChatTheme {
        self.theme
            .clone()
            .unwrap_or_else(|| AgentChatTheme::from_app(cx))
    }
}
