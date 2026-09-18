//! 轮次视图：把 [`crate::turn::TurnProjection`] 渲染成「轮次头 → 可折叠过程 → 结论 → 风险 → 轮次脚」。
//!
//! 与 `message_view.rs` 的分工：那个文件负责**单条消息**长什么样（气泡、卡片、代码块），
//! 本文件负责**消息怎么组织成一次对话**。两条渲染路径共用
//! [`crate::message_view::message_scroll_container`]，保证宽度约束不会各自漂移。
//!
//! 三条纪律：
//!
//! 1. **风险区永远展开。** 待审批卡片、失败的工具/子代理、系统提示不走折叠分支。
//! 2. **颜色不凭空造。** [`AgentChatTheme`] 只有中性色，语义色一律取 `cx.theme()`。
//! 3. **不虚构活动。** 折叠头的步数与工具分布全部来自真实卡片；时间缺失就不显示时长。

use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::ExpansionState;
use crate::code_block::CodeBlockActionRegistry;
use crate::message_tool_group::{message_render_items_for, render_tool_target_group};
use crate::message_view::{MessageListLayout, message_scroll_container, render_one};
use crate::theme::{AgentChatTheme, resolve_agent_chat_theme};
use crate::transcript_search::TranscriptSearch;
use crate::turn::{TurnProjection, TurnTimings, breakdown_text, project_turns};
use crate::{ChatMessageUI};

/// 折叠头最多列几个工具名。
const BREAKDOWN_LIMIT: usize = 2;

/// 渲染层向宿主发出的交互请求。
///
/// 用「动作 + 回调」而不是直接持有 `Entity<AgentChatView>`：渲染函数是普通自由函数，
/// 不应该知道宿主的类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageListAction {
    /// 用户切换了某个过程区的展开态。
    SetProcessExpanded { key: String, expanded: bool },
    /// 用户点击「回到最新」。
    ScrollToLatest,
}

pub type MessageListActionHandler = Rc<dyn Fn(MessageListAction, &mut Window, &mut App)>;

/// 轮次视图的渲染参数。
pub struct MessageListContext<'a> {
    pub layout: MessageListLayout,
    /// 追加在列表末尾的活动指示（"执行中…"）。
    pub activity: Option<AnyElement>,
    pub code_actions: Option<&'a CodeBlockActionRegistry>,
    pub theme: Option<&'a AgentChatTheme>,
    /// 过程区展开态覆盖表；`None` 时完全按默认规则决定。
    pub expansion: Option<&'a ExpansionState>,
    /// 轮次时间表；缺失时折叠头不显示时长。
    pub timings: Option<&'a TurnTimings>,
    pub on_action: Option<MessageListActionHandler>,
    /// 是否显示「回到最新」浮层。
    pub show_scroll_to_latest: bool,
    /// 是否渲染轮次头/脚。侧边栏紧凑模式关掉，省掉一层视觉噪音。
    pub turn_chrome: bool,
    /// 会话内搜索状态；提供后命中的轮次会高亮、当前命中更亮。
    pub search: Option<&'a TranscriptSearch>,
    /// 会话内搜索的浮层（findbar）。由宿主提供，这里只负责摆到正确位置。
    pub findbar: Option<AnyElement>,
}

impl<'a> MessageListContext<'a> {
    pub fn new(layout: MessageListLayout) -> Self {
        Self {
            layout,
            activity: None,
            code_actions: None,
            theme: None,
            expansion: None,
            timings: None,
            on_action: None,
            show_scroll_to_latest: false,
            turn_chrome: true,
            search: None,
            findbar: None,
        }
    }

    pub fn with_activity(mut self, activity: Option<AnyElement>) -> Self {
        self.activity = activity;
        self
    }

    pub fn with_code_actions(mut self, code_actions: Option<&'a CodeBlockActionRegistry>) -> Self {
        self.code_actions = code_actions;
        self
    }

    pub fn with_theme(mut self, theme: Option<&'a AgentChatTheme>) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_expansion(mut self, expansion: Option<&'a ExpansionState>) -> Self {
        self.expansion = expansion;
        self
    }

    pub fn with_timings(mut self, timings: Option<&'a TurnTimings>) -> Self {
        self.timings = timings;
        self
    }

    pub fn with_action_handler(mut self, handler: Option<MessageListActionHandler>) -> Self {
        self.on_action = handler;
        self
    }

    pub fn with_turn_chrome(mut self, turn_chrome: bool) -> Self {
        self.turn_chrome = turn_chrome;
        self
    }

    pub fn with_scroll_to_latest(mut self, show: bool) -> Self {
        self.show_scroll_to_latest = show;
        self
    }

    pub fn with_search(mut self, search: Option<&'a TranscriptSearch>) -> Self {
        self.search = search;
        self
    }

    pub fn with_findbar(mut self, findbar: Option<AnyElement>) -> Self {
        self.findbar = findbar;
        self
    }
}

