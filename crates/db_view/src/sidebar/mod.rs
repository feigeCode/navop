//! 数据库视图侧边栏模块
//!
//! 提供数据库视图的侧边栏功能，包括：
//! - AI 聊天面板

mod ai_context;
pub(crate) mod cell_preview_panel;
pub(crate) mod execution_history;
pub(crate) mod execution_history_panel;
mod execution_history_view;

use ai_chat_view::{
    AskAiEvent, CodeBlockAction, DefaultAgentChatPanel, DefaultAgentChatPanelEvent,
    DefaultTargetReason, ResourceCatalog, ResourceContext, ResourceId, build_resource_catalog,
    build_sidebar_resource_state, get_ask_ai_notifier,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};
use gpui_component::{ActiveTheme, Icon, Selectable, Size, h_flex, v_flex};
use one_assets::IconName;
use one_core::layout::TOOLBAR_WIDTH;
use one_core::storage::StoredConnection;
use one_ui::{IconButton, IconSize as OneIconSize};
use rust_i18n::t;

use self::ai_context::{
    SelectedDatabaseScope, TableMentionLoad, TableMentionLoadParts, apply_database_scope,
};
use self::execution_history_panel::ExecutionHistoryPanel;
use crate::extension_panels::{
    self, DatabasePanelContext, ExtensionPanelEntry,
};
use crate::sql_library::SqlLibraryPanel;

