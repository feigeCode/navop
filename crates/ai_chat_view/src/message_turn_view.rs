//! 轮次视图：把 [`crate::turn::TurnProjection`] 渲染成「轮次头 → 可折叠过程 → 结论 → 风险 → 轮次脚」。
//!
//! 与 `message_view.rs` 的分工：那个文件负责**单条消息**长什么样（气泡、卡片、代码块），
//! 本文件负责**消息怎么组织成一次对话**。两条渲染路径共用
//! [`crate::message_view::message_scroll_container`]，保证宽度约束不会各自漂移。
//!
//! 三条纪律：
//!
//! 1. **风险项永远展开，但排在原位。** 待审批卡片、失败的工具/子代理、系统提示
//!    不走折叠分支，也**不**被挪到轮次末尾 —— 否则失败卡片会一直钉在底部。
//! 2. **颜色不凭空造。** [`AgentChatTheme`] 只有中性色，语义色一律取 `cx.theme()`。
//! 3. **不虚构活动。** 折叠头的步数与工具分布全部来自真实卡片；时间缺失就不显示时长。

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt, AnyElement, App, InteractiveElement, IntoElement, ParentElement,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, div, ease_out_quint,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::ChatMessageUI;
use crate::ExpansionState;
use crate::agent_cards::compact_path;
use crate::agent_diff::FileChangeSummary;
use crate::code_block::CodeBlockActionRegistry;
use crate::message_tool_group::{message_render_items_for, render_tool_call_group};
use crate::message_view::{MessageListLayout, message_scroll_container, render_one};
use crate::theme::{AgentChatTheme, resolve_agent_chat_theme, sp};
use crate::transcript_search::TranscriptSearch;
use crate::turn::{TurnProjection, TurnTimings, breakdown_text, project_turns};

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
    /// 用户点击「回到这一轮」：把工作区恢复到该轮结束时的快照。
    RestoreTurn { turn_id: String },
    /// 用户点击本轮改动摘要里的某个文件：在审阅面板里打开这个文件的 diff。
    ///
    /// 带上 `turn_id` 是要**哪一轮**的 diff —— 点历史轮的文件时，审阅面板必须裁
    /// 那一轮的快照，而不是最近一轮的。历史恢复出来的轮次没有 turn id，给不出就
    /// 不装懂（发 `None`，由 Explorer 退到最近一轮）。
    OpenFileInReview {
        path: String,
        turn_id: Option<String>,
    },
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
    /// 可以回滚到的轮次 id（宿主查过工作区快照后注入）。
    ///
    /// `None` = 这次渲染不支持回滚，页脚的「回到这一轮」入口不出现——宁可没有，
    /// 也不给一个点了没反应的控件。
    pub restorable_turns: Option<&'a HashSet<String>>,
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
            restorable_turns: None,
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

    pub fn with_restorable_turns(mut self, turns: Option<&'a HashSet<String>>) -> Self {
        self.restorable_turns = turns;
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

    message_scroll_container(scroll_handle, context.layout, items, overlays)
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

/// 新轮次进场动效时长。
///
/// 180ms 与主题的 `duration_normal` 同量级:快到不挡事,又足够让眼睛
/// 捕到「这轮是新来的」。
const TURN_ENTRANCE_DURATION: Duration = Duration::from_millis(180);

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
        children.push(slot(render_one(
            head,
            context.code_actions,
            theme,
            window,
            cx,
        )));
    }

    if turn.has_process() {
        children.push(render_process(turn, context, theme, window, cx));
    }

    for message in &turn.answer {
        children.push(slot(render_one(
            message,
            context.code_actions,
            theme,
            window,
            cx,
        )));
    }

    if context.turn_chrome {
        if turn.head.is_some() && turn.answer.is_empty() && !turn.live && turn.risks.is_empty() {
            children.push(render_no_answer(theme));
        }
        children.push(render_turn_foot(turn, theme, context, cx));
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
        .with_animation(
            SharedString::from(format!("ai-chat-turn-enter:{}", turn.key)),
            Animation::new(TURN_ENTRANCE_DURATION).with_easing(ease_out_quint()),
            |turn, delta| turn.opacity(delta),
        )
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
                .size(sp(20.0))
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

/// 过程区当前实际渲染的消息。
///
/// 展开 = 全部过程项；收起 = 只剩「不许被折叠吞掉的项」（风险 + 仍在跑的步骤
/// + 结构标记，见 [`crate::turn::survives_collapse`]）。
/// 位置由 `process` 决定，所以两种状态下顺序都是消息的原始顺序
/// —— 失败卡片不会被挪到末尾，正在跑的步骤也不会消失。
fn visible_process_items<'a>(
    turn: &'a TurnProjection<'_>,
    expanded: bool,
) -> Vec<&'a ChatMessageUI> {
    if expanded {
        turn.process.clone()
    } else {
        turn.process
            .iter()
            .copied()
            .filter(|message| crate::turn::survives_collapse(message))
            .collect()
    }
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

    // 收起时也保留风险项：它们不参与折叠（见 `turn.rs` 的纪律 1）。
    // 位置由 `process` 决定，所以无论展开还是收起，顺序都是消息的原始顺序。
    let visible = visible_process_items(turn, expanded);
    let items = message_render_items_for(&visible);
    let children: Vec<AnyElement> = items
        .into_iter()
        .map(|item| match item {
            crate::message_tool_group::MessageRenderItem::Single(message) => {
                slot(render_one(message, context.code_actions, theme, window, cx))
            }
            crate::message_tool_group::MessageRenderItem::ToolCallGroup(group) => {
                let inner = group
                    .messages()
                    .iter()
                    .map(|message| render_one(message, context.code_actions, theme, window, cx))
                    .collect();
                slot(render_tool_call_group(group, inner, theme, cx))
            }
        })
        .collect();
    // 收起且这一轮没有风险项时，整块过程正文都不渲染（只留折叠头）。
    let body = if children.is_empty() {
        None
    } else {
        Some(
            v_flex()
                .debug_selector(|| "ai-chat-process-body".to_string())
                .w_full()
                .min_w_0()
                .gap_2()
                .children(children)
                .into_any_element(),
        )
    };

    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(header)
        .when_some(body, |this, body| this.child(body))
        .into_any_element()
}

