//! 工具调用的行内分组。
//!
//! **一块 = 一次连续调用**:从第一张工具卡开始,直到出现非工具消息为止。读一个
//! 文件、改一处、再跑一条命令,本来就是一件事的连续步骤;按「动作 × 目录」切块
//! 只会把同一件事拆开,让读者自己去拼。
//!
//! 块头按**类别计数**说话:「改 6 个文件,读 4 个文件,3 个命令 +173 −8」——先说
//! 做了什么、各做了几次,合计增删跟在后面;逐条的细节在下面每一行里,不在块头。
//!
//! 折叠态默认跟着状态走:**还在跑就展开**(正在发生的事没理由藏起来),**跑完就
//! 收起来**(块头那句话就是这一块的索引)。用户一旦手动展开 / 折叠,这个选择就
//! 覆盖默认,直到条目按上限淘汰。

use crate::agent_cards::{TOOL_CARD, ToolCardData, diff_stat_chips, file_change_totals};
use crate::theme::AgentChatTheme;
use crate::{ChatMessageUI, MessageVariant};
use agent_runtime::ToolAction;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};
use gpui_component::{Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// 手动折叠态覆盖表的上限。key 含消息 id，历史裁剪后旧条目永不复用，只增不减
/// 会随会话无限增长。超出后按最旧优先淘汰；上限内用户手动的选择不受影响。
const MAX_TOOL_GROUP_TOGGLES: usize = 512;

/// 用户对一个块的手动覆盖。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupToggle {
    Expanded,
    Collapsed,
}

static TOOL_GROUP_TOGGLES: OnceLock<Mutex<VecDeque<(String, GroupToggle)>>> = OnceLock::new();

/// 一个块内的类别计数。
///
/// `other` 按工具名分开计数:协议没声明类别的调用,块头照实说「3 次 ssh.exec」,
/// 而不是编一个动词把它们混成一件不存在的事。
#[derive(Default)]
struct ToolCallTally {
    edit: usize,
    delete: usize,
    moved: usize,
    read: usize,
    search: usize,
    fetch: usize,
    execute: usize,
    think: usize,
    switch_mode: usize,
    /// 未声明类别且工具有名:工具名 → 次数。
    other: BTreeMap<String, usize>,
    /// 未声明类别且工具名也为空。
    unnamed: usize,
}

impl ToolCallTally {
    fn add(&mut self, data: &ToolCardData) {
        match data.action {
            ToolAction::Edit => self.edit += 1,
            ToolAction::Delete => self.delete += 1,
            ToolAction::Move => self.moved += 1,
            ToolAction::Read => self.read += 1,
            ToolAction::Search => self.search += 1,
            ToolAction::Fetch => self.fetch += 1,
            ToolAction::Execute => self.execute += 1,
            ToolAction::Think => self.think += 1,
            ToolAction::SwitchMode => self.switch_mode += 1,
            ToolAction::Other => {
                let name = data.tool_name.trim();
                if name.is_empty() {
                    self.unnamed += 1;
                } else {
                    *self.other.entry(name.to_string()).or_default() += 1;
                }
            }
        }
    }
}

/// 一个活动块:连续的若干次工具调用。
pub(crate) struct ToolCallGroup<'a> {
    id: String,
    tally: ToolCallTally,
    /// 块内改动合计(所有卡片求和,不按文件去重:这里说的是「这一块动了多少行」)。
    added: u32,
    removed: u32,
    /// 组内是否还有未完成的调用。
    running: bool,
    messages: Vec<&'a ChatMessageUI>,
}