/// 面板 `when` 求值上下文：当前活跃连接的类型与参数。
fn panel_context(connection: Option<&StoredConnection>) -> DatabasePanelContext {
    let Some(connection) = connection else {
        return DatabasePanelContext::default();
    };
    let connection_id = connection.id;
    DatabasePanelContext {
        connection_id,
        connection_type: Some(connection.connection_type.to_string().to_lowercase()),
        params: serde_json::from_str(&connection.params).ok(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    AiChat,
    ExecutionHistory,
    SqlLibrary,
}

#[derive(Clone, Debug)]
pub enum DatabaseSidebarEvent {
    PanelChanged,
    AskAi,
}

pub struct DatabaseSidebar {
    active_panel: Option<SidebarPanel>,
    /// 扩展面板（`anchor: "db-sidebar"`）：工具栏按钮 + 视图缓存。
    extension_panels: Vec<ExtensionPanelEntry>,
    /// 本侧栏所在的页签容器：面板里 `App.open` 要落在同一个容器里。
    extension_tab_container: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
    /// 当前打开的扩展面板（与内建 `active_panel` 互斥）。
    active_extension: Option<SharedString>,
    chat_panel: Entity<DefaultAgentChatPanel>,
    execution_history: Entity<ExecutionHistoryPanel>,
    sql_library: Entity<SqlLibraryPanel>,
    connections: Vec<StoredConnection>,
    active_conn_id: Option<i64>,
    table_context_seq: usize,
    focus_handle: FocusHandle,
    is_active: bool,
    _subs: Vec<Subscription>,
}

impl DatabaseSidebar {
    pub fn new(
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        execution_history: Entity<ExecutionHistoryPanel>,
        sql_library: Entity<SqlLibraryPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let active_connection = active_conn_id
            .and_then(|id| connections.iter().find(|conn| conn.id == Some(id)))
            .or_else(|| connections.first());
        let ai_access_levels = ai_chat_view::load_ai_access_levels(cx);
        let chat_panel = if let Some(connection) = active_connection {
            let (scope, catalog, mentions) = build_sidebar_resource_state(
                connection,
                &connections,
                DefaultTargetReason::CurrentDatabase,
                &ai_access_levels,
            );
            cx.new(|cx| {
                DefaultAgentChatPanel::new_sidebar_with_scope_and_catalog(
                    scope, catalog, mentions, window, cx,
                )
            })
        } else {
            cx.new(|cx| DefaultAgentChatPanel::new(window, cx))
        };

        let mut subs = Vec::new();
        // 面板除了「关闭」还会发 TurnStarted / TurnFinished / 子代理详情等事件，那些由
        // 工作台外壳处理。这里以前对任何事件都收起侧栏，用户一发消息（TurnStarted）AI
        // 面板就自己消失了。
        subs.push(cx.subscribe(
            &chat_panel,
            |this, _, event: &DefaultAgentChatPanelEvent, cx| match event {
                DefaultAgentChatPanelEvent::Close => {
                    this.set_active_panel(None, cx);
                }
                // 数据库侧栏固定在页签内，没有「移动到另一侧」的去处。
                DefaultAgentChatPanelEvent::MoveTo(_)
                | DefaultAgentChatPanelEvent::TurnStarted { .. }
                | DefaultAgentChatPanelEvent::TurnFinished { .. }
                | DefaultAgentChatPanelEvent::RestoreTurn { .. }
                | DefaultAgentChatPanelEvent::OpenFileInReview { .. }
                | DefaultAgentChatPanelEvent::OpenTurnReview { .. }
                | DefaultAgentChatPanelEvent::SubagentDetailRequested { .. }
                | DefaultAgentChatPanelEvent::SubagentDetailUpdated { .. } => {}
            },
        ));

        if let Some(notifier) = get_ask_ai_notifier(cx) {
            subs.push(
                cx.subscribe(&notifier, move |this, _, event: &AskAiEvent, cx| {
                    if this.is_active {
                        let AskAiEvent::Request(message) = event;
                        this.ask_ai(message.clone(), cx);
                    }
                }),
            );
        }

        let extension_panels = extension_panels::sync_extension_panels(
            Vec::new(),
            extension_panels::database_extension_panel_specs(
                cx,
                &panel_context(active_connection),
            ),
        );

        Self {
            active_panel: None,
            extension_panels,
            extension_tab_container: None,
            active_extension: None,
            chat_panel,
            execution_history,
            sql_library,
            connections,
            active_conn_id,
            table_context_seq: 0,
            focus_handle: cx.focus_handle(),
            is_active: false,
            _subs: subs,
        }
    }

    /// 记下本侧栏所在的页签容器：面板里 `App.open` 要落在同一个容器里，
    /// 而不是主窗口的页签栏。
    pub fn set_extension_tab_container(
        &mut self,
        container: gpui::WeakEntity<one_core::tab_container::TabContainer>,
    ) {
        self.extension_tab_container = Some(container);
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        let became_active = active && !self.is_active;
        self.is_active = active;
        if became_active {
            // 扩展可能在标签页不活跃期间被安装/停用：激活时对齐一次。
            self.sync_extension_panels(cx);
        }
        if became_active && self.active_panel == Some(SidebarPanel::AiChat) {
            self.chat_panel.update(cx, |panel, cx| {
                panel.on_sidebar_shown(cx);
            });
        }
        cx.notify();
    }

    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        if self.active_panel != panel {
            self.active_panel = panel;
            self.active_extension = None;
            if self.is_active && panel == Some(SidebarPanel::AiChat) {
                self.chat_panel.update(cx, |panel, cx| {
                    panel.on_sidebar_shown(cx);
                });
            }
            if panel == Some(SidebarPanel::ExecutionHistory) {
                self.execution_history.update(cx, |history, cx| {
                    history.reload(cx);
                });
            }
            if panel == Some(SidebarPanel::SqlLibrary) {
                self.sql_library.update(cx, |library, cx| {
                    library.reload(cx);
                });
            }
            cx.emit(DatabaseSidebarEvent::PanelChanged);
            cx.notify();
        }
    }

    pub fn toggle_panel(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.active_panel == Some(panel) {
            self.set_active_panel(None, cx);
        } else {
            self.set_active_panel(Some(panel), cx);
        }
    }

    pub fn is_panel_visible(&self) -> bool {
        self.active_panel.is_some() || self.active_extension.is_some()
    }

    /// 按宿主提供的规格对齐扩展面板（保留已建视图）。
    pub fn sync_extension_panels(&mut self, cx: &mut Context<Self>) {
        let context = panel_context(self.active_connection());
        let specs = extension_panels::database_extension_panel_specs(cx, &context);
        let previous: Vec<(SharedString, SharedString, i64)> = self
            .extension_panels
            .iter()
            .map(|entry| (entry.id.clone(), entry.title.clone(), entry.order))
            .collect();
        let next = extension_panels::sync_extension_panels(
            std::mem::take(&mut self.extension_panels),
            specs,
        );
        // 只在“面板集合或顺序/标题真的变了”时通知，避免每次激活标签页都重绘。
        let changed = next.len() != previous.len()
            || next
                .iter()
                .zip(previous.iter())
                .any(|(entry, (id, title, order))| {
                    entry.id != *id || entry.title != *title || entry.order != *order
                });
        self.extension_panels = next;
        let removed_active = match self.active_extension.as_ref() {
            Some(active) => !self.extension_panels.iter().any(|entry| &entry.id == active),
            None => false,
        };
        if removed_active {
            self.active_extension = None;
            cx.emit(DatabaseSidebarEvent::PanelChanged);
        }
        if changed || removed_active {
            cx.notify();
        }
    }

    /// 切换扩展面板（首次打开时经宿主创建视图）。
    pub fn toggle_extension_panel(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.active_extension.as_ref() == Some(&id) {
            self.close_extension_panel(&id, cx);
            return;
        }
        // 面板视图可能来自另一条连接（when 只过滤“可见”，不区分连接），
        // 所以每次真正打开都用当前连接的上下文重建它。
        let panel_context = panel_context(self.active_connection());
        let has_view = match self
            .extension_panels
            .iter_mut()
            .find(|entry| entry.id == id)
        {
            Some(entry) => {
                if entry.view_connection != panel_context.connection_id {
                    entry.view = None;
                }
                if entry.view.is_none() {
                    entry.view = extension_panels::open_database_extension_panel(
                        entry.id.as_ref(),
                        &panel_context,
                        self.extension_tab_container.clone(),
                        cx,
                    );
                    entry.view_connection = panel_context.connection_id;
                }
                entry.view.is_some()
            }
            None => false,
        };
        if !has_view {
            tracing::warn!(
                panel = %id,
                "扩展面板没有可用视图（宿主未注册或面板已失效）"
            );
            return;
        }
        self.active_panel = None;
        self.active_extension = Some(id);
        cx.emit(DatabaseSidebarEvent::PanelChanged);
        cx.notify();
    }

    /// 关闭扩展面板（视图保留，下次打开复用）。
    pub fn close_extension_panel(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        if self.active_extension.as_ref() == Some(id) {
            self.active_extension = None;
            cx.emit(DatabaseSidebarEvent::PanelChanged);
            cx.notify();
        }
    }

    /// 当前活跃连接的引用（面板 when 上下文与资源范围共用）。
    fn active_connection(&self) -> Option<&StoredConnection> {
        self.active_conn_id
            .and_then(|id| self.connections.iter().find(|conn| conn.id == Some(id)))
            .or_else(|| self.connections.first())
    }

    /// 执行历史面板句柄：SQL 编辑器页签需要它来记录执行历史。
    pub fn execution_history(&self) -> Entity<ExecutionHistoryPanel> {
        self.execution_history.clone()
    }

    pub fn ask_ai(&mut self, message: String, cx: &mut Context<Self>) {
        if self.active_panel != Some(SidebarPanel::AiChat) {
            self.active_panel = Some(SidebarPanel::AiChat);
            self.active_extension = None;
        }

        self.chat_panel.update(cx, |panel, cx| {
            panel.send_external_message(message, cx);
        });

        cx.emit(DatabaseSidebarEvent::AskAi);
        cx.notify();
    }

    pub fn set_database_scope(
        &mut self,
        connection_id: &str,
        database: Option<String>,
        schema: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let active_id = connection_id.parse::<i64>().ok().or(self.active_conn_id);
        let active_connection = active_id
            .and_then(|id| self.connections.iter().find(|conn| conn.id == Some(id)))
            .or_else(|| self.connections.first());
        let ai_access_levels = ai_chat_view::load_ai_access_levels(cx);
        let (mut resources, mentions, catalog) = if let Some(connection) = active_connection {
            let (scope, catalog, mentions) = build_sidebar_resource_state(
                connection,
                &self.connections,
                DefaultTargetReason::CurrentDatabase,
                &ai_access_levels,
            );
            (scope.to_resource_context(), mentions, catalog)
        } else {
            (
                ResourceContext::new(),
                Vec::new(),
                ResourceCatalog::new(build_resource_catalog(&self.connections, &ai_access_levels)),
            )
        };
        let resource_id = ResourceId::new(connection_id.to_string());
        apply_database_scope(
            &mut resources,
            &resource_id,
            SelectedDatabaseScope {
                database: database.as_deref(),
                schema: schema.as_deref(),
            },
        );
        resources.current = Some(resource_id);
        self.table_context_seq = self.table_context_seq.wrapping_add(1);
        let seq = self.table_context_seq;
        let load = TableMentionLoad::new(TableMentionLoadParts {
            seq,
            connection_id: connection_id.to_string(),
            database,
            schema,
            resources: resources.clone(),
            mentions: mentions.clone(),
        });
        self.chat_panel.update(cx, |panel, cx| {
            panel.set_resource_context_with_catalog(
                resources.clone(),
                mentions.clone(),
                catalog.resources.clone(),
                cx,
            );
        });
        self.load_table_mentions(load, cx);
        cx.notify();
    }

    pub fn register_code_block_action(&self, action: CodeBlockAction, cx: &mut Context<Self>) {
        self.chat_panel.update(cx, |panel, cx| {
            panel.register_code_block_action(action, cx);
        });
    }

    fn render_toolbar_button(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_panel == Some(panel);
        let item_size = Size::Size(one_ui::theme_geometry().layout.global_rail_item);
        let (icon, tooltip) = match panel {
            SidebarPanel::AiChat => (IconName::AILine, t!("DatabaseSidebar.ai_chat").to_string()),
            SidebarPanel::ExecutionHistory => (
                IconName::BookOpen,
                t!("DatabaseSidebar.execution_history").to_string(),
            ),
            SidebarPanel::SqlLibrary => (IconName::SqlLibrary, t!("SqlLibrary.title").to_string()),
        };

        IconButton::new(format!("database-sidebar-btn-{panel:?}"), Icon::new(icon))
            .hit_size(item_size)
            .glyph_size(OneIconSize::Medium)
            .selected(is_active)
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_panel(panel, cx);
            }))
    }

    pub fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let border_color = cx.theme().border;
        let muted_bg = cx.theme().muted;

        v_flex()
            .flex_shrink_0()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .bg(muted_bg)
            .border_l_1()
            .border_color(border_color)
            .items_center()
            .py_2()
            .gap_1()
            .child(self.render_toolbar_button(SidebarPanel::AiChat, window, cx))
            .child(self.render_toolbar_button(SidebarPanel::ExecutionHistory, window, cx))
            .child(self.render_toolbar_button(SidebarPanel::SqlLibrary, window, cx))
            .when(!self.extension_panels.is_empty(), |this| {
                let border_color = cx.theme().border;
                let buttons: Vec<AnyElement> = self
                    .extension_panels
                    .iter()
                    .map(|entry| {
                        let id = entry.id.clone();
                        let selected = self.active_extension.as_ref() == Some(&entry.id);
                        IconButton::new(
                            format!("database-sidebar-ext-{}", entry.id),
                            entry.icon.clone(),
                        )
                        .hit_size(Size::Size(one_ui::theme_geometry().layout.global_rail_item))
                        .glyph_size(OneIconSize::Medium)
                        .selected(selected)
                        .tooltip(entry.title.clone())
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.toggle_extension_panel(id.clone(), cx);
                        }))
                        .into_any_element()
                    })
                    .collect();
                this.child(
                    div()
                        .w(gpui::px(16.0))
                        .h(gpui::px(1.0))
                        .my_1()
                        .bg(border_color),
                )
                .children(buttons)
            })
            .into_any_element()
    }

    pub fn render_panel_content(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        match panel {
            SidebarPanel::AiChat => self.chat_panel.clone().into_any_element(),
            SidebarPanel::ExecutionHistory => self.execution_history.clone().into_any_element(),
            SidebarPanel::SqlLibrary => self.sql_library.clone().into_any_element(),
        }
    }
}