/// 该轮是否给出「回到这一轮」入口；给出了就返回它对应的 turn id。
///
/// 两个条件缺一不可：① 这一轮有 turn id —— 历史恢复出来的轮次没有 id，
/// 无从定位快照；② 宿主明确把这一轮放进了可回滚集合（说明它真的锚到了快照）。
///
/// **拿不准就不给入口**：宁可少一个按钮，也不给一个点了没反应的控件。
///
/// 返回值的生命周期只跟 `turn_id` 走（第二个参数刻意不取名）——否则调用方会
/// 被 `restorable_turns` 的生命周期绑死。
fn restorable_turn_id<'a>(
    turn_id: Option<&'a str>,
    restorable_turns: Option<&HashSet<String>>,
) -> Option<&'a str> {
    let turn_id = turn_id?;
    restorable_turns
        .filter(|turns| turns.contains(turn_id))
        .map(|_| turn_id)
}

fn render_turn_foot(
    turn: &TurnProjection<'_>,
    theme: &AgentChatTheme,
    context: &MessageListContext<'_>,
    cx: &mut App,
) -> AnyElement {
    // 状态徽标是**可选**的：轮内已经有一条「还在跑」的状态消息时（例如
    // 「OpenCode 正在继续响应…」），页脚不再说一遍「进行中」——它俩紧挨着就是
    // 同一句话说两遍。此时徽标整块省略，页脚其余部分（回滚入口）照常。
    let status: Option<(String, gpui::Hsla)> = if let Some(pending) = turn
        .has_pending_decision()
        .then(|| t!("AgentUi.turn_state_pending_decision").to_string())
    {
        Some((pending, cx.theme().warning))
    } else if turn.live {
        (!turn.has_live_status()).then(|| {
            (
                t!("AgentUi.turn_state_running").to_string(),
                theme.muted_foreground,
            )
        })
    } else if turn.has_failure() {
        Some((
            t!("AgentUi.turn_state_failed").to_string(),
            cx.theme().danger,
        ))
    } else if turn.is_cancelled() {
        Some((
            t!("AgentUi.turn_state_cancelled").to_string(),
            theme.muted_foreground,
        ))
    } else {
        Some((
            t!("AgentUi.turn_state_done").to_string(),
            cx.theme().success,
        ))
    };

    // 只有拿到该轮快照 id 的轮次才给入口；点击后交给宿主决定怎么恢复。
    let restorable_turn = restorable_turn_id(turn.turn_id.as_deref(), context.restorable_turns);
    let changed_files = turn.changed_files();

    v_flex()
        .w_full()
        .min_w_0()
        .child(
            h_flex()
                .debug_selector(|| "ai-chat-turn-foot".to_string())
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_2()
                .pt_1()
                .when_some(status, |this, (label, color)| {
                    this.child(div().size(sp(5.0)).rounded_full().bg(color).flex_shrink_0())
                        .child(div().text_xs().text_color(color).child(label))
                })
                .child(div().flex_1())
                .when_some(restorable_turn, |this, turn_id| {
                    let on_action = context.on_action.clone();
                    // 先落成 owned：on_click 的闭包要活过这一帧，借来的 &str 撑不住。
                    let turn_id = turn_id.to_string();
                    this.child(
                        Button::new(SharedString::from(format!("restore-turn-{turn_id}")))
                            .ghost()
                            .xsmall()
                            .label(t!("AgentUi.restore_turn").to_string())
                            .debug_selector(|| "ai-chat-turn-restore".to_string())
                            .on_click(move |_, window, cx| {
                                if let Some(on_action) = on_action.as_ref() {
                                    on_action(
                                        MessageListAction::RestoreTurn {
                                            turn_id: turn_id.clone(),
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    )
                }),
        )
        .when(!changed_files.is_empty(), |this| {
            this.child(render_turn_changes(
                &changed_files,
                turn.turn_id.as_deref(),
                theme,
                context,
                cx,
            ))
        })
        .into_any_element()
}

/// 一行的改动摘要：「本轮改动 N 个文件」+ 前几个文件名(可点开) + 合计增删。
///
/// 只列前几个文件：页脚是导航，不是清单。剩下的交给审阅面板——逐个列全只会把
/// 页脚变成一堵墙。
/// 本轮的改动文件行。
///
/// `turn_id` 跟着每个文件按钮一起发出去：同一个文件在第 2 轮和第 5 轮改出来的
/// 不是同一份 diff，审阅面板得知道该裁哪一轮的快照。
fn render_turn_changes(
    changed_files: &[FileChangeSummary],
    turn_id: Option<&str>,
    theme: &AgentChatTheme,
    context: &MessageListContext<'_>,
    cx: &mut App,
) -> AnyElement {
    const MAX_LISTED: usize = 3;
    let added: u32 = changed_files.iter().map(|change| change.added).sum();
    let removed: u32 = changed_files.iter().map(|change| change.removed).sum();
    let hidden = changed_files.len().saturating_sub(MAX_LISTED);
    let on_action = context.on_action.clone();
    let turn_id = turn_id.map(str::to_owned);

    h_flex()
        .debug_selector(|| "ai-chat-turn-changes".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .pb_1()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.turn_changed_files", count = changed_files.len()).to_string()),
        )
        .children(changed_files.iter().take(MAX_LISTED).map(|change| {
            let label = compact_path(&change.path);
            let path = change.path.clone();
            let on_action = on_action.clone();
            let turn_id = turn_id.clone();
            Button::new(SharedString::from(format!("turn-change-{path}")))
                .ghost()
                .xsmall()
                .label(label)
                .debug_selector(|| "ai-chat-turn-change".to_string())
                .on_click(move |_, window, cx| {
                    if let Some(on_action) = on_action.as_ref() {
                        on_action(
                            MessageListAction::OpenFileInReview {
                                path: path.clone(),
                                turn_id: turn_id.clone(),
                            },
                            window,
                            cx,
                        );
                    }
                })
        }))
        .when(hidden > 0, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("+{hidden}")),
            )
        })
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().success)
                .child(format!("+{added}")),
        )
        .when(removed > 0, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(format!("−{removed}")),
            )
        })
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

