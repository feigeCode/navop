//! 子代理详情面板：在右侧面板里回放某只子代理的完整推理。
//!
//! # 为什么是「回放」而不是「实时」
//!
//! 外部 agent（实测 OpenCode 1.18.30）跑子代理时用一条**独立子会话**，但子会话的
//! 通知在协议层就被挡掉了：agent 的事件订阅只转发它自己登记过的会话，而子会话
//! 只在 `session/load` 之后才登记得上。更关键的是，子会话的地址只在子代理**结束时**
//! 才随工具结果一起送到（运行态的 `tool_call_update` 不带 `raw_output`）。
//!
//! 所以「运行中逐字实时」是上游协议限制，navop 单方面做不到；能确定做到的是
//! **跑完之后完整回放**：拿到子会话地址 → `session/load` → agent 把整段历史（含全部
//! `reasoning` 推理块）重放回来。这个面板显示的就是这段回放，边到边渲染，
//! 体验上仍是「点开就能看着它一点点铺出来」。
//!
//! # 数据从哪来
//!
//! 面板自己不持有转录，而是每次渲染从 [`DefaultAgentChatPanel`] 现取一份快照。
//! 原因是 GPUI 的借用规则：`entity.read(cx)` 借出的切片会一直占着 `cx`，
//! 没法再交给需要 `&mut App` 的 [`render_messages`]。取快照时用转录的**修订号**
//! 做去重，避免光鼠标划过就把整段转录克隆一遍（工具卡片里躺着几十 KB 的 JSON）。

use gpui::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, SharedString, Styled as _, Subscription, Window, div,
};
use gpui_component::{ActiveTheme as _, Icon, Sizable as _, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::acp::detail_session_id_for;
use crate::default_panel::{DefaultAgentChatPanel, DefaultAgentChatPanelEvent};
use crate::message::ChatMessageUI;
use crate::message_view::render_messages;

/// 详情面板当前指向哪条子代理。
///
/// 这是**面板自己的**视图状态：目标从哪来、切到哪，都由面板订阅
/// [`DefaultAgentChatPanelEvent::SubagentDetailRequested`] 决定。放在这里而不是
/// 聊天视图里，是为了不制造第二份副本——两份状态迟早会对不上。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentDetailTarget {
    /// 子会话的协议 id。
    pub acp_session_id: String,
    /// 卡片标题；面板头部展示。
    pub title: String,
}

impl SubagentDetailTarget {
    /// 详情会话的内置 id（转录的键）。
    pub fn detail_session_id(&self) -> String {
        detail_session_id_for(&self.acp_session_id)
    }
}

/// 子代理详情面板。
pub struct SubagentDetailPanel {
    /// 内层聊天面板；转录的权威副本在它那里。
    chat: Entity<DefaultAgentChatPanel>,
    /// 当前看着哪条子代理。`None` = 还没被要求打开过任何一条。
    target: Option<SubagentDetailTarget>,
    /// 上次取快照时的状态令牌（转录修订号, 是否有失败文案）。
    ///
    /// 令牌相同就不重新取：否则面板每次重绘都要把整段转录克隆一遍，而面板里
    /// 躺着成百上千条消息（工具卡片各带一份 JSON）。鼠标划过都会触发重绘。
    cached_token: Option<(u64, bool)>,
    cached_messages: Vec<ChatMessageUI>,
    cached_error: Option<String>,
    scroll_handle: ScrollHandle,
    /// 内层面板事件；目标切换与内容更新都靠它。
    _chat_events: Subscription,
}

impl SubagentDetailPanel {
    pub fn new(chat: Entity<DefaultAgentChatPanel>, cx: &mut Context<Self>) -> Self {
        // 面板自己认领「要看哪条子代理」：宿主只负责把它切到前台，不必再回填一次
        // 目标——那是同一份状态的第二个副本，迟早对不上。
        let chat_events = cx.subscribe(&chat, |this, _, event: &DefaultAgentChatPanelEvent, cx| {
            match event {
                DefaultAgentChatPanelEvent::SubagentDetailRequested {
                    acp_session_id,
                    title,
                } => {
                    this.target = Some(SubagentDetailTarget {
                        acp_session_id: acp_session_id.clone(),
                        title: title.clone(),
                    });
                    // 换了目标，缓存必须作废，否则会先渲染上一条子代理的内容。
                    this.cached_token = None;
                    this.cached_messages.clear();
                    this.cached_error = None;
                    this.scroll_handle.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
                    cx.notify();
                }
                DefaultAgentChatPanelEvent::SubagentDetailUpdated { detail_session_id } => {
                    // 只关心自己这条；别的子代理在后台回放不该让面板动。
                    if this
                        .target
                        .as_ref()
                        .is_some_and(|target| target.detail_session_id() == *detail_session_id)
                    {
                        cx.notify();
                    }
                }
                _ => {}
            }
        });
        Self {
            chat,
            target: None,
            cached_token: None,
            cached_messages: Vec::new(),
            cached_error: None,
            scroll_handle: ScrollHandle::new(),
            _chat_events: chat_events,
        }
    }

    /// 如果转录变了，重新取一份快照。
    fn refresh_snapshot(&mut self, cx: &App) {
        let Some(target) = self.target.as_ref() else {
            return;
        };
        let detail_session_id = target.detail_session_id();
        let Some((revision, has_error, messages, error)) =
            self.chat.read(cx).subagent_detail_snapshot(&detail_session_id, cx)
        else {
            return;
        };
        let token = (revision, has_error);
        if self.cached_token == Some(token) {
            return;
        }
        self.cached_token = Some(token);
        self.cached_messages = messages;
        self.cached_error = error;
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title: SharedString = self
            .target
            .as_ref()
            .map(|target| SharedString::from(target.title.clone()))
            .unwrap_or_else(|| t!("Workbench.panel_subagent").into());
        div()
            .debug_selector(|| "subagent-detail-header".to_string())
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("AgentUi.subagent_detail_hint").to_string()),
                    ),
            )
    }

    fn render_placeholder(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let (icon, text) = if self.target.is_none() {
            (IconName::AILine, t!("AgentUi.subagent_detail_empty").to_string())
        } else if let Some(error) = self.cached_error.clone() {
            (IconName::TriangleAlert, error)
        } else {
            (
                IconName::LoaderCircle,
                t!("AgentUi.subagent_detail_loading").to_string(),
            )
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(
                Icon::new(icon)
                    .text_color(theme.muted_foreground)
                    .with_size(gpui_component::Size::Large),
            )
            .child(
                div()
                    .max_w_full()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(text),
            )
            .into_any_element()
    }
}

impl Render for SubagentDetailPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_snapshot(cx);

        // 背景色要先取成 `Copy` 的颜色值：`cx.theme()` 借出的主题会一直占着 `cx`，
        // 而 `render_messages` 要的是 `&mut App`。
        let background = cx.theme().background;
        let body = if self.cached_messages.is_empty() {
            self.render_placeholder(cx)
        } else {
            render_messages(&self.cached_messages, &self.scroll_handle, window, cx)
        };

        v_flex()
            .debug_selector(|| "subagent-detail-panel".to_string())
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(background)
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
    }
}