impl DatabaseSidebar {
    /// 扩展面板：标题栏（标题 + 关闭）+ 宿主给的视图。
    fn render_extension_panel(
        &self,
        id: SharedString,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(entry) = self.extension_panels.iter().find(|entry| entry.id == id) else {
            return div().into_any_element();
        };
        let title = entry.title.clone();
        let view = entry.view.clone();
        let close_id = id.clone();
        let header_height = one_ui::theme_geometry().layout.panel_header;
        let border_color = cx.theme().border;

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(header_height)
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(border_color)
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(title))
                    .child(
                        IconButton::new(
                            format!("database-sidebar-ext-close-{id}"),
                            Icon::new(IconName::Close),
                        )
                        .glyph_size(OneIconSize::Small)
                        .tooltip(t!("DatabaseSidebar.close_extension").to_string())
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.close_extension_panel(&close_id, cx);
                        })),
                    ),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().children(view))
            .into_any_element()
    }
}

impl EventEmitter<DatabaseSidebarEvent> for DatabaseSidebar {}

impl Focusable for DatabaseSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DatabaseSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg_color = cx.theme().background;
        let active_panel = self.active_panel;
        let active_extension = self.active_extension.clone();
        let visible = active_panel.is_some() || active_extension.is_some();

        h_flex()
            .h_full()
            .flex_shrink_0()
            .bg(bg_color)
            .when(visible, |this| this.w_full())
            .when(!visible, |this| this.w(TOOLBAR_WIDTH))
            .when_some(active_panel, |this, panel| {
                this.child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .overflow_hidden()
                        .bg(bg_color)
                        .child(self.render_panel_content(panel, window, cx)),
                )
            })
            .when_some(active_extension, |this, id| {
                this.child(self.render_extension_panel(id, window, cx))
            })
            .child(self.render_toolbar(window, cx))
    }
}

