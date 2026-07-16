use crate::card::{CardMessage, CardRegistry};
use crate::code_block::CodeBlockActionRegistry;
use crate::message_code_actions::apply_code_block_features;
use crate::message_tool_group::{
    MessageRenderItem, message_render_items, render_tool_target_group,
};
use crate::theme::{
    AgentChatTheme, resolve_agent_chat_theme, themed_markdown, with_agent_chat_theme,
};
use crate::{
    ChatMessageUI, ChatMessageUIGeneric, ChatRole, MessageExtension, MessageVariant,
    render_reasoning_block,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, scroll::Scrollbar, v_flex,
};

pub fn render_messages(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_code_actions(messages, scroll_handle, None, None, window, cx)
}

pub fn render_messages_with_code_actions(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_layout(
        messages,
        scroll_handle,
        code_actions,
        theme,
        MessageListLayout::Centered,
        window,
        cx,
    )
}

pub fn render_sidebar_messages_with_code_actions(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_layout(
        messages,
        scroll_handle,
        code_actions,
        theme,
        MessageListLayout::EdgeToEdge,
        window,
        cx,
    )
}

#[derive(Clone, Copy)]
enum MessageListLayout {
    Centered,
    EdgeToEdge,
}

fn render_messages_with_layout(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    layout: MessageListLayout,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = resolve_agent_chat_theme(theme, cx);
    let items: Vec<AnyElement> = message_render_items(messages)
        .into_iter()
        .map(|item| {
            div()
                .debug_selector(|| "ai-chat-message-slot".to_string())
                .min_w_0()
                .self_stretch()
                .flex_shrink_0()
                .child(render_item(item, code_actions, &theme, window, cx))
                .into_any_element()
        })
        .collect();

    div()
        .id("ai-chat-messages")
        .debug_selector(|| "ai-chat-messages".to_string())
        .flex_1()
        .min_h_0()
        .min_w_0()
        .w_full()
        .relative()
        .overflow_hidden()
        .child(
            div()
                .id("ai-chat-messages-scroll")
                .debug_selector(|| "ai-chat-messages-scroll".to_string())
                .size_full()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(scroll_handle)
                .p_4()
                .child(message_column(layout).gap_3().children(items)),
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

fn message_column(layout: MessageListLayout) -> Div {
    let column = v_flex()
        .debug_selector(|| "ai-chat-message-column".to_string())
        .w_full()
        .min_w_0()
        .items_stretch();
    match layout {
        MessageListLayout::Centered => column.max_w(px(920.0)).mx_auto(),
        MessageListLayout::EdgeToEdge => column,
    }
}

fn render_item(
    item: MessageRenderItem<'_>,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match item {
        MessageRenderItem::Single(msg) => render_one(msg, code_actions, theme, window, cx),
        MessageRenderItem::ToolTargetGroup(group) => {
            let children = group
                .messages()
                .iter()
                .map(|msg| render_one(msg, code_actions, theme, window, cx))
                .collect();
            render_tool_target_group(group, children, theme, cx)
        }
    }
}

fn render_one(
    msg: &ChatMessageUI,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match msg.role {
        ChatRole::User => render_user_message_themed(msg, theme),
        ChatRole::System => render_system_message_themed(msg, theme),
        ChatRole::Assistant => match &msg.variant {
            MessageVariant::Status { title, is_done } => {
                render_status_message_themed(msg, title, *is_done, theme, cx)
            }
            MessageVariant::Text => {
                render_assistant_text_with_code_actions(msg, code_actions, Some(theme), window, cx)
            }
            MessageVariant::SqlResult => render_sql_result_placeholder(&theme),
            MessageVariant::Card { kind } => render_card(msg, kind, theme, window, cx),
        },
    }
}

pub fn render_user_message<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    cx: &App,
) -> AnyElement {
    let theme = AgentChatTheme::from_app(cx);
    render_user_message_themed(msg, &theme)
}

fn render_user_message_themed<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    theme: &AgentChatTheme,
) -> AnyElement {
    h_flex()
        .debug_selector(|| "ai-chat-user-row".to_string())
        .w_full()
        .min_w_0()
        .justify_end()
        .child(
            div()
                .debug_selector(|| "ai-chat-user-bubble".to_string())
                .w_full()
                .max_w(px(720.0))
                .min_w_0()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(theme.accent.opacity(0.28))
                .bg(theme.accent.opacity(0.12))
                .text_color(theme.foreground)
                .child(
                    themed_markdown(
                        SharedString::from(format!("user-msg-{}", msg.id)),
                        msg.content.clone(),
                        theme,
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
    let theme = AgentChatTheme::from_app(cx);
    render_system_message_themed(msg, &theme)
}

fn render_system_message_themed<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    theme: &AgentChatTheme,
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
                .text_color(theme.muted_foreground)
                .bg(theme.muted.opacity(0.45))
                .child(
                    themed_markdown(
                        SharedString::from(format!("system-msg-{}", msg.id)),
                        msg.content.clone(),
                        theme,
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
    let theme = AgentChatTheme::from_app(cx);
    render_status_message_themed(msg, title, is_done, &theme, cx)
}

fn render_status_message_themed<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    title: &str,
    is_done: bool,
    theme: &AgentChatTheme,
    cx: &App,
) -> AnyElement {
    let (icon, color) = if is_done {
        (IconName::Check, cx.theme().success)
    } else {
        (IconName::Loader, theme.muted_foreground)
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
                .text_color(theme.muted_foreground)
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
    render_assistant_text_with_code_actions(msg, None, None, window, cx)
}

fn render_assistant_text_with_code_actions<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = resolve_agent_chat_theme(theme, cx);
    if msg.is_streaming && msg.content.is_empty() && msg.reasoning_content.is_empty() {
        return render_thinking_themed(&theme);
    }
    let text = themed_markdown(
        SharedString::from(format!("ai-msg-{}", msg.id)),
        msg.content.clone(),
        &theme,
    )
    .selectable(true);
    let text = apply_code_block_features(text, code_actions, Some(&theme), msg.is_streaming);

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
                    this.child(render_reasoning_block(msg, Some(&theme), window, cx))
                })
                .when(!msg.content.is_empty(), |this| {
                    this.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .px_1()
                            .py_1()
                            .text_color(theme.foreground)
                            .child(text),
                    )
                }),
        )
        .into_any_element()
}

pub fn render_thinking(cx: &App) -> AnyElement {
    let theme = AgentChatTheme::from_app(cx);
    render_thinking_themed(&theme)
}

fn render_thinking_themed(theme: &AgentChatTheme) -> AnyElement {
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("思考中..."),
        )
        .into_any_element()
}

fn render_card(
    msg: &ChatMessageUI,
    kind: &str,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let card_msg = CardMessage {
        id: &msg.id,
        kind,
        content: &msg.content,
        is_streaming: msg.is_streaming,
    };
    if let Some(element) =
        with_agent_chat_theme(theme, || CardRegistry::render_global(&card_msg, window, cx))
    {
        return div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .text_color(theme.foreground)
            .child(element)
            .into_any_element();
    }
    render_placeholder_themed(format!("[未注册卡片: {kind}]"), theme)
}

fn render_sql_result_placeholder(theme: &AgentChatTheme) -> AnyElement {
    render_placeholder_themed("[SQL 结果卡片需要业务渲染器]", theme)
}

fn render_placeholder_themed(text: impl Into<String>, theme: &AgentChatTheme) -> AnyElement {
    div()
        .w_full()
        .py_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text.into()),
        )
        .into_any_element()
}