pub(crate) enum MessageRenderItem<'a> {
    Single(&'a ChatMessageUI),
    ToolCallGroup(ToolCallGroup<'a>),
}

impl<'a> ToolCallGroup<'a> {
    fn new(msg: &'a ChatMessageUI) -> Self {
        let mut group = Self {
            id: String::new(),
            tally: ToolCallTally::default(),
            added: 0,
            removed: 0,
            running: false,
            messages: Vec::new(),
        };
        group.push(msg);
        group
    }

    fn push(&mut self, msg: &'a ChatMessageUI) {
        self.messages.push(msg);
        let Some(data) = ToolCardData::from_json(&msg.content) else {
            return;
        };
        self.running |= data.running;
        self.tally.add(&data);
        if let Some((added, removed)) = file_change_totals(&data.file_changes) {
            self.added += added;
            self.removed += removed;
        }
    }

    pub(crate) fn messages(&self) -> &[&'a ChatMessageUI] {
        &self.messages
    }

    /// 块头文案:各类别各做了几次,按「先改动、再读取、再检索、最后执行」排。
    ///
    /// 顺序固定而不是按出现顺序:同一个块今天和明天的读法应该一样,不然每次都要
    /// 重新扫一遍才知道「改了没有」。
    fn header_text(&self) -> String {
        let tally = &self.tally;
        let mut parts: Vec<String> = Vec::new();
        if tally.edit > 0 {
            parts.push(t!("AgentUi.tool_block_edit", count = tally.edit).to_string());
        }
        if tally.delete > 0 {
            parts.push(t!("AgentUi.tool_block_delete", count = tally.delete).to_string());
        }
        if tally.moved > 0 {
            parts.push(t!("AgentUi.tool_block_move", count = tally.moved).to_string());
        }
        if tally.read > 0 {
            parts.push(t!("AgentUi.tool_block_read", count = tally.read).to_string());
        }
        if tally.search > 0 {
            parts.push(t!("AgentUi.tool_block_search", count = tally.search).to_string());
        }
        if tally.fetch > 0 {
            parts.push(t!("AgentUi.tool_block_fetch", count = tally.fetch).to_string());
        }
        if tally.execute > 0 {
            parts.push(t!("AgentUi.tool_block_execute", count = tally.execute).to_string());
        }
        if tally.think > 0 {
            parts.push(t!("AgentUi.tool_block_think", count = tally.think).to_string());
        }
        if tally.switch_mode > 0 {
            parts.push(t!("AgentUi.tool_block_switch_mode", count = tally.switch_mode).to_string());
        }
        if tally.unnamed > 0 {
            parts.push(t!("AgentUi.tool_block_invocations", count = tally.unnamed).to_string());
        }
        for (name, count) in &tally.other {
            parts.push(t!("AgentUi.tool_block_other", count = *count, name = name).to_string());
        }
        parts.join(&t!("AgentUi.tool_block_separator"))
    }
}

impl MessageRenderItem<'_> {
    /// 列表渲染用的稳定身份:单条消息用 `msg.id`,工具块用块首个
    /// 消息的 id。
    ///
    /// 调用方（`message_view`）拿它给元素挂 `id` 与进场动画——
    /// 流式追加时 id 不变,动画就不会每帧重放。
    pub(crate) fn element_id(&self) -> SharedString {
        match self {
            MessageRenderItem::Single(msg) => SharedString::from(msg.id.clone()),
            MessageRenderItem::ToolCallGroup(group) => SharedString::from(group.id.clone()),
        }
    }

    #[cfg(test)]
    fn group(&self) -> Option<&ToolCallGroup<'_>> {
        match self {
            MessageRenderItem::ToolCallGroup(group) => Some(group),
            MessageRenderItem::Single(_) => None,
        }
    }
}

pub(crate) fn message_render_items(messages: &[ChatMessageUI]) -> Vec<MessageRenderItem<'_>> {
    let refs: Vec<&ChatMessageUI> = messages.iter().collect();
    message_render_items_for(&refs)
}