#[cfg(test)]
mod tests {
    /// 扩展面板的三处接线：工具栏按钮、面板渲染分支、宿主规格同步。
    ///
    /// 用结构断言而不是渲染断言：真实渲染需要 nsh 视图，这里只保证接线不会
    /// 被后续重构顺手删掉（终端侧栏用的是同一套办法）。
    #[test]
    fn extension_panels_are_wired_into_toolbar_render_and_sync() {
        let source = include_str!("mod.rs");
        assert!(source.contains("database-sidebar-ext-"), "工具栏缺扩展按钮");
        assert!(source.contains("fn render_extension_panel("), "缺面板渲染分支");
        assert!(source.contains("self.sync_extension_panels(cx)"), "缺同步调用");
        assert!(
            source.contains("self.active_panel.is_some() || self.active_extension.is_some()"),
            "可见性判断没有纳入扩展面板"
        );

        let tab = include_str!("../database_tab.rs");
        assert!(tab.contains("sync_extension_panels(cx)"), "构造时没有同步规格");
    }

    /// 回归：发消息会先发 `TurnStarted`，侧栏不能把它当成「关闭」。
    ///
    /// 旧实现订阅了整个 `DefaultAgentChatPanelEvent` 却不看变体，一律 `active_panel = None`，
    /// 于是用户一按发送，侧栏里的 AI 面板就自己消失。这里钉住「只有 Close 收起侧栏」。
    #[test]
    fn ai_panel_stays_open_on_turn_events() {
        // 只看生产代码：`include_str!` 会把本测试也读进来，needle 必须只在正文里找。
        let source = include_str!("mod.rs");
        let body = source.split("#[cfg(test)]").next().unwrap_or(source);

        // 旧写法的特征串拼出来，否则会命中测试自身里的字面量。
        let unconditional = format!("|this, _, {}: &DefaultAgentChatPanelEvent, cx|", "_event");
        assert!(
            !body.contains(&unconditional),
            "订阅又变回不看变体、任何事件都收起侧栏的写法"
        );
        assert!(
            body.contains("DefaultAgentChatPanelEvent::TurnStarted { .. }"),
            "TurnStarted 没有被显式忽略"
        );
        assert!(
            body.contains("DefaultAgentChatPanelEvent::Close =>"),
            "缺 Close 分支，AI 面板关不掉"
        );
        assert!(
            body.contains("this.set_active_panel(None, cx);"),
            "Close 没有走统一的收起入口"
        );
    }
}
