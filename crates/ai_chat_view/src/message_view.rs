//! `ai_chat_view` 消息列表渲染。
//!
//! 保持业务逻辑在 transcript / runtime 层,这里只负责 Agent/Chat 视图内的输出排版。
//! `MessageVariant::Card { kind }` 通过 [`CardRegistry`] 按 `kind` 分发到注册的卡片,
//! 未注册时回退占位符。

use crate::card::{CardMessage, CardRegistry};
use crate::code_block::CodeBlockActionRegistry;
use crate::message_code_actions::render_code_block_actions;
use crate::{ChatMessageUI, ChatMessageUIGeneric, ChatRole, MessageExtension, MessageVariant};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, scroll::Scrollbar, text::TextView, v_flex,
};

/// 把消息列表渲染为可滚动区域。
pub fn render_messages(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_code_actions(messages, scroll_handle, None, window, cx)
}

/// 把消息列表渲染为可滚动区域，并在助手代码块下方显示可用操作。
pub fn render_messages_with_code_actions(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let items: Vec<AnyElement> = messages
        .iter()
        .map(|m| render_one(m, code_actions, window, cx))
        .collect();

    div()
        .id("ai-chat-messages")
        .flex_1()
        .min_h_0()
        .w_full()
        .relative()
        .child(
            div()
                .id("ai-chat-messages-scroll")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll_handle)
                .p_4()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(920.0))
                        .mx_auto()
                        .gap_3()
                        .children(items),
                ),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(16.0))
                .child(Scrollbar::vertical(scroll_handle)),
        )
        .into_any_element()
}

/// 渲染单条消息（通用路由）。
fn render_one(
    msg: &ChatMessageUI,
    code_actions: Option<&CodeBlockActionRegistry>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match msg.role {
        ChatRole::User => render_user_message(msg, cx),
        ChatRole::System => render_system_message(msg, cx),
        ChatRole::Assistant => match &msg.variant {
            MessageVariant::Status { title, is_done } => {
                render_status_message(msg, title, *is_done, cx)
            }
            MessageVariant::Text => render_assistant_text_with_code_actions(msg, code_actions, cx),
            MessageVariant::SqlResult => render_sql_result_placeholder(cx),
            MessageVariant::Card { kind } => render_card(msg, kind, window, cx),
        },
    }
}

/// 渲染用户消息:右侧紧凑气泡,避免占满整行。
pub fn render_user_message<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .justify_end()
        .child(
            div()
                .max_w(px(720.0))
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().primary.opacity(0.22))
                .bg(cx.theme().primary.opacity(0.1))
                .text_color(cx.theme().foreground)
                .child(
                    TextView::markdown(
                        SharedString::from(format!("user-msg-{}", msg.id)),
                        msg.content.clone(),
                    )
                    .selectable(true),
                ),
        )
        .into_any_element()
}

/// 渲染系统消息:作为轻量提示行,不参与正文视觉重量。
pub fn render_system_message<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .py_1()
        .child(
            div()
                .max_w(px(760.0))
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .bg(cx.theme().muted.opacity(0.45))
                .child(msg.content.clone()),
        )
        .into_any_element()
}

/// 渲染状态消息:Codex 风格的行内进度,不做大卡片。
pub fn render_status_message<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    title: &str,
    is_done: bool,
    cx: &App,
) -> AnyElement {
    let (icon, color) = if is_done {
        (IconName::Check, cx.theme().success)
    } else {
        (IconName::Loader, cx.theme().muted_foreground)
    };

    h_flex()
        .id(SharedString::from(msg.id.clone()))
        .w_full()
        .items_center()
        .gap_2()
        .py_1()
        .child(Icon::new(icon).with_size(Size::Small).text_color(color))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(title.to_string()),
        )
        .into_any_element()
}

/// 渲染助手文本消息（markdown）。
pub fn render_assistant_text<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    cx: &App,
) -> AnyElement {
    render_assistant_text_with_code_actions(msg, None, cx)
}

fn render_assistant_text_with_code_actions<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    code_actions: Option<&CodeBlockActionRegistry>,
    cx: &App,
) -> AnyElement {
    if msg.is_streaming && msg.content.is_empty() {
        return render_thinking(cx);
    }
    div()
        .w_full()
        .max_w(px(820.0))
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    div().w_full().px_1().py_1().child(
                        TextView::markdown(
                            SharedString::from(format!("ai-msg-{}", msg.id)),
                            msg.content.clone(),
                        )
                        .selectable(true),
                    ),
                )
                .when_some(
                    code_actions.and_then(|r| render_code_block_actions(msg, r, cx)),
                    |this, actions| this.child(actions),
                ),
        )
        .into_any_element()
}

pub fn render_thinking(cx: &App) -> AnyElement {
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("思考中..."),
        )
        .into_any_element()
}

/// 渲染卡片消息：查全局注册表，未注册回退占位符。
fn render_card(msg: &ChatMessageUI, kind: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let card_msg = CardMessage {
        id: &msg.id,
        kind,
        content: &msg.content,
        is_streaming: msg.is_streaming,
    };
    if let Some(element) = CardRegistry::render_global(&card_msg, window, cx) {
        return div().w_full().child(element).into_any_element();
    }
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("[未注册卡片: {kind}]")),
        )
        .into_any_element()
}

fn render_sql_result_placeholder(cx: &App) -> AnyElement {
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("[SQL 结果卡片需要业务渲染器]"),
        )
        .into_any_element()
}
