//! MQTT 侧边栏(照 redis_view/sidebar.rs 复制,提供 AI 聊天面板)

use ai_chat_view::{
    AskAiEvent, DefaultAgentChatPanel, DefaultAgentChatPanelEvent, DefaultTargetReason,
    build_sidebar_resource_state, get_ask_ai_notifier,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
};
use gpui_component::{
    ActiveTheme, IconName, IconSize, ObjectIcon, Selectable, Size, button::IconButton, h_flex,
    v_flex,
};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::storage::StoredConnection;
use rust_i18n::t;

/// 侧边栏面板类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    AiChat,
}

/// MQTT 侧边栏事件
#[derive(Clone, Debug)]
pub enum MqttSidebarEvent {
    PanelChanged,
    AskAi,
}

/// MQTT 侧边栏
pub struct MqttSidebar {
    active_panel: Option<SidebarPanel>,
    ai_chat_panel: Entity<DefaultAgentChatPanel>,
    focus_handle: FocusHandle,
    is_active: bool,
    _subs: Vec<Subscription>,
}

impl MqttSidebar {
    pub fn new(
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let active_connection = active_conn_id
            .and_then(|id| connections.iter().find(|conn| conn.id == Some(id)))
            .or_else(|| connections.first());
        let ai_chat_panel = if let Some(connection) = active_connection {
            let (scope, catalog, mentions) = build_sidebar_resource_state(
                connection,
                &connections,
                DefaultTargetReason::CurrentConnection,
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
            &ai_chat_panel,
            |this, _, _event: &DefaultAgentChatPanelEvent, cx| {
                this.active_panel = None;
                cx.emit(MqttSidebarEvent::PanelChanged);
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
            ai_chat_panel,
            focus_handle: cx.focus_handle(),
            is_active: false,
            _subs: subs,
        }
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        let became_active = active && !self.is_active;
        self.is_active = active;
        if became_active && self.active_panel == Some(SidebarPanel::AiChat) {
            self.ai_chat_panel.update(cx, |panel, cx| {
                panel.on_sidebar_shown(cx);
            });
        }
        cx.notify();
    }

    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        if self.active_panel != panel {
            self.active_panel = panel;
            if self.is_active && panel == Some(SidebarPanel::AiChat) {
                self.ai_chat_panel.update(cx, |panel, cx| {
                    panel.on_sidebar_shown(cx);
                });
            }
            cx.emit(MqttSidebarEvent::PanelChanged);
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

        self.ai_chat_panel.update(cx, |panel, cx| {
            panel.send_external_message(message, cx);
        });

        cx.emit(MqttSidebarEvent::AskAi);
        cx.notify();
    }

    fn render_toolbar_button(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_panel == Some(panel);
        let item_size = Size::Size(cx.theme().geometry.layout.global_rail_item);

        IconButton::new(
            format!("mqtt-sidebar-btn-{panel:?}"),
            ObjectIcon::new(IconName::AILine),
        )
        .hit_size(item_size)
        .glyph_size(IconSize::Medium)
        .selected(is_active)
        .tooltip(t!("MqttSidebar.ai_chat").to_string())
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
            .into_any_element()
    }

    pub fn render_panel_content(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        match panel {
            SidebarPanel::AiChat => self.ai_chat_panel.clone().into_any_element(),
        }
    }
}

impl EventEmitter<MqttSidebarEvent> for MqttSidebar {}

impl Focusable for MqttSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MqttSidebar {
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
