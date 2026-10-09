//! 空转录时的起手态。
//!
//! 一个刚建出来的会话，转录是空的：过去这里什么都没有，整块面板是背景色，
//! 看起来像没加载出来。起手态补上「这是开始的地方」的说明，并给几句可点
//! 的起手话——把第一句话从「想想要问什么」变成「挑一句改改就发」。
//!
//! 只在**真的空着、也真的没在等**的时候画，判定见 [`should_show_empty_state`]：
//! 连接期由骨架屏负责，轮次在飞由活动指示负责，两者都不该再叠一层问候语。

use std::rc::Rc;

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::acp::AcpConnectionPhase;
use crate::message_view::{MessageListLayout, message_column};
use crate::theme::{AgentChatTheme, sp};

/// 起手态图标字号（像素）。
///
/// `Size` 只接受 `Pixels`，这里给不了 `sp()`；图标字形按像素定尺与 1px 发丝线同类。
const ICON_SIZE: f32 = 20.0;

/// 解释文案的宽度上限：一句话横跨整个面板很难读。
const HINT_MAX_WIDTH: f32 = 360.0;

/// 起手话条数。
pub(crate) const STARTER_COUNT: usize = 3;

/// 第 `index` 句起手话。
///
/// 单独成函数是为了让「点哪一句」这个下标成为唯一输入：渲染侧给 `ElementId`，
/// 点击侧回传同一个下标再取文案，两边不会各写一份字符串表。
pub(crate) fn starter_prompt(index: usize) -> String {
    match index {
        0 => t!("AgentUi.empty_starter_overview").to_string(),
        1 => t!("AgentUi.empty_starter_improve").to_string(),
        _ => t!("AgentUi.empty_starter_tests").to_string(),
    }
}

/// 该不该画起手态。
///
/// 与骨架屏是互斥的：骨架屏说明「正在等数据」，起手态说明「这儿本来就空」——
/// 同一块区域同时说两句话一定是哪里错了，所以直接问骨架屏的判定结果。
pub(crate) fn should_show_empty_state(
    phase: Option<&AcpConnectionPhase>,
    is_running: bool,
    transcript_is_empty: bool,
) -> bool {
    if !transcript_is_empty || is_running {
        return false;
    }
    // 相位只有落在 Ready（或本地：没有相位）才敢说「这儿本来就空」：
    // 连接中由骨架屏负责，失败/已关闭由错误条负责——两种情况下再垫一句
    // 问候语，要么和占位条抢同一块区域，要么让界面显得没意识到出事了。
    matches!(
        phase,
        None | Some(AcpConnectionPhase::Ready) | Some(AcpConnectionPhase::RunningTurn { .. })
    )
}

/// 起手态主体。
///
/// `on_starter` 回传被点那句的下标（见 [`starter_prompt`]）。
pub fn render_empty_state(
    theme: &AgentChatTheme,
    layout: MessageListLayout,
    compact: bool,
    on_starter: Rc<dyn Fn(usize, &mut Window, &mut App)>,
) -> AnyElement {
    let mut body = v_flex()
        .debug_selector(|| "ai-chat-empty-state".to_string())
        .items_center()
        .justify_center()
        .gap_3()
        .px_8()
        .child(
            Icon::new(IconName::Asterisk)
                .text_color(theme.accent)
                .with_size(px(ICON_SIZE)),
        )
        .child(
            div()
                .text_size(sp(15.0))
                .text_color(theme.foreground)
                .child(t!("AgentUi.empty_title").to_string()),
        );

    // 侧边栏只有一条窄栏，塞下标题已经够挤；解释和起手话留给全宽面板。
    if !compact {
        body = body
            .child(
                div()
                    .max_w(sp(HINT_MAX_WIDTH))
                    .text_center()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.empty_hint").to_string()),
            )
            .child(
                h_flex()
                    .debug_selector(|| "ai-chat-empty-starters".to_string())
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .children((0..STARTER_COUNT).map(|index| {
                        let on_starter = on_starter.clone();
                        starter_chip(theme, index, move |window, cx| {
                            on_starter(index, window, cx)
                        })
                    })),
            );
    }

    message_column(layout)
        .flex_1()
        .justify_center()
        .items_center()
        .child(body)
        .into_any_element()
}

