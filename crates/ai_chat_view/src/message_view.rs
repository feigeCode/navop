use crate::card::{CardMessage, CardRegistry};
use crate::code_block::CodeBlockActionRegistry;
use crate::message_code_actions::apply_code_block_features;
use crate::message_tool_group::{
    MessageRenderItem, message_render_items, render_tool_target_group,
};
use crate::theme::{
    AgentChatTheme, resolve_agent_chat_theme, themed_html, themed_markdown, with_agent_chat_theme,
};
use crate::{
    ChatMessageUI, ChatMessageUIGeneric, ChatRole, MessageExtension, MessageVariant,
    render_reasoning_block,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ColorExt, Div, InteractiveElement, IntoElement, ParentElement, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, clipboard::Clipboard, h_flex, scroll::Scrollbar,
    v_flex,
};
use rust_i18n::t;

fn message_copy_id(message_id: &str) -> SharedString {
    SharedString::from(format!("copy-message-{message_id}"))
}

fn trim_leading_blank_lines(content: &str) -> &str {
    let mut content_start = 0;
    for line in content.split_inclusive('\n') {
        let line_content = line.trim_end_matches(['\r', '\n']);
        if !line_content.trim().is_empty() {
            break;
        }
        content_start += line.len();
    }
    &content[content_start..]
}

fn message_copy_value<E: MessageExtension>(
    message: &ChatMessageUIGeneric<E>,
) -> Option<SharedString> {
    let copyable = matches!(
        (&message.role, &message.variant),
        (ChatRole::User, MessageVariant::Text) | (ChatRole::Assistant, MessageVariant::Text)
    );
    if !copyable {
        return None;
    }

    let content = trim_leading_blank_lines(&message.content);
    (!content.is_empty()).then(|| content.to_owned().into())
}

fn render_message_copy<E: MessageExtension>(
    message: &ChatMessageUIGeneric<E>,
) -> Option<AnyElement> {
    let value = message_copy_value(message)?;
    Some(
        div()
            .debug_selector(|| "ai-chat-message-copy".to_string())
            .flex_shrink_0()
            .child(
                Clipboard::new(message_copy_id(&message.id))
                    .value(value)
                    .tooltip(t!("AgentUi.copy_message").to_string()),
            )
            .into_any_element(),
    )
}

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
    let bubble_width = user_message_bubble_width(&msg.content);
    let plain_text_html = user_plain_text_html(&msg.content);
    let copy = render_message_copy(msg);

    h_flex()
        .debug_selector(|| "ai-chat-user-row".to_string())
        .w_full()
        .min_w_0()
        .gap_1()
        .justify_end()
        .when_some(copy, |this, copy| this.child(copy))
        .child(
            div()
                .debug_selector(|| "ai-chat-user-bubble".to_string())
                .w(bubble_width)
                .max_w(px(820.0))
                .min_w_0()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(theme.accent.opacity(0.28))
                .bg(theme.accent.opacity(0.12))
                .text_color(theme.foreground)
                .child(
                    div()
                        .debug_selector(|| "ai-chat-user-plain-text".to_string())
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .child(
                            themed_html(
                                SharedString::from(format!("user-msg-{}", msg.id)),
                                plain_text_html,
                                theme,
                            )
                            .selectable(true)
                            .w_full(),
                        ),
                ),
        )
        .into_any_element()
}

fn user_plain_text_html(text: &str) -> String {
    let mut html = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                html.push_str("<br>");
            }
            '\n' => html.push_str("<br>"),
            _ => html.push(character),
        }
    }

    html
}