/// 与 [`message_render_items`] 相同，但直接消费引用切片。
///
/// 轮次投影已经把消息按轮次切成 `Vec<&ChatMessageUI>`，这里避免为此再拷贝一份。
pub(crate) fn message_render_items_for<'a>(
    messages: &[&'a ChatMessageUI],
) -> Vec<MessageRenderItem<'a>> {
    let mut items = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        if !is_tool_card(messages[index]) {
            items.push(MessageRenderItem::Single(messages[index]));
            index += 1;
            continue;
        }
        let mut group = ToolCallGroup::new(messages[index]);
        // 块 id 只用**第一条**消息的 id:块是边跑边长的,用最后一条会让 id 每来一次
        // 调用就变一次,用户的手动折叠状态会因此丢掉。
        group.id = format!("agent-tool-group-{}", messages[index].id);
        index += 1;
        while index < messages.len() && is_tool_card(messages[index]) {
            group.push(messages[index]);
            index += 1;
        }
        items.push(MessageRenderItem::ToolCallGroup(group));
    }
    items
}

pub(crate) fn render_tool_call_group(
    group: ToolCallGroup<'_>,
    children: Vec<AnyElement>,
    theme: &AgentChatTheme,
    cx: &mut App,
) -> AnyElement {
    let expanded = is_tool_call_group_expanded(&group.id, group.running);
    let group_id = group.id.clone();
    let toggle_group_id = group_id.clone();
    let running = group.running;
    let label = group.header_text();
    let stats = diff_stat_chips(group.added, group.removed, cx);
    let chevron = if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let hover_bg = theme.panel_hover;
    let muted = theme.muted_foreground;

    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .id(SharedString::from(group_id))
                .debug_selector(|| "agent-tool-group-head".to_string())
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_1()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    toggle_tool_call_group(&toggle_group_id, expanded);
                    cx.refresh_windows();
                })
                .child(
                    Icon::new(IconName::SquareTerminal)
                        .mono()
                        .xsmall()
                        .text_color(muted)
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(label),
                )
                .when(running, |this| {
                    // 还在跑就给一个点:块头那句话在两种状态下都成立,这个点表明
                    // 它现在还在往前走(而不是「已经这样了」)。
                    this.child(div().flex_shrink_0().text_xs().text_color(muted).child("●"))
                })
                .children(stats.into_iter().map(|(text, color)| {
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color)
                        .child(text)
                }))
                .child(
                    Icon::new(chevron)
                        .xsmall()
                        .text_color(muted)
                        .flex_shrink_0(),
                ),
        )
        .when(expanded, |this| {
            // 子行缩进一级:块头说「做了什么」,下面的行说「具体哪几次」,层级要
            // 看得出来,不然块头看起来像是另一条调用。
            this.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_0()
                    .pl_4()
                    .children(children),
            )
        })
        .into_any_element()
}

/// 一条消息是不是工具卡。
fn is_tool_card(msg: &ChatMessageUI) -> bool {
    matches!(msg.variant, MessageVariant::Card { ref kind } if kind == TOOL_CARD)
}

fn tool_group_toggles() -> &'static Mutex<VecDeque<(String, GroupToggle)>> {
    TOOL_GROUP_TOGGLES.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn tool_group_toggle(group_id: &str) -> Option<GroupToggle> {
    tool_group_toggles()
        .lock()
        .ok()?
        .iter()
        .find(|(id, _)| id == group_id)
        .map(|(_, toggle)| *toggle)
}

/// 展开与否:用户手动选择优先,否则**跑着的展开、跑完的收起**。
fn is_tool_call_group_expanded(group_id: &str, running: bool) -> bool {
    match tool_group_toggle(group_id) {
        Some(GroupToggle::Expanded) => true,
        Some(GroupToggle::Collapsed) => false,
        None => running,
    }
}

