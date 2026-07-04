use crate::agent_cards::{TOOL_CARD, ToolCardData};
use crate::theme::AgentChatTheme;
use crate::{ChatMessageUI, MessageVariant};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};
use gpui_component::{Icon, IconName, Sizable, h_flex, v_flex};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

static TOOL_TARGET_GROUP_COLLAPSED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct ToolTarget {
    key: String,
    id: String,
    label: String,
}

pub(crate) struct ToolTargetGroup<'a> {
    id: String,
    target_id: String,
    target_label: String,
    messages: Vec<&'a ChatMessageUI>,
}

pub(crate) enum MessageRenderItem<'a> {
    Single(&'a ChatMessageUI),
    ToolTargetGroup(ToolTargetGroup<'a>),
}

impl<'a> ToolTargetGroup<'a> {
    pub(crate) fn messages(&self) -> &[&'a ChatMessageUI] {
        &self.messages
    }

    fn target_title(&self) -> String {
        if self.target_id == self.target_label {
            self.target_label.clone()
        } else {
            format!("{} · {}", self.target_label, self.target_id)
        }
    }
}

impl MessageRenderItem<'_> {
    #[cfg(test)]
    fn tool_target_group_summary(&self) -> Option<(&str, &str, usize)> {
        match self {
            MessageRenderItem::ToolTargetGroup(group) => Some((
                group.target_id.as_str(),
                group.target_label.as_str(),
                group.messages.len(),
            )),
            MessageRenderItem::Single(_) => None,
        }
    }
}

pub(crate) fn message_render_items(messages: &[ChatMessageUI]) -> Vec<MessageRenderItem<'_>> {
    let mut items = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let Some(target) = tool_message_target(&messages[index]) else {
            items.push(MessageRenderItem::Single(&messages[index]));
            index += 1;
            continue;
        };
        let mut group = ToolTargetGroup {
            id: String::new(),
            target_id: target.id,
            target_label: target.label,
            messages: vec![&messages[index]],
        };
        index += 1;
        while index < messages.len() {
            let Some(next_target) = tool_message_target(&messages[index]) else {
                break;
            };
            if next_target.key != target.key {
                break;
            }
            group.messages.push(&messages[index]);
            index += 1;
        }
        group.id = tool_target_group_id(&target.key, &group.messages);
        items.push(MessageRenderItem::ToolTargetGroup(group));
    }
    items
}

pub(crate) fn render_tool_target_group(
    group: ToolTargetGroup<'_>,
    children: Vec<AnyElement>,
    theme: &AgentChatTheme,
    _cx: &mut App,
) -> AnyElement {
    let expanded = !is_tool_target_group_collapsed(&group.id);
    let group_id = group.id.clone();
    let toggle_group_id = group_id.clone();
    let chevron = if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let hover_bg = theme.panel_hover;
    let target_title = group.target_title();

    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            h_flex()
                .id(SharedString::from(group_id))
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    toggle_tool_target_group(&toggle_group_id);
                    cx.refresh_windows();
                })
                .child(
                    Icon::new(chevron)
                        .xsmall()
                        .text_color(theme.muted_foreground)
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!(
                            "{target_title} · {} 个工具结果",
                            group.messages.len()
                        )),
                ),
        )
        .when(expanded, |this| this.children(children))
        .into_any_element()
}

fn tool_message_target(msg: &ChatMessageUI) -> Option<ToolTarget> {
    if !matches!(msg.variant, MessageVariant::Card { ref kind } if kind == TOOL_CARD) {
        return None;
    }
    let data = ToolCardData::from_json(&msg.content)?;
    let id = data.target_id.unwrap_or_default();
    let label = data.target_label.unwrap_or_default();
    let key = if id.is_empty() {
        label.clone()
    } else {
        id.clone()
    };
    if key.is_empty() {
        return None;
    }
    let display_label = if label.is_empty() { id.clone() } else { label };
    Some(ToolTarget {
        key,
        id,
        label: display_label,
    })
}

fn tool_target_group_id(target_key: &str, messages: &[&ChatMessageUI]) -> String {
    let first = messages
        .first()
        .map(|msg| msg.id.as_str())
        .unwrap_or_default();
    let last = messages
        .last()
        .map(|msg| msg.id.as_str())
        .unwrap_or_default();
    format!("agent-tool-target-group-{target_key}-{first}-{last}")
}

fn tool_target_group_collapsed_ids() -> &'static Mutex<HashSet<String>> {
    TOOL_TARGET_GROUP_COLLAPSED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_tool_target_group_collapsed(group_id: &str) -> bool {
    tool_target_group_collapsed_ids()
        .lock()
        .map(|ids| ids.contains(group_id))
        .unwrap_or(false)
}

fn toggle_tool_target_group(group_id: &str) {
    if let Ok(mut ids) = tool_target_group_collapsed_ids().lock()
        && !ids.insert(group_id.to_string())
    {
        ids.remove(group_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_message(call_id: &str, target_id: &str, target_label: &str) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CARD,
            ToolCardData {
                call_id: call_id.to_string(),
                tool_name: "ssh.exec".to_string(),
                target_id: Some(target_id.to_string()),
                target_label: Some(target_label.to_string()),
                input_summary: "df -h".to_string(),
                input_json: String::new(),
                running: false,
                success: Some(true),
                summary: "ok".to_string(),
                data_text: String::new(),
            }
            .to_json(),
        )
    }

    #[test]
    fn render_items_group_consecutive_tool_cards_by_target() {
        let messages = vec![
            tool_message("call-a-1", "ssh-a", "prod-a"),
            tool_message("call-a-2", "ssh-a", "prod-a"),
            tool_message("call-b-1", "ssh-b", "prod-b"),
        ];

        let items = message_render_items(&messages);

        assert_eq!(2, items.len());
        assert_eq!(
            Some(("ssh-a", "prod-a", 2)),
            items[0].tool_target_group_summary()
        );
        assert_eq!(
            Some(("ssh-b", "prod-b", 1)),
            items[1].tool_target_group_summary()
        );
    }

    #[test]
    fn render_items_do_not_group_tool_cards_across_text_messages() {
        let messages = vec![
            tool_message("call-a-1", "ssh-a", "prod-a"),
            ChatMessageUI::assistant("中间说明"),
            tool_message("call-a-2", "ssh-a", "prod-a"),
        ];

        let items = message_render_items(&messages);

        assert_eq!(3, items.len());
        assert_eq!(
            Some(("ssh-a", "prod-a", 1)),
            items[0].tool_target_group_summary()
        );
        assert_eq!(None, items[1].tool_target_group_summary());
        assert_eq!(
            Some(("ssh-a", "prod-a", 1)),
            items[2].tool_target_group_summary()
        );
    }
}
