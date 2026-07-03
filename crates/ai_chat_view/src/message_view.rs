use crate::card::{CardMessage, CardRegistry};
use crate::code_block::CodeBlockActionRegistry;
use crate::message_code_actions::apply_code_block_features;
use crate::message_tool_group::{
    MessageRenderItem, message_render_items, render_tool_target_group,
};
use crate::{
    ChatMessageUI, ChatMessageUIGeneric, ChatRole, MessageExtension, MessageVariant,
    render_reasoning_block,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, scroll::Scrollbar, text::TextView, v_flex,
};

pub fn render_messages(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_code_actions(messages, scroll_handle, None, window, cx)
}

pub fn render_messages_with_code_actions(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let items: Vec<AnyElement> = message_render_items(messages)
        .into_iter()
        .map(|item| render_item(item, code_actions, window, cx))
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

fn render_item(
    item: MessageRenderItem<'_>,
    code_actions: Option<&CodeBlockActionRegistry>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match item {
        MessageRenderItem::Single(msg) => render_one(msg, code_actions, window, cx),
        MessageRenderItem::ToolTargetGroup(group) => {
            let children = group
                .messages()
                .iter()
                .map(|msg| render_one(msg, code_actions, window, cx))
                .collect();
            render_tool_target_group(group, children, cx)
        }
    }
}

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
            MessageVariant::Text => {
                render_assistant_text_with_code_actions(msg, code_actions, window, cx)
            }
            MessageVariant::SqlResult => render_sql_result_placeholder(cx),
            MessageVariant::Card { kind } => render_card(msg, kind, window, cx),
        },
    }
}

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
                .min_w_0()
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
                .w_full()
                .max_w(px(760.0))
                .min_w_0()
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .bg(cx.theme().muted.opacity(0.45))
                .child(
                    TextView::markdown(
                        SharedString::from(format!("system-msg-{}", msg.id)),
                        msg.content.clone(),
                    )
                    .text_xs()
                    .selectable(true),
                ),
        )
        .into_any_element()
}

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
        .min_w_0()
        .items_center()
        .gap_2()
        .py_1()
        .child(
            Icon::new(icon)
                .with_size(Size::Small)
                .text_color(color)
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(title.to_string()),
        )
        .into_any_element()
}

pub fn render_assistant_text<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_assistant_text_with_code_actions(msg, None, window, cx)
}

fn render_assistant_text_with_code_actions<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    code_actions: Option<&CodeBlockActionRegistry>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if msg.is_streaming && msg.content.is_empty() && msg.reasoning_content.is_empty() {
        return render_thinking(cx);
    }
    let text = TextView::markdown(
        SharedString::from(format!("ai-msg-{}", msg.id)),
        msg.content.clone(),
    )
    .selectable(true);
    let text = apply_code_block_features(text, code_actions);

    div()
        .w_full()
        .max_w(px(820.0))
        .min_w_0()
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .when(!msg.reasoning_content.is_empty(), |this| {
                    this.child(render_reasoning_block(msg, window, cx))
                })
                .when(!msg.content.is_empty(), |this| {
                    this.child(div().w_full().min_w_0().px_1().py_1().child(text))
                }),
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
    render_placeholder(format!("[未注册卡片: {kind}]"), cx)
}

fn render_sql_result_placeholder(cx: &App) -> AnyElement {
    render_placeholder("[SQL 结果卡片需要业务渲染器]", cx)
}

fn render_placeholder(text: impl Into<String>, cx: &App) -> AnyElement {
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text.into()),
        )
        .into_any_element()
}
