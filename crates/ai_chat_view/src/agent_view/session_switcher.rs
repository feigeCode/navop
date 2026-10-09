//! 最近会话切换器。
//!
//! 交互模型：`cmd-shift-j`（可自定义）打开 → `tab` / 方向键移动高亮 →
//! `enter`、点击卡片或**松开修饰键**提交 → `escape` / 点击背景取消。
//! 列表是「最近访问过的会话」，上限 [`MAX_SWITCHER_SESSIONS`]，最新在前。
//!
//! 为什么不是 `ctrl-tab`：那是本应用「工作台 Tab 切换」的全局键，
//! 会话切换器抢它会变成「同一个键在面板内外干两件事」。
//! （钉死这条的测试见 `session_shortcut` 模块。）

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ModifiersChangedEvent, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::ActiveTheme;
use rust_i18n::t;

use crate::session_shortcut::{
    AI_CHAT_SESSION_SWITCHER_CONTEXT, CancelSessionSwitch, ConfirmSessionSwitch,
    CycleSessionSwitcherBackward, CycleSessionSwitcherForward, SelectFirstSessionInSwitcher,
    SelectLastSessionInSwitcher,
};
use crate::theme::sp;

use super::{AgentChatTheme, current_agent_task_title};

/// 切换器列表上限（两行卡片 × 5 列的量级）。
pub(super) const MAX_SWITCHER_SESSIONS: usize = 10;

/// 浮层底部的操作提示词条。渲染与守卫测试**共用**这两个常量：
/// 写成字面量的话，渲染里的拼写错误只有等用户看到 key 才会被发现。
const SWITCHER_HINT_KEY: &str = "AgentUi.session_switcher_hint";
/// 「当前会话」标记词条。
const SWITCHER_CURRENT_KEY: &str = "AgentUi.session_switcher_current";

/// 切换器的纯状态机；不接触 GPUI，渲染层只读 `open` / `ordered` / `highlighted`。
pub(super) struct SessionSwitcherUi {
    open: bool,
    ordered: Vec<String>,
    highlighted: usize,
    /// 最近访问过的会话，最新在前（recency 的事实源）。
    recent: Vec<String>,
    /// 打开那一刻的当前会话。提交时若视图的当前会话已经不是它，说明期间
    /// 发生了别的切换（侧栏点击、归档……），本次提交作废——不然会把一个
    /// 用户已经离开的会话又拽回来。
    origin_session: Option<String>,
}

impl SessionSwitcherUi {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            ordered: Vec::new(),
            highlighted: 0,
            recent: Vec::new(),
            origin_session: None,
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// 切换器当前展示的会话列表（`open` 时才有意义）。
    pub(super) fn ordered(&self) -> &[String] {
        &self.ordered
    }

    /// 当前高亮下标（越界视为 0，防御渲染期列表被 `remove` 缩短）。
    pub(super) fn highlighted(&self) -> usize {
        self.ordered.len().min(self.highlighted)
    }

    pub(super) fn highlighted_session(&self) -> Option<&str> {
        self.ordered.get(self.highlighted()).map(String::as_str)
    }

    /// 记录一次会话访问（recency 前插、去重）。
    pub(super) fn record_access(&mut self, uid: &str) {
        self.recent.retain(|recent| recent != uid);
        self.recent.insert(0, uid.to_string());
    }

    /// 会话消失（删除 / 归档）时：recency、列表、高亮一起清掉它。
    pub(super) fn remove(&mut self, uid: &str) {
        self.recent.retain(|recent| recent != uid);
        let highlighted_id = self.ordered.get(self.highlighted()).cloned();
        self.ordered.retain(|candidate| candidate != uid);
        if highlighted_id.as_deref() == Some(uid) {
            // 高亮行被删掉：原地留在最近的邻居上。
            self.highlighted = self.highlighted().min(self.ordered.len().saturating_sub(1));
        } else {
            self.highlighted = self.highlighted();
        }
    }

