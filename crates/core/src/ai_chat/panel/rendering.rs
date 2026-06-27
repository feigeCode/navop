#![allow(unused_imports)]
use gpui::{
    Context, Anchor, Corners, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IconName, Selectable, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    list::List,
    popover::Popover,
    v_flex,
};
use rust_i18n::t;

use super::super::rendering::ChatMessageRenderer;
use super::super::types::{AiChatMode, AiChatPlanBackend};
use super::AiChatPanel;
use super::AiChatPanelEvent;
use super::helpers::plan_backend_option_button;

impl AiChatPanel {
    pub(super) fn render_header(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = self.border(cx);
        let muted = self.muted(cx);
        let fg = self.foreground(cx);
        let session_list = self.session_list.clone();

        h_flex()
            .flex_shrink_0()
            .w_full()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(border)
            .bg(muted)
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(t!("AiChat.title").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("new-session")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.start_new_session(cx);
                            })),
                    )
                    .child(
                        Popover::new("history-popover")
                            .anchor(Anchor::TopRight)
                            .p_0()
                            .open(self.history_popover_open)
                            .on_open_change(cx.listener(|this, open, window, cx| {
                                this.history_popover_open = *open;
                                if *open {
                                    this.update_session_list(window, cx);
                                    this.load_history_sessions(cx);
                                }
                                cx.notify();
                            }))
                            .when_some(session_list.as_ref(), |popover, list| {
                                popover.track_focus(&list.focus_handle(cx))
                            })
                            .trigger(
                                Button::new("history")
                                    .icon(IconName::BookOpen)
                                    .ghost()
                                    .small(),
                            )
                            .when_some(session_list, |popover, list| {
                                popover.child(
                                    List::new(&list)
                                        .w(px(280.0))
                                        .max_h(px(350.0))
                                        .border_1()
                                        .border_color(border)
                                        .rounded(cx.theme().radius),
                                )
                            }),
                    )
                    .child(
                        Button::new("close-panel")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|_this, _event, _window, cx| {
                                cx.emit(AiChatPanelEvent::Close);
                            })),
                    ),
            )
    }

    pub(super) fn render_messages(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let code_block_actions = self.engine.code_block_actions.clone();

        div()
            .id("chat-messages-list")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.engine.scroll_handle)
            .p_4()
            .pb_8()
            .child(
                v_flex()
                    .w_full()
                    .gap_4()
                    .children(self.engine.messages.iter().map(|msg| {
                        ChatMessageRenderer::render_message(msg, &code_block_actions, window, cx)
                    })),
            )
    }

    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.ai_input_state.read(cx).value().to_string();
        if content.trim().is_empty() {
            return;
        }
        self.send_message(content, cx);
        self.ai_input_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    pub(super) fn render_input(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = self.border(cx);
        let bg = self.background(cx);
        let muted = self.muted(cx);

        v_flex()
            .flex_shrink_0()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .bg(bg)
            // 输入框
            .child(
                Input::new(&self.ai_input_state)
                    .w_full()
                    .with_size(Size::Large)
                    .bordered(false)
                    .appearance(false)
                    .bg(muted)
                    .rounded(cx.theme().radius),
            )
            // 底部工具栏
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_2()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .flex_shrink_0()
                                    .child(
                                        Button::new("ai-chat-mode-ask")
                                            .icon(IconName::Bot)
                                            .label("Ask")
                                            .ghost()
                                            .with_size(Size::Small)
                                            .selected(self.engine.mode() == AiChatMode::Ask)
                                            .on_click(cx.listener(|this, _, _window, cx| {
                                                this.set_mode(AiChatMode::Ask, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("ai-chat-mode-plan")
                                            .icon(IconName::Map)
                                            .label("Plan")
                                            .ghost()
                                            .with_size(Size::Small)
                                            .selected(self.engine.mode() == AiChatMode::Plan)
                                            .on_click(cx.listener(|this, _, _window, cx| {
                                                this.set_mode(AiChatMode::Plan, cx);
                                            })),
                                    ),
                            )
                            .when(self.engine.mode() == AiChatMode::Plan, |toolbar| {
                                toolbar.child(self.render_plan_backend_controls(cx))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.provider_select_state.render(cx)),
                            )
                            // 模型设置按钮
                            .child({
                                let settings_panel = self.settings_panel.clone();
                                Popover::new("model-settings-popover")
                                    .anchor(Anchor::BottomLeft)
                                    .trigger(
                                        Button::new("model-settings-btn")
                                            .icon(IconName::Settings)
                                            .ghost()
                                            .with_size(Size::Small),
                                    )
                                    .content(move |_state, _window, _cx| settings_panel.clone())
                            }),
                    )
                    .child(if self.can_cancel() {
                        // 加载中显示终止按钮
                        Button::new("cancel")
                            .with_size(Size::Small)
                            .danger()
                            .icon(IconName::CircleX)
                            .label(t!("AiChat.cancel").to_string())
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.cancel_current_operation(cx);
                            }))
                    } else {
                        // 正常状态显示发送按钮
                        Button::new("send")
                            .with_size(Size::Small)
                            .primary()
                            .icon(IconName::ArrowRight)
                            .label(t!("AiChat.send").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit(window, cx);
                            }))
                    }),
            )
    }

    pub(super) fn render_plan_backend_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_id = self
            .acp_agent_config
            .as_ref()
            .map(|config| config.id.to_string());
        let current_label = self.current_plan_backend_label();
        let is_local = self.plan_backend == AiChatPlanBackend::LocalRuntime;
        let is_acp = self.plan_backend == AiChatPlanBackend::AcpAgent;
        let configs = self.acp_agent_configs.clone();
        let panel = cx.entity();
        let open = self.plan_backend_popover_open;

        Popover::new("ai-chat-plan-backend-popover")
            .anchor(Anchor::BottomLeft)
            .p_0()
            .open(open)
            .on_open_change(cx.listener(|this, open, _window, cx| {
                this.plan_backend_popover_open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("ai-chat-plan-backend-selector")
                    .icon(IconName::ChevronsUpDown)
                    .label(current_label)
                    .ghost()
                    .with_size(Size::Small),
            )
            .content(move |_, _, _cx| {
                let mut items: Vec<gpui::AnyElement> = Vec::new();
                items.push(
                    plan_backend_option_button(
                        "ai-chat-plan-backend-local",
                        "Local",
                        IconName::Bot,
                        is_local,
                        {
                            let panel = panel.clone();
                            move |cx| {
                                panel.update(cx, |this, cx| {
                                    this.set_plan_backend(AiChatPlanBackend::LocalRuntime, cx);
                                });
                            }
                        },
                    )
                    .into_any_element(),
                );
                let sel = selected_id.clone();
                for config in configs.iter() {
                    let id = config.id.to_string();
                    let selected = is_acp && sel.as_deref() == Some(id.as_str());
                    items.push(
                        plan_backend_option_button(
                            format!("ai-chat-plan-backend-acp-{id}"),
                            config.name.to_string(),
                            IconName::Map,
                            selected,
                            {
                                let panel = panel.clone();
                                let config = config.clone();
                                move |cx| {
                                    panel.update(cx, |this, cx| {
                                        this.set_acp_agent_config(Some(config.clone()), cx);
                                        this.set_plan_backend(AiChatPlanBackend::AcpAgent, cx);
                                    });
                                }
                            },
                        )
                        .into_any_element(),
                    );
                }
                v_flex().w(px(260.0)).p_1().gap_1().children(items)
            })
    }
    pub(super) fn current_plan_backend_label(&self) -> String {
        match self.plan_backend {
            AiChatPlanBackend::LocalRuntime => "Local".to_string(),
            AiChatPlanBackend::AcpAgent => self
                .acp_agent_config
                .as_ref()
                .map(|config| config.name.to_string())
                .unwrap_or_else(|| "ACP".to_string()),
        }
    }
}
