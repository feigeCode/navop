//! 工作台外壳：会话导航 + 内容区 + 工具面板停靠。
//!
//! 外壳不持有业务会话：会话、审阅、文件、终端都以 [`WorkbenchPanelEntry`]
//! 注入，外壳只负责切换、停靠与折叠；面板之间不互相持有。
//! 停靠语义在 [`super::state`]，本模块只做渲染。

use std::collections::HashMap;

use gpui::{AnyView, App, Context, Entity, FocusHandle, Subscription, Window};
use one_core::sidebar_contribution::SidebarPlacement;

use super::state::{WorkbenchPanelKind, WorkbenchState};
use crate::DefaultAgentChatPanel;
use crate::theme::AgentChatTheme;

mod impls;
mod nav;
mod widgets;


/// 侧栏内嵌面板的默认宽度。
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
    pub initial_active: WorkbenchPanelKind,
    /// 主题覆盖；`None` 时取 `cx.theme()`。
    pub theme: Option<AgentChatTheme>,
    pub subscriptions: Vec<Subscription>,
    /// 当前工作区根目录；仅用于在顶部常显，切换由宿主驱动。
    pub workspace_root: Option<std::path::PathBuf>,
}

/// 外壳对外事件：面板内请求“在内容区打开某个标签”。
#[derive(Clone, Copy, Debug)]
pub enum WorkbenchShellEvent {
    RequestOpenTab(WorkbenchPanelKind),
}

pub struct WorkbenchShell {
    pub(super) state: WorkbenchState,
    pub(super) panels: HashMap<WorkbenchPanelKind, AnyView>,
    pub(super) nav: Option<AnyView>,
    pub(super) session_source: Option<Entity<DefaultAgentChatPanel>>,
    pub(super) theme: Option<AgentChatTheme>,
    pub(super) tab_closeable: bool,
    pub(super) focus_handle: FocusHandle,
    /// 当前工作区根目录；`None` 表示宿主未提供。
    pub(super) workspace_root: Option<std::path::PathBuf>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl WorkbenchShell {
    pub fn new(
        config: WorkbenchShellConfig,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let WorkbenchShellConfig {
            panels,
            session_nav,
            session_source,
            initial_active,
            theme,
            subscriptions: mut external_subscriptions,
            workspace_root,
        } = config;
        let mut subscriptions = Vec::new();
        subscriptions.append(&mut external_subscriptions);
        if let Some(source) = session_source.as_ref() {
            subscriptions.push(cx.observe(source, |_this, _, cx| cx.notify()));
        }
        Self {
            state: WorkbenchState::new(initial_active),
            panels: panels
                .into_iter()
                .map(|entry| (entry.kind, entry.view))
                .collect(),
            nav: session_nav,
            session_source,
            theme,
            tab_closeable: true,
            focus_handle: cx.focus_handle(),
            workspace_root,
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

    pub fn set_active(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        if self.has_panel(kind) {
            self.state.set_active(kind);
            cx.notify();
        }
    }

    pub fn toggle_session_nav(&mut self, cx: &mut Context<Self>) {
        self.state.toggle_nav();
        cx.notify();
    }

    pub fn toggle_dock_panel(&mut self, kind: WorkbenchPanelKind, cx: &mut Context<Self>) {
        if !self.has_panel(kind) {
            return;
        }
        self.state.toggle_panel(kind, SidebarPlacement::Right);
        cx.notify();
    }

    pub fn close_dock_panel(&mut self, placement: SidebarPlacement, cx: &mut Context<Self>) {
        if self.state.close_panel(placement) {
            cx.notify();
        }
    }

    pub fn move_dock_panel(
        &mut self,
        kind: WorkbenchPanelKind,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        if self.state.move_panel(kind, placement) {
            cx.notify();
        }
    }

    fn snapshot(&self, cx: &App) -> AgentChatTheme {
        self.theme
            .clone()
            .unwrap_or_else(|| AgentChatTheme::from_app(cx))
    }
}