    /// 打开切换器：按 recency 快照出有序列表，初始高亮「上一个会话」。
    ///
    /// `known` 是合法会话 id 集合（侧栏可见 + 当前会话）。列表为空则不开
    /// （返回 `false`）——孤身一人没什么可切的。
    pub(super) fn open(&mut self, current: Option<&str>, known: &dyn Fn(&str) -> bool) -> bool {
        let ordered = ordered_session_ids(current, &self.recent, known, MAX_SWITCHER_SESSIONS);
        if ordered.is_empty() {
            return false;
        }
        self.ordered = ordered;
        self.highlighted = self
            .ordered
            .iter()
            .position(|candidate| Some(candidate.as_str()) != current)
            .unwrap_or(0);
        self.origin_session = current.map(str::to_string);
        self.open = true;
        true
    }

    /// 关闭并清空瞬态（列表 / 高亮 / 来源）。
    pub(super) fn dismiss(&mut self) {
        self.open = false;
        self.ordered.clear();
        self.highlighted = 0;
        self.origin_session = None;
    }

    /// 打开时的来源会话；提交守卫用（见 [`Self::open`]）。
    pub(super) fn origin_session(&self) -> Option<&str> {
        self.origin_session.as_deref()
    }

    /// 移动高亮；`reverse` 反向。列表为空时 no-op。
    pub(super) fn cycle_highlight(&mut self, reverse: bool) {
        let len = self.ordered.len();
        if len == 0 {
            return;
        }
        let index = self.highlighted();
        self.highlighted = if reverse {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        };
    }

    /// 高亮到首 / 末。
    pub(super) fn select_first(&mut self) {
        if !self.ordered.is_empty() {
            self.highlighted = 0;
        }
    }

    pub(super) fn select_last(&mut self) {
        if !self.ordered.is_empty() {
            self.highlighted = self.ordered.len() - 1;
        }
    }

    /// 点卡片直接指定高亮（指针悬停跟随）。
    pub(super) fn highlight_session(&mut self, uid: &str) {
        if let Some(index) = self.ordered.iter().position(|candidate| candidate == uid) {
            self.highlighted = index;
        }
    }
}

/// 组装切换器列表：当前会话 → recency 顺序，只保留 `known` 里仍然存在
/// 的会话，去重，截到 `cap`。
pub(super) fn ordered_session_ids(
    current: Option<&str>,
    recent: &[String],
    known: &dyn Fn(&str) -> bool,
    cap: usize,
) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::with_capacity(cap.min(1 + recent.len()));
    let mut push = |uid: &str| {
        if ordered.len() < cap && known(uid) && !ordered.iter().any(|existing| existing == uid) {
            ordered.push(uid.to_string());
        }
    };
    if let Some(current) = current {
        push(current);
    }
    for uid in recent {
        push(uid);
    }
    ordered
}

/// 「松开修饰键提交」的判定：macOS 看松开 `cmd`，其他平台看松开 `ctrl`
/// （与 [`crate::session_shortcut`] 的平台默认键一致）。
fn switcher_modifier_released(modifiers: &gpui::Modifiers) -> bool {
    // `secondary()` 本身就是「macOS 上是 cmd、其他平台是 ctrl」，
    // 与 `session_shortcut` 的平台默认键同源，不必再手写 cfg。
    !modifiers.secondary()
}

/// 渲染一行卡片需要的数据（先物化，避免渲染闭包里借用视图）。
struct SwitcherRow {
    uid: String,
    title: SharedString,
    time: String,
    running: bool,
    is_current: bool,
    highlighted: bool,
}