/// 记下与**此刻显示的状态**相反的选择。
///
/// `showing` 由调用方传入(它刚算过默认态),所以「跑着的块点一下收起来」和
/// 「跑完的块点一下展开」都只需要一次取反,不必在这里再猜一遍默认。
fn toggle_tool_call_group(group_id: &str, showing: bool) {
    let next = if showing {
        GroupToggle::Collapsed
    } else {
        GroupToggle::Expanded
    };
    let Ok(mut toggles) = tool_group_toggles().lock() else {
        return;
    };
    toggles.retain(|(id, _)| id != group_id);
    toggles.push_back((group_id.to_string(), next));
    while toggles.len() > MAX_TOOL_GROUP_TOGGLES {
        toggles.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_diff::FileChangeSummary;

    fn tool_message_with(
        call_id: &str,
        action: ToolAction,
        tool_name: &str,
        input_summary: &str,
        file_changes: Option<FileChangeSummary>,
        running: bool,
    ) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CARD,
            ToolCardData {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                action,
                target_id: None,
                target_label: None,
                input_summary: input_summary.to_string(),
                input_json: String::new(),
                running,
                success: (!running).then_some(true),
                summary: "ok".to_string(),
                data_text: String::new(),
                file_changes: file_changes.into_iter().collect(),
                duration_ms: None,
            }
            .to_json(),
        )
    }

    fn exec_message(call_id: &str, command: &str) -> ChatMessageUI {
        tool_message_with(
            call_id,
            ToolAction::Execute,
            "ssh.exec",
            command,
            None,
            false,
        )
    }

    fn edit_message(call_id: &str, path: &str) -> ChatMessageUI {
        tool_message_with(
            call_id,
            ToolAction::Edit,
            "write_file",
            "",
            Some(FileChangeSummary::from_texts(path, Some("a\n"), "b\n")),
            false,
        )
    }

    fn read_message(call_id: &str, path: &str) -> ChatMessageUI {
        tool_message_with(call_id, ToolAction::Read, "read_file", path, None, false)
    }

    #[test]
    fn consecutive_tool_cards_form_one_block_whatever_they_do() {
        // 这条正是行为变更本身:读、改、执行混在一起也是一块。
        let messages = vec![
            read_message("call-1", "/repo/src/lib.rs"),
            edit_message("call-2", "/repo/src/lib.rs"),
            exec_message("call-3", "cargo test"),
            exec_message("call-4", "cargo test --lib"),
        ];

        let items = message_render_items(&messages);

        assert_eq!(1, items.len());
        let group = items[0].group().expect("group");
        assert_eq!(4, group.messages.len());
    }

    #[test]
    fn a_non_tool_message_closes_the_block() {
        let messages = vec![
            exec_message("call-1", "df -h"),
            ChatMessageUI::assistant("中间说明"),
            exec_message("call-2", "uptime"),
        ];

        let items = message_render_items(&messages);

        assert_eq!(3, items.len());
        assert_eq!(1, items[0].group().expect("group").messages.len());
        assert!(items[1].group().is_none());
        assert_eq!(1, items[2].group().expect("group").messages.len());
    }

    #[test]
    fn header_counts_each_category_in_a_fixed_order() {
        let messages = vec![
            exec_message("call-1", "df -h"),
            read_message("call-2", "/repo/src/lib.rs"),
            edit_message("call-3", "/repo/src/lib.rs"),
            tool_message_with(
                "call-4",
                ToolAction::Search,
                "grep",
                "tool_row",
                None,
                false,
            ),
        ];

        let items = message_render_items(&messages);
        let label = items[0].group().expect("group").header_text();

        assert_eq!(
            t!("AgentUi.tool_block_edit", count = 1).to_string()
                + &t!("AgentUi.tool_block_separator").to_string()
                + &t!("AgentUi.tool_block_read", count = 1).to_string()
                + &t!("AgentUi.tool_block_separator").to_string()
                + &t!("AgentUi.tool_block_search", count = 1).to_string()
                + &t!("AgentUi.tool_block_separator").to_string()
                + &t!("AgentUi.tool_block_execute", count = 1).to_string(),
            label
        );
    }

    #[test]
    fn block_header_sums_the_file_changes() {
        let messages = vec![
            edit_message("call-1", "/repo/src/lib.rs"),
            edit_message("call-2", "/repo/src/other.rs"),
        ];

        let items = message_render_items(&messages);
        let group = items[0].group().expect("group");

        assert_eq!(2, group.added);
        assert_eq!(2, group.removed);
    }

    #[test]
    fn undeclared_tools_are_counted_by_name_and_never_given_a_verb() {
        let messages = vec![
            tool_message_with("call-1", ToolAction::Other, "echo", "hi", None, false),
            tool_message_with("call-2", ToolAction::Other, "echo", "ho", None, false),
            tool_message_with("call-3", ToolAction::Other, "", "", None, false),
        ];

        let items = message_render_items(&messages);
        let label = items[0].group().expect("group").header_text();

        assert!(label.contains("echo"), "{label}");
        assert!(label.contains('2'), "{label}");
        assert!(
            label.contains(&t!("AgentUi.tool_block_invocations", count = 1).to_string()),
            "{label}"
        );
        // 没有声明就没有动词:块头不该出现任何「读取 / 编辑」字样。
        assert!(
            !label.contains(&t!("AgentUi.action_read").to_string()),
            "{label}"
        );
        assert!(
            !label.contains(&t!("AgentUi.action_edit").to_string()),
            "{label}"
        );
    }

    #[test]
    fn a_running_member_makes_the_whole_block_running() {
        let messages = vec![
            tool_message_with("call-1", ToolAction::Read, "read_file", "a.rs", None, false),
            tool_message_with("call-2", ToolAction::Read, "read_file", "b.rs", None, true),
        ];

        let items = message_render_items(&messages);
        let group = items[0].group().expect("group");

        assert!(group.running);
        assert_eq!(2, group.messages.len());
    }

    #[test]
    fn blocks_default_to_expanded_while_running_and_collapsed_when_done() {
        let toggles = tool_group_toggles();
        toggles.lock().expect("lock").clear();

        assert!(is_tool_call_group_expanded("block-running", true));
        assert!(!is_tool_call_group_expanded("block-done", false));
    }

    #[test]
    fn a_manual_toggle_overrides_the_default_in_both_directions() {
        let toggles = tool_group_toggles();
        toggles.lock().expect("lock").clear();

        // 跑完的块默认收起,点一下应该展开。
        toggle_tool_call_group("block-done", false);
        assert!(is_tool_call_group_expanded("block-done", false));
        // 再点一下回到收起。
        toggle_tool_call_group("block-done", true);
        assert!(!is_tool_call_group_expanded("block-done", false));

        // 跑着的块默认展开,点一下应该收起。
        toggle_tool_call_group("block-running", true);
        assert!(!is_tool_call_group_expanded("block-running", true));

        toggles.lock().expect("lock").clear();
    }

    #[test]
    fn toggle_state_is_bounded_and_repeated_toggles_do_not_accumulate() {
        let toggles = tool_group_toggles();
        toggles.lock().expect("lock").clear();

        for index in 0..(MAX_TOOL_GROUP_TOGGLES + 10) {
            toggle_tool_call_group(&format!("block-{index}"), false);
        }

        assert_eq!(
            MAX_TOOL_GROUP_TOGGLES,
            toggles.lock().expect("lock").len(),
            "手动折叠态不能无限增长"
        );
        assert!(
            !is_tool_call_group_expanded("block-0", false),
            "最旧条目按上限淘汰,回到默认态"
        );
        assert!(
            is_tool_call_group_expanded(&format!("block-{}", MAX_TOOL_GROUP_TOGGLES + 9), false),
            "最新条目保留"
        );

        // 同一个块反复 toggle 不该堆出多条记录。
        toggles.lock().expect("lock").clear();
        for _ in 0..5 {
            toggle_tool_call_group("block-stable", false);
        }
        assert_eq!(1, toggles.lock().expect("lock").len());

        toggles.lock().expect("lock").clear();
    }

    #[test]
    fn a_group_id_survives_the_block_growing() {
        // 边跑边长时 id 必须稳定,否则跑到第 2 次调用时用户的手动折叠就丢了。
        let first = exec_message("call-1", "df -h");
        let one_messages = vec![first.clone()];
        let grown_messages = vec![first, exec_message("call-2", "free -m")];

        let one = message_render_items(&one_messages);
        let two = message_render_items(&grown_messages);

        assert_eq!(
            one[0].group().expect("group").id,
            two[0].group().expect("group").id
        );
    }
}