/// 渲染完整消息列表（轮次视图）。
pub fn render_message_list(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    mut context: MessageListContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = resolve_agent_chat_theme(context.theme, cx);
    let empty_timings = TurnTimings::new();
    let timings = context.timings.unwrap_or(&empty_timings);
    let turns = project_turns(messages, timings);

    let mut items: Vec<AnyElement> = turns
        .iter()
        .enumerate()
        .map(|(index, turn)| {
            let highlight = context
                .search
                .map(|search| {
                    if search.is_current_turn(index) {
                        TurnHighlight::Current
                    } else if search.hits_turn(index) {
                        TurnHighlight::Match
                    } else {
                        TurnHighlight::None
                    }
                })
                .unwrap_or(TurnHighlight::None);
            render_turn(turn, highlight, &context, &theme, window, cx)
        })
        .collect();
    if let Some(activity) = context.activity.take() {
        items.push(slot(activity));
    }

    let mut overlays: Vec<AnyElement> = Vec::new();
    if context.show_scroll_to_latest {
        overlays.push(render_scroll_to_latest(&context, &theme));
    }
    if let Some(findbar) = context.findbar.take() {
        overlays.push(findbar);
    }

    message_scroll_container(
        scroll_handle,
        context.layout,
        items,
        overlays,
    )
}

/// 搜索命中在轮次上的标记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnHighlight {
    /// 未命中。
    None,
    /// 命中，但不是当前定位的那一个。
    Match,
    /// 当前定位的命中。
    Current,
}

fn slot(child: AnyElement) -> AnyElement {
    div()
        .debug_selector(|| "ai-chat-message-slot".to_string())
        .min_w_0()
        .self_stretch()
        .flex_shrink_0()
        .child(child)
        .into_any_element()
}

fn render_turn(
    turn: &TurnProjection<'_>,
    highlight: TurnHighlight,
    context: &MessageListContext<'_>,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mut children: Vec<AnyElement> = Vec::new();

    if let Some(head) = turn.head {
        if context.turn_chrome {
            children.push(render_turn_head(turn, theme));
        }
        children.push(slot(render_one(head, context.code_actions, theme, window, cx)));
    }

    if turn.has_process() {
        children.push(render_process(turn, context, theme, window, cx));
    }

    for message in &turn.answer {
        children.push(slot(render_one(message, context.code_actions, theme, window, cx)));
    }

    // 风险区不参与折叠：无条件展开渲染。
    for message in &turn.risks {
        children.push(slot(render_one(message, context.code_actions, theme, window, cx)));
    }

    if context.turn_chrome {
        if turn.head.is_some() && turn.answer.is_empty() && !turn.live && turn.risks.is_empty() {
            children.push(render_no_answer(theme));
        }
        children.push(render_turn_foot(turn, theme, cx));
    }

    v_flex()
        .debug_selector(|| "ai-chat-turn".to_string())
        .w_full()
        .min_w_0()
        .gap_2()
        .rounded_md()
        // 命中高亮是**整轮**的：正文经 Markdown 渲染后没有可靠的字符区间映射，
        // 假装能精确标出子串属于虚构。当前定位那一轮更亮，且带强调描边。
        .when(highlight == TurnHighlight::Match, |this| {
            this.bg(theme.accent.opacity(0.08))
        })
        .when(highlight == TurnHighlight::Current, |this| {
            this.bg(theme.accent.opacity(0.16))
                .border_1()
                .border_color(theme.accent)
        })
        .children(children)
        .into_any_element()
}

fn render_turn_head(turn: &TurnProjection<'_>, theme: &AgentChatTheme) -> AnyElement {
    let time = turn
        .timing
        .started_at
        .map(|at| format_clock(at))
        .filter(|value| !value.is_empty());

    h_flex()
        .debug_selector(|| "ai-chat-turn-head".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_shrink_0()
                .size(px(20.0))
                .rounded_full()
                .bg(theme.accent.opacity(0.18))
                .text_xs()
                .text_color(theme.foreground)
                .flex()
                .items_center()
                .justify_center()
                .child(t!("AgentUi.turn_role_you").to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.turn_role_you").to_string()),
        )
        .when_some(time, |this, time| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(time),
            )
        })
        .child(div().flex_1())
        .into_any_element()
}

