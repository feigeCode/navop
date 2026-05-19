//! Redis 工具页签布局渲染。

use crate::redis_chart_view::RedisChartView;
use crate::redis_tool_data::RedisToolKind;
use crate::redis_tool_pages::build_header;
use crate::redis_tool_view::{AUTO_REFRESH_SECONDS, ActionState, LoadState, RedisToolView};
use gpui::prelude::FluentBuilder;
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    spinner::Spinner,
    switch::Switch,
    table::Table,
    v_flex,
};

impl RedisToolView {
    pub(crate) fn icon_name(&self) -> IconName {
        match self.kind {
            RedisToolKind::Info => IconName::Info,
            RedisToolKind::Memory => IconName::MemoryStick,
            RedisToolKind::SlowLog => IconName::LoaderCircle,
            RedisToolKind::Monitor => IconName::Monitor,
            RedisToolKind::PubSub => IconName::Network,
            RedisToolKind::Chart => IconName::ChartPie,
        }
    }

    pub(crate) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let auto_view = cx.entity().clone();
        let reset_view = cx.entity().clone();
        h_flex()
            .w_full()
            .h(px(44.0))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Icon::new(self.icon_name()).with_size(Size::Small))
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(self.kind.title()),
            )
            .child(div().flex_1())
            .when(self.kind != RedisToolKind::Chart, |this| {
                this.child(div().w(px(260.0)).child(Input::new(&self.filter_input)))
            })
            .child(
                Switch::new("redis-tool-auto-refresh")
                    .small()
                    .checked(self.auto_refresh)
                    .label(format!("Auto {AUTO_REFRESH_SECONDS}s"))
                    .on_click(move |checked, _, cx| {
                        auto_view.update(cx, |view, cx| view.set_auto_refresh(*checked, cx));
                    }),
            )
            .when(self.kind == RedisToolKind::SlowLog, |this| {
                this.child(
                    Button::new("redis-slowlog-reset")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Delete)
                        .on_click(move |_, _, cx| {
                            reset_view.update(cx, |view, cx| view.reset_slowlog(cx));
                        }),
                )
            })
            .child(
                Button::new("redis-tool-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Refresh)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |view, cx| view.refresh(cx));
                    }),
            )
    }

    pub(crate) fn render_pubsub_actions(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = cx.entity().clone();
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w(px(220.0))
                    .child(Input::new(&self.publish_channel_input)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(240.0))
                    .child(Input::new(&self.publish_message_input)),
            )
            .child(
                Button::new("redis-pubsub-publish")
                    .primary()
                    .small()
                    .icon(IconName::Upload)
                    .label("Publish")
                    .on_click(move |_, _, cx| {
                        view.update(cx, |view, cx| view.publish_message(cx));
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn render_action_status(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (message, color) = match &self.action_state {
            ActionState::Idle => return div().into_any_element(),
            ActionState::Pending(message) => (message.clone(), cx.theme().muted_foreground),
            ActionState::Success(message) => (message.clone(), cx.theme().success),
            ActionState::Error(message) => (message.clone(), cx.theme().danger),
        };
        div()
            .w_full()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .text_color(color)
            .child(message)
            .into_any_element()
    }

    pub(crate) fn render_body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        match &self.load_state {
            LoadState::Empty => empty_connection(cx),
            LoadState::Loading => loading(),
            LoadState::Loaded(_) => loaded_body(self, cx),
            LoadState::Error(error) => div()
                .flex_1()
                .p_4()
                .text_color(cx.theme().danger)
                .child(error.clone())
                .into_any_element(),
        }
    }
}

fn empty_connection(cx: &mut Context<RedisToolView>) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child("请选择并连接一个 Redis 连接")
        .into_any_element()
}

fn loading() -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(Spinner::new().with_size(Size::Medium))
        .into_any_element()
}

fn loaded_body(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> gpui::AnyElement {
    if view.kind == RedisToolKind::Chart {
        return div()
            .flex_1()
            .min_h_0()
            .child(RedisChartView::new(&view.chart_history))
            .into_any_element();
    }

    let Some(table_state) = view.table_state.as_ref() else {
        return v_flex().flex_1().into_any_element();
    };

    let filtered = view.filtered_rows();
    let header = build_header(view.kind, &filtered, cx);

    v_flex()
        .size_full()
        .min_h_0()
        .gap_4()
        .p_4()
        .child(header)
        .child(
            div()
                .id("redis-tool-table-wrap")
                .w_full()
                .flex_1()
                .min_h_0()
                .child(Table::new(table_state).stripe(true)),
        )
        .into_any_element()
}