impl super::AgentChatView {
    /// 打开 / 关闭切换器。
    pub(super) fn toggle_session_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_switcher.is_open() {
            self.cancel_session_switcher(window, cx);
        } else {
            self.open_session_switcher(window, cx);
        }
    }

    /// 打开切换器。合法会话集合 = 侧栏可见 + 当前会话（空白会话也能被切回）。
    fn open_session_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_sessions(cx);
        let known: std::collections::HashSet<String> = self
            .sessions
            .iter()
            .map(|summary| summary.id.clone())
            .collect();
        let current = self.current_session.clone();
        let opened = self
            .session_switcher
            .open(Some(current.as_str()), &|uid: &str| {
                uid == current || known.contains(uid)
            });
        if opened {
            let focus = self.session_switcher_focus.clone();
            window.focus(&focus, cx);
            cx.notify();
        }
    }

    /// 切换器内移动高亮（tab / 方向键）。
    pub(super) fn cycle_session_switcher(&mut self, reverse: bool, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        self.session_switcher.cycle_highlight(reverse);
        cx.notify();
    }

    pub(super) fn select_first_session_in_switcher(&mut self, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        self.session_switcher.select_first();
        cx.notify();
    }

    pub(super) fn select_last_session_in_switcher(&mut self, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        self.session_switcher.select_last();
        cx.notify();
    }

    /// 指针悬停跟随高亮。
    fn highlight_session_in_switcher(&mut self, uid: &str, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        self.session_switcher.highlight_session(uid);
        cx.notify();
    }

    /// 提交：切到高亮会话。打开期间若用户已经用别的途径切走，放弃本次提交。
    pub(super) fn confirm_session_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        let target = self
            .session_switcher
            .highlighted_session()
            .map(str::to_string);
        let origin = self.session_switcher.origin_session().map(str::to_string);
        self.session_switcher.dismiss();
        if origin.is_some_and(|origin| origin != self.current_session) {
            // 期间切走了：不要再把用户拉回一个已离开的会话。
            return;
        }
        if let Some(target) = target.filter(|target| *target != self.current_session) {
            self.switch_session(&target, cx);
        }
        self.input
            .update(cx, |input, cx| input.focus_input(window, cx));
    }

    pub(super) fn cancel_session_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.session_switcher.is_open() {
            return;
        }
        self.session_switcher.dismiss();
        self.input
            .update(cx, |input, cx| input.focus_input(window, cx));
        cx.notify();
    }

    /// 切换器 overlay。未打开时返回 `None`，不留空层。
    pub(super) fn render_session_switcher(
        &mut self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.session_switcher.is_open() {
            return None;
        }

        let rows: Vec<SwitcherRow> = self
            .session_switcher
            .ordered()
            .iter()
            .enumerate()
            .map(|(index, uid)| {
                let summary = self.sessions.iter().find(|summary| summary.id == *uid);
                SwitcherRow {
                    uid: uid.clone(),
                    title: summary
                        .map(|summary| summary.name.clone())
                        .unwrap_or_else(|| SharedString::from(current_agent_task_title())),
                    time: summary
                        .map(|summary| crate::session_sidebar::format_timestamp(summary.updated_at))
                        .unwrap_or_default(),
                    running: self.running_sessions.contains(uid),
                    is_current: *uid == self.current_session,
                    highlighted: index == self.session_switcher.highlighted(),
                }
            })
            .collect();

        let row_elements: Vec<AnyElement> = rows
            .into_iter()
            .map(|row| {
                let uid = row.uid.clone();
                let hover_uid = row.uid.clone();
                let highlighted = row.highlighted;
                div()
                    .id(SharedString::from(format!(
                        "agent-session-switcher-row-{}",
                        row.uid
                    )))
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(highlighted, |row| row.bg(theme.selection_background()))
                    .when(!highlighted, |row| {
                        row.hover(|row| row.bg(theme.hover_background()))
                    })
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        this.highlight_session_in_switcher(&hover_uid, cx);
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.session_switcher.highlight_session(&uid);
                        this.confirm_session_switcher(window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(row.is_current, |title| {
                                title.text_color(theme.muted_foreground)
                            })
                            .child(row.title),
                    )
                    .when(row.running, |row| {
                        row.child(div().flex_none().size_2().rounded_full().bg(theme.accent))
                    })
                    .when(row.is_current, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t!(SWITCHER_CURRENT_KEY).to_string()),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(row.time),
                    )
                    .into_any_element()
            })
            .collect();

        let switcher_focus = self.session_switcher_focus.clone();
        Some(
            div()
                .id("agent-session-switcher-layer")
                .debug_selector(|| "agent-session-switcher-layer".to_string())
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_start()
                .justify_center()
                .pt(sp(96.0))
                .bg(theme.background.opacity(0.32))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.cancel_session_switcher(window, cx);
                    }),
                )
                .on_modifiers_changed(cx.listener(
                    |this, event: &ModifiersChangedEvent, window, cx| {
                        if switcher_modifier_released(&event.modifiers) {
                            this.confirm_session_switcher(window, cx);
                        }
                    },
                ))
                .child(
                    div()
                        .id("agent-session-switcher")
                        .debug_selector(|| "agent-session-switcher".to_string())
                        .key_context(AI_CHAT_SESSION_SWITCHER_CONTEXT)
                        .track_focus(&switcher_focus)
                        .w(sp(380.0))
                        .occlude()
                        .rounded_xl()
                        .border_1()
                        .border_color(theme.border)
                        .bg(cx.theme().tokens.popover)
                        .shadow_lg()
                        .p_2()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .on_action(cx.listener(|this, _: &ConfirmSessionSwitch, window, cx| {
                            this.confirm_session_switcher(window, cx);
                        }))
                        .on_action(cx.listener(|this, _: &CancelSessionSwitch, window, cx| {
                            this.cancel_session_switcher(window, cx);
                        }))
                        .on_action(cx.listener(|this, _: &CycleSessionSwitcherForward, _, cx| {
                            this.cycle_session_switcher(false, cx);
                        }))
                        .on_action(
                            cx.listener(|this, _: &CycleSessionSwitcherBackward, _, cx| {
                                this.cycle_session_switcher(true, cx);
                            }),
                        )
                        .on_action(
                            cx.listener(|this, _: &SelectFirstSessionInSwitcher, _, cx| {
                                this.select_first_session_in_switcher(cx);
                            }),
                        )
                        .on_action(cx.listener(|this, _: &SelectLastSessionInSwitcher, _, cx| {
                            this.select_last_session_in_switcher(cx);
                        }))
                        .child(
                            div()
                                .w_full()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t!(SWITCHER_HINT_KEY).to_string()),
                        )
                        .children(row_elements),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known_all(_: &str) -> bool {
        true
    }

    #[test]
    fn ordering_starts_with_the_current_session_and_follows_recency() {
        let recent = vec!["a".into(), "b".into(), "c".into()];
        let ordered = ordered_session_ids(Some("now"), &recent, &known_all, 10);
        assert_eq!(vec!["now", "a", "b", "c"], ordered);
    }

    #[test]
    fn ordering_drops_missing_sessions_and_deduplicates() {
        let recent = vec!["now".into(), "gone".into(), "a".into(), "a".into()];
        let known = |uid: &str| uid != "gone";
        let ordered = ordered_session_ids(Some("now"), &recent, &known, 10);
        assert_eq!(vec!["now", "a"], ordered);
    }

    #[test]
    fn ordering_is_capped() {
        let recent: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let ordered = ordered_session_ids(Some("current"), &recent, &known_all, 5);
        assert_eq!(5, ordered.len());
        assert_eq!("current", ordered[0]);
        assert_eq!("s3", ordered[4], "截断必须从旧的开始丢");
    }

    #[test]
    fn recency_moves_the_most_recent_visit_to_the_front() {
        let mut switcher = SessionSwitcherUi::new();
        switcher.record_access("a");
        switcher.record_access("b");
        switcher.record_access("a");
        assert_eq!(
            vec!["a".to_string(), "b".to_string()],
            switcher.recent,
            "重复访问要前插去重，不是追加"
        );
    }

    #[test]
    fn opening_highlights_the_previous_session_and_empty_list_refuses_to_open() {
        let mut switcher = SessionSwitcherUi::new();
        switcher.record_access("a");
        switcher.record_access("b");
        // 当前是 b，列表 [b, a]，初始高亮 a（上一个会话）。
        assert!(switcher.open(Some("b"), &known_all));
        assert_eq!(Some("a"), switcher.highlighted_session());

        // 只有一个会话时：高亮停在它自己。
        let mut lone = SessionSwitcherUi::new();
        lone.record_access("only");
        assert!(lone.open(Some("only"), &known_all));
        assert_eq!(Some("only"), lone.highlighted_session());

        // 没有任何已知会话：不开。
        let mut empty = SessionSwitcherUi::new();
        assert!(!empty.open(Some("x"), &|_| false));
        assert!(!empty.is_open());
    }

    #[test]
    fn cycling_wraps_in_both_directions() {
        let mut switcher = SessionSwitcherUi::new();
        for uid in ["a", "b", "c"] {
            switcher.record_access(uid);
        }
        assert!(switcher.open(Some("c"), &known_all));
        // 列表 [c, b, a]，高亮 b。
        switcher.cycle_highlight(false);
        assert_eq!(Some("a"), switcher.highlighted_session());
        switcher.cycle_highlight(false);
        assert_eq!(Some("c"), switcher.highlighted_session(), "正向到头绕回");
        switcher.cycle_highlight(true);
        assert_eq!(Some("a"), switcher.highlighted_session(), "反向绕回");
    }

    #[test]
    fn first_and_last_selection_and_pointer_highlight() {
        let mut switcher = SessionSwitcherUi::new();
        for uid in ["a", "b", "c"] {
            switcher.record_access(uid);
        }
        assert!(switcher.open(Some("c"), &known_all));
        switcher.select_last();
        assert_eq!(Some("a"), switcher.highlighted_session());
        switcher.select_first();
        assert_eq!(Some("c"), switcher.highlighted_session());
        switcher.highlight_session("b");
        assert_eq!(Some("b"), switcher.highlighted_session());
    }

    #[test]
    fn remove_prunes_recency_and_keeps_the_highlight_on_a_neighbour() {
        let mut switcher = SessionSwitcherUi::new();
        for uid in ["a", "b", "c"] {
            switcher.record_access(uid);
        }
        assert!(switcher.open(Some("c"), &known_all));
        switcher.highlight_session("b");
        switcher.remove("b");
        // 列表 [c, a]，高亮留在原下标——后一项（a）顶上来。
        assert_eq!(Some("a"), switcher.highlighted_session());
        // recency 也被清掉：重开后 b 不再出现。
        assert!(switcher.open(Some("c"), &known_all));
        assert_eq!(vec!["c".to_string(), "a".to_string()], switcher.ordered);
    }

    #[test]
    fn dismiss_clears_the_transient_state() {
        let mut switcher = SessionSwitcherUi::new();
        switcher.record_access("a");
        assert!(switcher.open(Some("a"), &known_all));
        assert_eq!(Some("a"), switcher.origin_session());
        switcher.dismiss();
        assert!(!switcher.is_open());
        assert!(switcher.ordered().is_empty());
        assert_eq!(None, switcher.origin_session());
        // recency 是持久的（跨开合），dismiss 不清它。
        assert_eq!(vec!["a".to_string()], switcher.recent);
    }

    /// 浮层上的两条文案必须有词条。缺词条时 `t!` 原样返回 key，会把内部标识
    /// （`AgentUi.session_switcher_hint`）直接画在浮层里给用户看。
    #[test]
    fn switcher_locales_resolve_without_leftover_placeholders() {
        for key in [SWITCHER_HINT_KEY, SWITCHER_CURRENT_KEY] {
            let text = t!(key).to_string();
            assert_ne!(key, text, "词条 `{key}` 在 locales/ai_chat_view.yml 里缺失");
            assert!(
                !text.contains("%{"),
                "词条 `{key}` 的占位符没被替换，实际文案：{text}"
            );
        }
    }

    /// 「松开修饰键提交」= 松开 secondary（macOS 上是 cmd，其他平台是 ctrl）。
    /// 判反了会出现「一打开切换器就立刻切换」。
    #[test]
    fn the_switcher_commits_only_after_the_secondary_modifier_is_released() {
        assert!(
            switcher_modifier_released(&gpui::Modifiers::default()),
            "一个修饰键都没按 = 已经松开了"
        );
        let held = if cfg!(target_os = "macos") {
            gpui::Modifiers {
                platform: true,
                ..Default::default()
            }
        } else {
            gpui::Modifiers {
                control: true,
                ..Default::default()
            }
        };
        assert!(
            !switcher_modifier_released(&held),
            "按住 secondary 时不该提交"
        );
        // 只按住不相干的修饰键不构成「还按着 secondary」。
        let unrelated = if cfg!(target_os = "macos") {
            gpui::Modifiers {
                control: true,
                ..Default::default()
            }
        } else {
            gpui::Modifiers {
                platform: true,
                ..Default::default()
            }
        };
        assert!(switcher_modifier_released(&unrelated));
    }
}
