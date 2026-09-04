use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::Textarea;
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use gpui_component::{ActiveTheme, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use rust_i18n::t;

use super::ApiTestView;
use super::tcp_state::TcpState;
use crate::websocket::{MessageDirection, TimelineEntry};

impl ApiTestView {
    pub(super) fn render_tcp_response(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let connected = self.tcp_state.state.is_connected();
        let rows = self
            .tcp_state
            .timeline
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_tcp_entry(index, entry, cx))
            .collect::<Vec<_>>();

        v_flex()
            .id("api-tcp-timeline")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.06))
            .child(self.render_tcp_header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .p_3()
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .child(
                                div()
                                    .id("api-tcp-timeline-scroll")
                                    .size_full()
                                    .overflow_scroll()
                                    .track_scroll(&self.tcp_timeline_scroll_handle)
                                    .when(rows.is_empty(), |list| {
                                        list.child(self.render_tcp_empty(cx))
                                    })
                                    .when(!rows.is_empty(), |list| {
                                        list.child(v_flex().w_full().gap_2().children(rows))
                                    }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w(px(12.))
                                    .child(
                                        Scrollbar::vertical(&self.tcp_timeline_scroll_handle)
                                            .scrollbar_show(ScrollbarShow::Always),
                                    ),
                            ),
                    ),
            )
            .child(self.render_tcp_composer(connected, cx))
    }

    fn render_tcp_header(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let (label, color) = self.tcp_status(&theme);
        let peer = self
            .tcp_state
            .peer
            .clone()
            .unwrap_or_else(|| "—".to_string());

        h_flex()
            .id("api-tcp-header")
            .w_full()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.18))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(div().size(px(8.)).flex_shrink_0().rounded_full().bg(color))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child(t!("ApiTest.tcp_messages").to_string()),
                    )
                    .child(div().text_xs().text_color(color).child(label)),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(Self::render_tcp_metric(peer, &theme))
                    .child(Self::render_tcp_metric(
                        format!(
                            "{} {}",
                            self.tcp_state.timeline.entries().len(),
                            t!("ApiTest.messages")
                        ),
                        &theme,
                    )),
            )
    }

    fn tcp_status(&self, theme: &gpui_component::theme::Theme) -> (String, Hsla) {
        match &self.tcp_state.state {
            TcpState::Disconnected => (
                t!("ApiTest.disconnected").to_string(),
                theme.muted_foreground,
            ),
            TcpState::Connecting => (t!("ApiTest.connecting").to_string(), theme.warning),
            TcpState::Connected => (t!("ApiTest.connected").to_string(), theme.success),
            TcpState::Closing => (t!("ApiTest.disconnecting").to_string(), theme.warning),
            TcpState::Failed(_) => (t!("ApiTest.connection_failed").to_string(), theme.danger),
        }
    }

    fn render_tcp_metric(value: String, theme: &gpui_component::theme::Theme) -> Div {
        div()
            .flex_shrink_0()
            .px_2()
            .py_0p5()
            .rounded(px(999.))
            .bg(theme.muted.opacity(0.34))
            .text_xs()
            .whitespace_nowrap()
            .text_color(theme.muted_foreground)
            .child(value)
    }

    fn render_tcp_empty(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .p_2()
                    .rounded(px(8.))
                    .bg(theme.muted.opacity(0.3))
                    .child(Icon::new(IconName::Inbox)),
            )
            .child(t!("ApiTest.tcp_empty").to_string())
    }

    fn render_tcp_entry(
        &self,
        index: usize,
        entry: &TimelineEntry,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if entry.direction() == MessageDirection::System {
            return self.render_tcp_system_entry(index, entry, cx);
        }
        self.render_tcp_message_entry(index, entry, cx)
    }

    fn render_tcp_system_entry(
        &self,
        index: usize,
        entry: &TimelineEntry,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        div()
            .id(("api-tcp-system-entry", index))
            .w_full()
            .flex()
            .justify_center()
            .py_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(entry.display_text())
    }

    fn render_tcp_message_entry(
        &self,
        index: usize,
        entry: &TimelineEntry,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let sent = entry.direction() == MessageDirection::Sent;
        let color = if sent { theme.accent } else { theme.info };
        let label = if sent {
            t!("ApiTest.sent").to_string()
        } else {
            t!("ApiTest.received").to_string()
        };
        div()
            .id(("api-tcp-message-entry", index))
            .w_full()
            .flex()
            .when(sent, |row| row.justify_end())
            .when(!sent, |row| row.justify_start())
            .child(
                v_flex()
                    .max_w(px(820.))
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(color.opacity(0.3))
                    .bg(color.opacity(0.09))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(label),
                    )
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(entry.display_text()),
                    ),
            )
    }

    fn render_tcp_composer(&self, connected: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        h_flex()
            .id("api-tcp-composer")
            .w_full()
            .flex_shrink_0()
            .items_end()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                div().id("api-tcp-message-input").flex_1().min_w_0().child(
                    Textarea::new(&self.tcp_message_input)
                        .w_full()
                        .disabled(!connected),
                ),
            )
            .child(
                Button::new("api-tcp-send-message")
                    .primary()
                    .small()
                    .icon(IconName::ArrowUp)
                    .label(t!("ApiTest.send_message").to_string())
                    .disabled(!connected)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.send_tcp_message(window, cx);
                    })),
            )
    }
}
