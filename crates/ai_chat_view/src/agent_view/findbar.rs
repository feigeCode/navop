//! 会话内搜索栏（findbar）：覆盖在消息列表右上角的浮层。
//!
//! 纯逻辑在 [`crate::transcript_search`]，这里只负责把它渲染出来、把交互接回去。
//!
//! 三条纪律：
//!
//! 1. **命中以轮次为单位**。Markdown 渲染后没有可靠的字符区间映射，所以这里
//!    只把整条轮次染色（`TurnHighlight`），不假装标出精确子串。
//! 2. **跳转复用 GPUI 自己的最小滚动策略**。滚动容器的直接子元素就是轮次，
//!    所以 `ScrollHandle::scroll_to_item` 天然索引得到；这里不维护第二套坐标换算。
//! 3. **不抢焦点**（除了打开时主动聚焦搜索框）。改查询、跳转都不碰 composer。
//!
//! `Enter` / `Shift+Enter` 走 `InputEvent::PressEnter`，不额外绑键；
//! `escape` 由 `find_shortcut::init` 绑在 [`AI_CHAT_FINDBAR_CONTEXT`] 上关闭本栏。

use gpui::{
    AnyElement, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    px,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
};
use one_assets::IconName;
use rust_i18n::t;

use super::*;
use crate::find_shortcut::{
    AI_CHAT_FINDBAR_CONTEXT, CloseTranscriptFind, FindNextInTranscript, FindPreviousInTranscript,
};

/// findbar 的固定宽度：够放查询 + 三个按钮，又不会盖住半屏正文。
const FINDBAR_WIDTH: f32 = 320.0;

impl AgentChatView {
    /// 会话内搜索栏。未打开时返回 `None`（不留空浮层）。
    pub(super) fn render_findbar(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.findbar_open {
            return None;
        }

        let active = self.search.is_active();
        let has_matches = active && self.search.total() > 0;
        let case_sensitive = self.search.case_sensitive();
        let counter = findbar_counter(&self.search, active);

        let case_button = Button::new("ai-chat-findbar-case")
            .debug_selector(|| "ai-chat-findbar-case".to_string())
            .xsmall()
            .compact()
            .text()
            .icon(IconName::CaseSensitive)
            .selected(case_sensitive)
            .toggled(case_sensitive)
            .on_click(cx.listener(|this, _, _, cx| {
                let next = !this.search.case_sensitive();
                this.search.set_case_sensitive(next, &this.transcript.messages);
                // 改大小写规则后命中集变了：把视图拉回当前命中，否则高亮停在旧位置。
                this.jump_to_current_search_hit();
                cx.notify();
            }));

        // `Input` 自身不吃 `debug_selector`，包一层拿测试锚点。
        let query_input = div()
            .debug_selector(|| "ai-chat-findbar-input".to_string())
            .flex_1()
            .min_w_0()
            .child(
                Input::new(&self.findbar_input)
                    .small()
                    .w_full()
                    .focus_bordered(false)
                    .suffix(case_button),
            );

        let nav_button = |selector: &'static str,
                          icon: IconName,
                          enabled: bool,
                          forward: bool,
                          cx: &Context<Self>| {
            Button::new(SharedString::from(selector))
                .debug_selector(move || selector.to_string())
                .xsmall()
                .ghost()
                .icon(icon)
                .disabled(!enabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.step_search(forward, cx);
                }))
        };

        Some(
            div()
                .id("ai-chat-findbar")
                .debug_selector(|| "ai-chat-findbar".to_string())
                .key_context(AI_CHAT_FINDBAR_CONTEXT)
                .on_action(cx.listener(|this, _: &CloseTranscriptFind, window, cx| {
                    this.close_findbar(window, cx);
                }))
                .on_action(cx.listener(|this, _: &FindNextInTranscript, _, cx| {
                    this.step_search(true, cx);
                }))
                .on_action(cx.listener(|this, _: &FindPreviousInTranscript, _, cx| {
                    this.step_search(false, cx);
                }))
                .absolute()
                .top_2()
                .right_4()
                .w(px(FINDBAR_WIDTH))
                .occlude()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(cx.theme().tokens.popover)
                .p_2()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_1()
                        .child(query_input)
                        .child(nav_button(
                            "ai-chat-findbar-prev",
                            IconName::ChevronUp,
                            has_matches,
                            false,
                            cx,
                        ))
                        .child(nav_button(
                            "ai-chat-findbar-next",
                            IconName::ChevronDown,
                            has_matches,
                            true,
                            cx,
                        ))
                        .child(
                            div()
                                .debug_selector(|| "ai-chat-findbar-counter".to_string())
                                .flex_shrink_0()
                                .min_w(px(44.0))
                                .text_xs()
                                .text_color(if has_matches {
                                    theme.muted_foreground
                                } else {
                                    cx.theme().warning
                                })
                                .child(counter),
                        )
                        .child(
                            Button::new("ai-chat-findbar-close")
                                .debug_selector(|| "ai-chat-findbar-close".to_string())
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_findbar(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .pt_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if self.search.truncated() {
                            t!(
                                "AgentUi.findbar_truncated",
                                max = crate::transcript_search::MAX_SEARCH_HITS
                            )
                            .to_string()
                        } else {
                            t!("AgentUi.findbar_hint").to_string()
                        }),
                )
                .into_any_element(),
        )
    }
}