fn user_message_bubble_width(content: &str) -> gpui::Pixels {
    const MIN_WIDTH: f32 = 128.0;
    const MAX_WIDTH: f32 = 820.0;
    const HORIZONTAL_PADDING_AND_SLACK: f32 = 32.0;
    const ASCII_CHAR_WIDTH: f32 = 8.0;
    const WIDE_CHAR_WIDTH: f32 = 16.0;
    const WIDTH_SAFETY_FACTOR: f32 = 1.08;

    let content_width = content
        .lines()
        .map(|line| {
            line.chars()
                .map(|character| {
                    if character.is_ascii() {
                        ASCII_CHAR_WIDTH
                    } else {
                        WIDE_CHAR_WIDTH
                    }
                })
                .sum::<f32>()
        })
        .fold(0.0, f32::max);

    px(
        (content_width * WIDTH_SAFETY_FACTOR + HORIZONTAL_PADDING_AND_SLACK)
            .clamp(MIN_WIDTH, MAX_WIDTH),
    )
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
    let copy = render_message_copy(msg);

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
                })
                .when_some(copy, |this, copy| {
                    this.child(h_flex().w_full().justify_start().child(copy))
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
                .child(t!("AgentUi.thinking").to_string()),
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
        return v_flex()
            .w_full()
            .min_w_0()
            .items_stretch()
            .overflow_hidden()
            .text_color(theme.foreground)
            .child(element)
            .into_any_element();
    }
    render_placeholder_themed(
        t!("AgentUi.unregistered_card", kind = kind).to_string(),
        theme,
    )
}

fn render_sql_result_placeholder(theme: &AgentChatTheme) -> AnyElement {
    render_placeholder_themed(t!("AgentUi.sql_card_renderer_required").to_string(), theme)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_plain_text_html_escapes_markup_and_preserves_lines() {
        assert_eq!(
            "**保持** &lt;tag&gt; &amp; &quot;quoted&quot; &#39;value&#39;<br>下一行",
            user_plain_text_html("**保持** <tag> & \"quoted\" 'value'\n下一行")
        );
    }

    #[test]
    fn message_copy_uses_stable_id_and_raw_content() {
        let message = ChatMessageUI::user("**raw** <tag>\nnext").with_id("message-id");

        assert_eq!(message_copy_id(&message.id), "copy-message-message-id");
        assert_eq!(
            Some(SharedString::from("**raw** <tag>\nnext")),
            message_copy_value(&message)
        );
    }

    #[test]
    fn message_copy_removes_leading_blank_lines_without_losing_indentation() {
        let message = ChatMessageUI::assistant("\r\n\n  \n    indented\nnext");

        assert_eq!(
            Some(SharedString::from("    indented\nnext")),
            message_copy_value(&message)
        );
    }

    #[test]
    fn message_copy_preserves_markdown_code_block_after_leading_blank_lines() {
        let message = ChatMessageUI::assistant("\n \r\n```rust\n    let value = 1;\n```\n");

        assert_eq!(
            Some(SharedString::from("```rust\n    let value = 1;\n```\n")),
            message_copy_value(&message)
        );
    }

    #[test]
    fn message_copy_ignores_content_that_only_contains_blank_lines() {
        let message = ChatMessageUI::assistant("\r\n\n  ");

        assert_eq!(None, message_copy_value(&message));
    }

    #[test]
    fn only_user_and_assistant_text_bodies_are_copyable() {
        let assistant = ChatMessageUI::assistant("answer");
        let system = ChatMessageUI::system("system");
        let status = ChatMessageUI::status("running", false);
        let card = ChatMessageUI::card("kind", "payload");
        let user_card = ChatMessageUI::user("payload").with_variant(MessageVariant::Card {
            kind: "kind".into(),
        });

        assert_eq!(
            Some(SharedString::from("answer")),
            message_copy_value(&assistant)
        );
        assert_eq!(None, message_copy_value(&system));
        assert_eq!(None, message_copy_value(&status));
        assert_eq!(None, message_copy_value(&card));
        assert_eq!(None, message_copy_value(&user_card));
        assert_eq!(
            None,
            message_copy_value(&ChatMessageUI::assistant(String::new()))
        );
    }
}
