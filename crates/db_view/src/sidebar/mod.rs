//! 数据库视图侧边栏模块
//!
//! 提供数据库视图的侧边栏功能，包括：
//! - AI 聊天面板

mod ai_context;
pub(crate) mod cell_preview_panel;

use ai_chat_view::{
    AskAiEvent, CodeBlockAction, DefaultAgentChatPanel, DefaultAgentChatPanelEvent,
    DefaultTargetReason, ResourceCatalog, ResourceContext, ResourceId, build_resource_catalog,
    build_sidebar_resource_state, get_ask_ai_notifier,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, h_flex, v_flex};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::storage::StoredConnection;

use self::ai_context::{
    SelectedDatabaseScope, TableMentionLoad, TableMentionLoadParts, apply_database_scope,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    AiChat,
}

impl SidebarPanel {
    pub fn icon(&self) -> Icon {
        match self {
            SidebarPanel::AiChat => IconName::AI.color(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DatabaseSidebarEvent {
    PanelChanged,
    AskAi,
}

pub struct DatabaseSidebar {
    active_panel: Option<SidebarPanel>,
    chat_panel: Entity<DefaultAgentChatPanel>,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let active_connection = active_conn_id
            .and_then(|id| connections.iter().find(|conn| conn.id == Some(id)))
            .or_else(|| connections.first());
        let chat_panel = if let Some(connection) = active_connection {
            let (scope, catalog, mentions) = build_sidebar_resource_state(
                connection,
                &connections,
                DefaultTargetReason::CurrentDatabase,
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
        subs.push(cx.subscribe(
            &chat_panel,
            |this, _, _event: &DefaultAgentChatPanelEvent, cx| {
                this.active_panel = None;
                cx.emit(DatabaseSidebarEvent::PanelChanged);
                cx.notify();
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

        Self {
            active_panel: None,
            chat_panel,
            connections,
            active_conn_id,
            table_context_seq: 0,
            focus_handle: cx.focus_handle(),
            is_active: false,
            _subs: subs,
        }
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.is_active = active;
        cx.notify();
    }

    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        if self.active_panel != panel {
            self.active_panel = panel;
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
        self.active_panel.is_some()
    }

    pub fn ask_ai(&mut self, message: String, cx: &mut Context<Self>) {
        if self.active_panel != Some(SidebarPanel::AiChat) {
            self.active_panel = Some(SidebarPanel::AiChat);
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
        let (mut resources, mentions, catalog) = if let Some(connection) = active_connection {
            let (scope, catalog, mentions) = build_sidebar_resource_state(
                connection,
                &self.connections,
                DefaultTargetReason::CurrentDatabase,
            );
            (scope.to_resource_context(), mentions, catalog)
        } else {
            (
                ResourceContext::new(),
                Vec::new(),
                ResourceCatalog::new(build_resource_catalog(&self.connections)),
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
        let accent_color = cx.theme().accent;
        let accent_fg = cx.theme().accent_foreground;
        let muted_fg = cx.theme().muted_foreground;
        let muted_bg = cx.theme().muted;

        div()
            .id(SharedString::from(format!("sidebar-btn-{:?}", panel)))
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .when(is_active, |this| this.bg(accent_color))
            .when(!is_active, |this| this.hover(|s| s.bg(muted_bg)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_panel(panel, cx);
            }))
            .child(
                Icon::new(panel.icon())
                    .with_size(Size::Medium)
                    .text_color(if is_active { accent_fg } else { muted_fg }),
            )
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
        }
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

        h_flex()
            .h_full()
            .flex_shrink_0()
            .bg(bg_color)
            .when(active_panel.is_some(), |this| this.w_full())
            .when(active_panel.is_none(), |this| this.w(TOOLBAR_WIDTH))
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
            .child(self.render_toolbar(window, cx))
    }
}