/// 计数文案：`3 / 12`（命中被上限截断时补 `+`）。
///
/// 空查询显示空串而不是 `0 / 0`——「还没搜」和「搜了没命中」是两件事，
/// 后者由调用方用警示色单独表达。
fn findbar_counter(search: &crate::transcript_search::TranscriptSearch, active: bool) -> String {
    if !active {
        return String::new();
    }
    match search.progress() {
        Some((current, total)) => {
            if search.truncated() {
                format!("{current} / {total}+")
            } else {
                format!("{current} / {total}")
            }
        }
        None => t!("AgentUi.findbar_no_results").to_string(),
    }
}

impl AgentChatView {
    /// 打开 findbar 并把焦点交给搜索框。已打开时只重新聚焦（不重置查询）。
    pub(super) fn open_findbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.findbar_open = true;
        // 打开时先按当前正文刷新一次，避免展示的是上一次的命中集。
        self.refresh_search(cx);
        let focus_handle = self.findbar_input.read(cx).focus_handle(cx);
        focus_handle.focus(window, cx);
        cx.notify();
    }

    /// 关闭 findbar 并清空命中高亮。查询本身保留在输入框里，便于再次打开继续用。
    ///
    /// 关闭时要把焦点**还给 composer**。搜索框是浮层，浮层一消失，焦点就落在了一个已被
    /// 移出树的元素上（等价于没有焦点），键盘事件再没人接——用户下一次按键，包括再按一次
    /// `Cmd/Ctrl+F`，都会静默失效。
    pub(super) fn close_findbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.findbar_open {
            return;
        }
        self.findbar_open = false;
        self.search.clear();
        let input = self.input.clone();
        input.update(cx, |input, cx| input.focus_input(window, cx));
        cx.notify();
    }

    /// 打开/关闭切换。工具栏按钮入口（按钮本身有选中态，切换语义成立）。
    pub(super) fn toggle_findbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.findbar_open {
            self.close_findbar(window, cx);
        } else {
            self.open_findbar(window, cx);
        }
    }

    /// 上一条 / 下一条命中，并滚动到对应轮次。
    pub(super) fn step_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !self.findbar_open || !self.search.is_active() {
            return;
        }
        self.search.step(forward);
        self.jump_to_current_search_hit();
        cx.notify();
    }

    /// 按当前正文重算命中；正文没变就不重算。
    ///
    /// **无副作用版本**，供 `render()` 直接调用：渲染期间不能再 `notify`，
    /// 否则「重算 → 通知 → 再渲染」会自己转起来。需要通知的调用点用
    /// [`Self::refresh_search`]。
    ///
    /// 另有一条刻意的推迟：**流式输出期间不重算**。一轮回答会以 token 为粒度
    /// 反复推高修订号，每个 token 都全量重扫一遍正文是纯浪费；搜索框里的命中
    /// 停在「上一个稳定状态」，等本轮结束（`is_running` 落下）时一次性补齐。
    /// 用户在流式期间改查询不受影响——那条路径由输入事件直接触发重算。
    pub(super) fn refresh_search_if_stale(&mut self) -> bool {
        let revision = self.transcript.revision();
        if revision == self.search_revision {
            return false;
        }
        if self.is_running {
            return false;
        }
        self.search_revision = revision;
        if self.search.is_active() {
            self.search.refresh(&self.transcript.messages);
        }
        true
    }

    /// 重算命中并在确有变化时通知。快捷键 / 打开路径用。
    pub(super) fn refresh_search(&mut self, cx: &mut Context<Self>) {
        if self.refresh_search_if_stale() {
            cx.notify();
        }
    }

    /// 滚动到当前命中所在的轮次。
    ///
    /// 用 `scroll_to_top_of_item` 而不是 `scroll_to_item`：搜索跳转的意图是
    /// 「从这一轮的顶部开始读」，最小滚动策略在命中块本来就在视口内时什么都不做，
    /// 用户会以为「点了没反应」。
    pub(super) fn jump_to_current_search_hit(&self) {
        if let Some(turn_index) = self.search.current_turn_index() {
            self.scroll_handle.scroll_to_top_of_item(turn_index);
        }
    }
}
