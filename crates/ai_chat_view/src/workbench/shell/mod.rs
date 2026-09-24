//! 工作台外壳：会话导航 + 中心区 + 左右底三处停靠 + 工具条。
//!
//! 外壳不持有业务会话：面板以 [`WorkbenchPanelEntry`] 注入，外壳只负责切换、
//! 停靠、折叠与布局落盘；面板之间不互相持有。落位语义在 [`super::state`]，
//! 本模块只做渲染与状态编排。

use std::collections::HashMap;

use gpui::{AnyView, AppContext as _, Context, Entity, FocusHandle, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};
use one_core::settings::AppSettings;

use super::state::{WorkbenchPanelKind, WorkbenchPlacement, WorkbenchState};
use crate::DefaultAgentChatPanel;
use crate::theme::AgentChatTheme;

mod impls;
mod nav;
mod widgets;

/// 侧边停靠面板的默认宽度。
const DOCK_PANEL_WIDTH: f32 = 400.0;
/// 底部停靠面板的默认高度。
const DOCK_PANEL_HEIGHT: f32 = 260.0;
const HEADER_HEIGHT: f32 = 40.0;
const PANEL_HEADER_HEIGHT: f32 = 36.0;

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
    pub(super) panels: HashMap<WorkbenchPanelKind, AnyView>,
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
                .map(|entry| (entry.kind, entry.view))
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
        self.panels.contains_key(&kind)
    }

    pub fn has_session_nav(&self) -> bool {
        self.nav.is_some()
    }

    pub fn set_panel(&mut self, kind: WorkbenchPanelKind, view: AnyView, cx: &mut Context<Self>) {
        self.panels.insert(kind, view);
        cx.notify();
    }

    /// 工具条与面板头共用的唯一入口。
    ///
    /// - 面板未打开 → 打开到中心区。
    /// - 面板已打开但不在中心区 → 提到中心区。
    /// - 面板在中心区 → 关闭它（「对话」是中心区主场，关不掉）。
    pub fn activate_panel(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = if self.state.center() == Some(kind) {
            self.state.close(kind)
        } else {
            self.state.open(kind, WorkbenchPlacement::Center)
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

    /// 选中右侧标签组里的某个面板。
    pub fn select_right_tab(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        let changed = self.state.set_right_active(kind);
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

    /// 状态变更的统一出口：落盘 + 重绘。
    fn commit(&mut self, changed: bool, cx: &mut Context<Self>) {
        if !changed {
            return;
        }
        self.persist_layout(cx);
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