fn render_process(
    turn: &TurnProjection<'_>,
    context: &MessageListContext<'_>,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let default_expanded = turn.default_process_expanded();
    let expanded = context
        .expansion
        .map(|state| state.is_expanded(&turn.key, default_expanded))
        .unwrap_or(default_expanded);

    let summary = process_summary(turn);
    let breakdown = breakdown_text(&turn.tool_breakdown(), BREAKDOWN_LIMIT);
    let key = turn.key.clone();
    let toggle_key = key.clone();
    let hover = theme.hover_background();
    let muted = theme.muted_foreground;
    let foreground = theme.foreground;
    let on_action = context.on_action.clone();

    let header = h_flex()
        .id(SharedString::from(format!("ai-chat-process-{}", key)))
        .debug_selector(|| "ai-chat-process-head".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(move |this| this.bg(hover))
        .child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .xsmall()
            .text_color(muted)
            .flex_shrink_0(),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if turn.live { foreground } else { muted })
                .child(summary),
        )
        .when_some(breakdown, |this, breakdown| {
            this.child(div().flex_1()).child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(breakdown),
            )
        })
        .on_click(move |_, window, cx| {
            if let Some(handler) = on_action.as_ref() {
                handler(
                    MessageListAction::SetProcessExpanded {
                        key: toggle_key.clone(),
                        expanded: !expanded,
                    },
                    window,
                    cx,
                );
            }
        });

    let body = expanded.then(|| {
        let items = message_render_items_for(&turn.process);
        let children: Vec<AnyElement> = items
            .into_iter()
            .map(|item| match item {
                crate::message_tool_group::MessageRenderItem::Single(message) => {
                    slot(render_one(message, context.code_actions, theme, window, cx))
                }
                crate::message_tool_group::MessageRenderItem::ToolTargetGroup(group) => {
                    let inner = group
                        .messages()
                        .iter()
                        .map(|message| {
                            render_one(message, context.code_actions, theme, window, cx)
                        })
                        .collect();
                    slot(render_tool_target_group(group, inner, theme, cx))
                }
            })
            .collect();
        v_flex()
            .debug_selector(|| "ai-chat-process-body".to_string())
            .w_full()
            .min_w_0()
            .gap_2()
            .children(children)
            .into_any_element()
    });

    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(header)
        .when_some(body, |this, body| this.child(body))
        .into_any_element()
}

fn render_turn_foot(turn: &TurnProjection<'_>, theme: &AgentChatTheme, cx: &mut App) -> AnyElement {
    let (label, color) = if turn.has_pending_decision() {
        (
            t!("AgentUi.turn_state_pending_decision").to_string(),
            cx.theme().warning,
        )
    } else if turn.live {
        (
            t!("AgentUi.turn_state_running").to_string(),
            theme.muted_foreground,
        )
    } else if turn.has_failure() {
        (
            t!("AgentUi.turn_state_failed").to_string(),
            cx.theme().danger,
        )
    } else {
        (
            t!("AgentUi.turn_state_done").to_string(),
            cx.theme().success,
        )
    };

    h_flex()
        .debug_selector(|| "ai-chat-turn-foot".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .pt_1()
        .child(div().size(px(5.0)).rounded_full().bg(color).flex_shrink_0())
        .child(div().text_xs().text_color(color).child(label))
        .child(div().flex_1())
        .into_any_element()
}

fn render_no_answer(theme: &AgentChatTheme) -> AnyElement {
    slot(
        div()
            .debug_selector(|| "ai-chat-turn-no-answer".to_string())
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("AgentUi.turn_no_answer").to_string())
            .into_any_element(),
    )
}

fn render_scroll_to_latest(
    context: &MessageListContext<'_>,
    theme: &AgentChatTheme,
) -> AnyElement {
    let on_action = context.on_action.clone();
    let hover = theme.hover_background();
    div()
        .absolute()
        .bottom_3()
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            h_flex()
                .id("ai-chat-scroll-to-latest")
                .debug_selector(|| "ai-chat-scroll-to-latest".to_string())
                .items_center()
                .gap_1p5()
                .px_3()
                .py_1()
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(theme.panel)
                .text_xs()
                .text_color(theme.foreground)
                .cursor_pointer()
                .shadow_md()
                .hover(move |this| this.bg(hover))
                .child(Icon::new(IconName::ArrowDown).xsmall())
                .child(t!("AgentUi.scroll_to_latest").to_string())
                .on_click(move |_, window, cx| {
                    if let Some(handler) = on_action.as_ref() {
                        handler(MessageListAction::ScrollToLatest, window, cx);
                    }
                }),
        )
        .into_any_element()
}

/// 折叠头的摘要文案：按是否有真实耗时/步数选择。
fn process_summary(turn: &TurnProjection<'_>) -> String {
    let steps = turn.step_count();
    match (turn.timing.duration_secs(), turn.live) {
        (Some(seconds), false) => {
            t!("AgentUi.process_done_timed", seconds = seconds, count = steps).to_string()
        }
        (None, false) => t!("AgentUi.process_done", count = steps).to_string(),
        (_, true) => t!("AgentUi.process_running", count = steps).to_string(),
    }
}

/// Unix 秒 → `HH:MM`（本地时区按偏移近似，分钟级展示足够）。
fn format_clock(unix_secs: i64) -> String {
    const SECS_PER_DAY: i64 = 86_400;
    let secs_of_day = unix_secs.rem_euclid(SECS_PER_DAY);
    let hours = secs_of_day / 3600;
    let minutes = (secs_of_day % 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_format_wraps_around_midnight() {
        assert_eq!("00:00", format_clock(0));
        assert_eq!("01:30", format_clock(5_400));
        assert_eq!("23:59", format_clock(86_340));
        // 跨天与负值都不应产生越界输出。
        assert_eq!("00:00", format_clock(86_400));
        assert_eq!("23:59", format_clock(-60));
    }
}