/// 一句起手话的胶囊。
///
/// 不用按钮组件：这里要的是「像提示、不像提交」的轻样式，按下也只是填词，
/// 不是执行——点下去就把消息发出去会让人来不及改。
fn starter_chip(
    theme: &AgentChatTheme,
    index: usize,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("ai-chat-empty-starter-{index}")))
        .debug_selector(move || format!("ai-chat-empty-starter-{index}"))
        .cursor_pointer()
        .px(sp(10.0))
        .py(sp(5.0))
        .rounded_full()
        .border_1()
        .border_color(theme.border)
        .bg(theme.raised)
        .text_sm()
        .text_color(theme.muted_foreground)
        .hover(|style| {
            style
                .bg(theme.panel_hover)
                .text_color(theme.foreground)
                .border_color(theme.border_strong)
        })
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(starter_prompt(index))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Context, Modifiers, Render, TestAppContext, VisualTestContext};

    use super::*;
    use crate::message_view::MessageListLayout;

    /// 判定是纯函数，先把它钉死：起手态出现在哪、不出现在哪。
    #[test]
    fn empty_state_only_appears_when_nothing_is_pending() {
        assert!(
            should_show_empty_state(None, false, true),
            "本地空会话是最典型的场景"
        );
        assert!(
            !should_show_empty_state(None, false, false),
            "有消息就不叫空态了"
        );
        assert!(
            !should_show_empty_state(None, true, true),
            "轮次在飞时由活动指示负责"
        );
        assert!(
            !should_show_empty_state(Some(&AcpConnectionPhase::CreatingSession), false, true),
            "连接中由骨架屏负责，两句话不能抢同一块区域"
        );
        assert!(
            !should_show_empty_state(
                Some(&AcpConnectionPhase::AuthenticationRequired {
                    methods: Vec::new(),
                }),
                false,
                true
            ),
            "等鉴权时也不能说「这儿本来就空」"
        );
        assert!(
            !should_show_empty_state(Some(&AcpConnectionPhase::Closed), false, true),
            "连接已结束时由错误条负责"
        );
        assert!(
            should_show_empty_state(Some(&AcpConnectionPhase::Ready), false, true),
            "连接就绪但会话是新的，就是起手态"
        );
    }

    struct EmptyStateRoot {
        compact: bool,
        clicked: Rc<RefCell<Vec<usize>>>,
    }

    impl Render for EmptyStateRoot {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = AgentChatTheme::from_app(cx);
            let clicked = self.clicked.clone();
            render_empty_state(
                &theme,
                MessageListLayout::Centered,
                self.compact,
                Rc::new(move |index, _window, _cx| clicked.borrow_mut().push(index)),
            )
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        compact: bool,
    ) -> (Rc<RefCell<Vec<usize>>>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let clicked = Rc::new(RefCell::new(Vec::new()));
        let clicked_for_root = clicked.clone();
        let (_, cx) = cx.add_window_view(move |_window, _cx| EmptyStateRoot {
            compact,
            clicked: clicked_for_root,
        });
        (clicked, cx)
    }

    #[gpui::test]
    fn full_width_panels_offer_starters(cx: &mut TestAppContext) {
        let (clicked, cx) = mount(cx, false);

        assert!(
            cx.debug_bounds("ai-chat-empty-state").is_some(),
            "起手态要画出来"
        );
        let chip = cx
            .debug_bounds("ai-chat-empty-starter-1")
            .expect("三句起手话都要在");

        cx.simulate_click(chip.center(), Modifiers::default());

        assert_eq!(vec![1], *clicked.borrow(), "点第二句回传的是它自己的下标");
    }

    #[gpui::test]
    fn narrow_panels_keep_only_the_title(cx: &mut TestAppContext) {
        let (_, cx) = mount(cx, true);

        assert!(
            cx.debug_bounds("ai-chat-empty-state").is_some(),
            "窄栏也要有起手态"
        );
        assert!(
            cx.debug_bounds("ai-chat-empty-starters").is_none(),
            "一条窄栏塞不下一排胶囊"
        );
        assert!(cx.debug_bounds("ai-chat-empty-starter-0").is_none());
    }
}