fn render_scroll_to_latest(context: &MessageListContext<'_>, theme: &AgentChatTheme) -> AnyElement {
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
        (Some(seconds), false) => t!(
            "AgentUi.process_done_timed",
            seconds = seconds,
            count = steps
        )
        .to_string(),
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

    #[test]
    fn collapsed_process_keeps_only_risks_while_expanded_keeps_the_original_order() {
        use crate::agent_cards::{TOOL_CARD, ToolCardData};

        let tool = |call_id: &str, success: Option<bool>| {
            ChatMessageUI::card(
                TOOL_CARD,
                ToolCardData {
                    call_id: call_id.to_string(),
                    tool_name: "fs.read".to_string(),
                    action: agent_runtime::ToolAction::Read,
                    target_id: None,
                    target_label: None,
                    input_summary: String::new(),
                    input_json: String::new(),
                    running: success.is_none(),
                    success,
                    summary: String::new(),
                    data_text: String::new(),
                    file_changes: Vec::new(),
                    duration_ms: None,
                }
                .to_json(),
            )
        };
        // 失败的工具卡夹在中间：它的位置必须保持不变。
        let messages = vec![
            ChatMessageUI::user("Q1"),
            tool("ok", Some(true)),
            ChatMessageUI::status("继续响应", false),
            tool("boom", Some(false)),
        ];
        let turns = project_turns(&messages, &TurnTimings::new());

        let expanded: Vec<String> = visible_process_items(&turns[0], true)
            .into_iter()
            .map(|message| message.id.clone())
            .collect();
        assert_eq!(3, expanded.len(), "展开时过程项齐全");
        assert_eq!(
            messages[1].id, expanded[0],
            "展开时第一条就是原始顺序里的第一条"
        );

        let collapsed: Vec<String> = visible_process_items(&turns[0], false)
            .into_iter()
            .map(|message| message.id.clone())
            .collect();
        assert_eq!(
            vec![messages[2].id.clone(), messages[3].id.clone()],
            collapsed,
            "收起时只留「不许被折叠吞掉」的项：仍在跑的状态行 + 失败的工具卡"
        );

        // 已完成、且无风险的步骤在收起时让位 —— 否则折叠就等于没折叠。
        assert!(
            !collapsed.contains(&messages[1].id),
            "成功的工具卡属于可折叠的历史步骤"
        );
    }

    #[test]
    fn a_live_status_line_suppresses_the_duplicate_running_badge() {
        // 三处「还在跑」曾同时出现：轮内状态行、页脚「进行中」、列表末尾「执行中…」。
        // 状态行本身就是指示，页脚据此省略徽标，避免同义重复。
        let mut with_status = vec![
            ChatMessageUI::user("Q1"),
            ChatMessageUI::status("OpenCode 正在继续响应…", false),
        ];
        let turns = project_turns(&with_status, &TurnTimings::new());
        assert!(turns[0].live, "状态行未完成 ⇒ 本轮仍在进行");
        assert!(turns[0].has_live_status(), "页脚据此不重复显示「进行中」");

        with_status[1] = ChatMessageUI::status("已结束", true);
        let turns = project_turns(&with_status, &TurnTimings::new());
        assert!(
            !turns[0].has_live_status(),
            "状态行完成后，页脚恢复显示轮次状态"
        );
    }

    #[test]
    fn restore_entry_requires_both_a_turn_id_and_a_confirmed_snapshot() {
        let known = HashSet::from(["turn-1".to_string()]);

        assert_eq!(
            Some("turn-1"),
            restorable_turn_id(Some("turn-1"), Some(&known)),
            "有 id 且宿主确认锚过快照 ⇒ 给入口"
        );
        assert_eq!(
            None,
            restorable_turn_id(Some("turn-2"), Some(&known)),
            "宿主没确认过这一轮 ⇒ 不给入口，点了也没快照可回"
        );
        assert_eq!(
            None,
            restorable_turn_id(None, Some(&known)),
            "历史恢复出来的轮次没有 id ⇒ 定位不到快照，不给入口"
        );
        assert_eq!(
            None,
            restorable_turn_id(Some("turn-1"), None),
            "整页不支持回滚 ⇒ 不给入口"
        );
        assert_eq!(
            None,
            restorable_turn_id(Some("turn-1"), Some(&HashSet::new())),
            "可回滚集合为空 ⇒ 不给入口"
        );
    }
}
