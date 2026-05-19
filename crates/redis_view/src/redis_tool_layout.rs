//! Redis 工具页签布局渲染。

use crate::redis_chart_view::RedisChartView;
use crate::redis_pubsub::{PubSubMessage, PubSubMessageKind};
use crate::redis_tool_data::RedisToolKind;
use crate::redis_tool_pages::build_header;
use crate::redis_tool_view::{
    AUTO_REFRESH_SECONDS, ActionState, LoadState, PubSubBodyTab, RedisToolView,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    spinner::Spinner,
    switch::Switch,
    table::Table,
    tag::Tag,
    v_flex,
};
use rust_i18n::t;

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
                    .label(
                        t!(
                            "RedisTool.auto_refresh_label",
                            seconds = AUTO_REFRESH_SECONDS.to_string().as_str()
                        )
                        .to_string(),
                    )
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

    pub(crate) fn render_pubsub_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.render_subscribe_row(cx))
            .child(self.render_subscribed_chips(cx))
            .child(self.render_publish_row(cx))
            .when_some(self.subscribe_error.clone(), |this, err| {
                this.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(err),
                )
            })
            .into_any_element()
    }

    fn render_subscribe_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let view_subscribe = cx.entity().clone();
        let view_psubscribe = cx.entity().clone();
        let view_clear = cx.entity().clone();
        let has_subscriptions =
            !self.subscribed_channels.is_empty() || !self.subscribed_patterns.is_empty();
        h_flex()
            .w_full()
            .min_h(px(46.0))
            .px_3()
            .pt_2()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(Input::new(&self.subscribe_input)),
            )
            .child(
                Button::new("redis-pubsub-subscribe")
                    .primary()
                    .small()
                    .icon(IconName::Plus)
                    .label(t!("RedisPubSub.subscribe").to_string())
                    .on_click(move |_, window, cx| {
                        view_subscribe.update(cx, |view, cx| {
                            let value = view
                                .subscribe_input
                                .read(cx)
                                .text()
                                .to_string()
                                .trim()
                                .to_string();
                            if !value.is_empty() {
                                view.subscribe_channel(value, window, cx);
                            }
                        });
                    }),
            )
            .child(
                Button::new("redis-pubsub-psubscribe")
                    .small()
                    .icon(IconName::Plus)
                    .label(t!("RedisPubSub.psubscribe").to_string())
                    .on_click(move |_, window, cx| {
                        view_psubscribe.update(cx, |view, cx| {
                            let value = view
                                .subscribe_input
                                .read(cx)
                                .text()
                                .to_string()
                                .trim()
                                .to_string();
                            if !value.is_empty() {
                                view.subscribe_pattern(value, window, cx);
                            }
                        });
                    }),
            )
            .when(has_subscriptions, |this| {
                this.child(
                    Button::new("redis-pubsub-unsubscribe-all")
                        .ghost()
                        .small()
                        .icon(IconName::Delete)
                        .label(t!("RedisPubSub.unsubscribe_all").to_string())
                        .on_click(move |_, _, cx| {
                            view_clear.update(cx, |view, cx| view.unsubscribe_all(cx));
                        }),
                )
            })
            .into_any_element()
    }

    fn render_subscribed_chips(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.subscribed_channels.is_empty() && self.subscribed_patterns.is_empty() {
            return div()
                .w_full()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("RedisPubSub.subscribed_empty").to_string())
                .into_any_element();
        }

        let mut row = h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .px_3()
            .py_1p5()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("RedisPubSub.subscribed_label").to_string()),
            );

        for channel in self.subscribed_channels.clone() {
            row = row.child(self.render_chip(&channel, false, cx));
        }
        for pattern in self.subscribed_patterns.clone() {
            row = row.child(self.render_chip(&pattern, true, cx));
        }

        row.into_any_element()
    }

    fn render_chip(&self, name: &str, is_pattern: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().clone();
        let name_owned = name.to_string();
        let close_id = format!(
            "redis-pubsub-chip-close-{}-{}",
            if is_pattern { "p" } else { "c" },
            name
        );
        let close_button = Button::new(SharedString::from(close_id))
            .ghost()
            .xsmall()
            .icon(IconName::Close)
            .on_click(move |_, _, cx| {
                let name_inner = name_owned.clone();
                view.update(cx, move |view, cx| {
                    if is_pattern {
                        view.unsubscribe_pattern(&name_inner, cx);
                    } else {
                        view.unsubscribe_channel(&name_inner, cx);
                    }
                });
            });

        let tag = if is_pattern {
            Tag::warning().outline().small()
        } else {
            Tag::info().outline().small()
        };

        h_flex()
            .gap_1()
            .items_center()
            .child(tag.child(name.to_string()))
            .child(close_button)
            .into_any_element()
    }

    fn render_publish_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().clone();
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_3()
            .pb_2()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(220.0))
                    .child(Input::new(&self.publish_channel_input)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(180.0))
                    .child(Input::new(&self.publish_message_input)),
            )
            .child(
                Button::new("redis-pubsub-publish")
                    .primary()
                    .small()
                    .icon(IconName::Upload)
                    .label(t!("RedisPubSub.publish").to_string())
                    .on_click(move |_, window, cx| {
                        view.update(cx, |view, cx| view.publish_message(window, cx));
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn render_action_status(&self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(crate) fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
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

fn empty_connection(cx: &mut Context<RedisToolView>) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(t!("RedisTool.connection_required").to_string())
        .into_any_element()
}

fn loading() -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(Spinner::new().with_size(Size::Medium))
        .into_any_element()
}

fn loaded_body(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> AnyElement {
    if view.kind == RedisToolKind::Chart {
        return div()
            .flex_1()
            .min_h_0()
            .child(RedisChartView::new(&view.chart_history))
            .into_any_element();
    }

    // PubSub 页签:在 Channels 表格和 Messages 列表之间切换
    if view.kind == RedisToolKind::PubSub {
        return render_pubsub_body(view, cx).into_any_element();
    }

    let Some(table_state) = view.table_state.as_ref() else {
        return v_flex().flex_1().into_any_element();
    };

    // header 用全量行,表格用过滤后的行
    let all_rows = view.all_rows();
    let header = build_header(view.kind, &all_rows, cx);

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

fn render_pubsub_body(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> AnyElement {
    let tabs = render_pubsub_body_tabs(view, cx);
    let body: AnyElement = match view.body_tab {
        PubSubBodyTab::Channels => render_pubsub_channels(view, cx),
        PubSubBodyTab::Messages => render_received_messages(view, cx),
    };

    v_flex()
        .size_full()
        .min_h_0()
        .gap_3()
        .p_4()
        .child(tabs)
        .child(div().flex_1().min_h_0().w_full().child(body))
        .into_any_element()
}

fn render_pubsub_body_tabs(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> AnyElement {
    let view_channels = cx.entity().clone();
    let view_messages = cx.entity().clone();
    let view_clear = cx.entity().clone();
    let active = view.body_tab;
    let message_count = view.received_messages.len();

    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            Button::new("redis-pubsub-tab-channels")
                .when(active == PubSubBodyTab::Channels, |b| b.primary())
                .when(active != PubSubBodyTab::Channels, |b| b.ghost())
                .small()
                .label(t!("RedisPubSub.tab_channels").to_string())
                .on_click(move |_, _, cx| {
                    view_channels.update(cx, |view, cx| {
                        view.set_pubsub_body_tab(PubSubBodyTab::Channels, cx);
                    });
                }),
        )
        .child(
            Button::new("redis-pubsub-tab-messages")
                .when(active == PubSubBodyTab::Messages, |b| b.primary())
                .when(active != PubSubBodyTab::Messages, |b| b.ghost())
                .small()
                .label(
                    t!(
                        "RedisPubSub.tab_messages",
                        count = message_count.to_string().as_str()
                    )
                    .to_string(),
                )
                .on_click(move |_, _, cx| {
                    view_messages.update(cx, |view, cx| {
                        view.set_pubsub_body_tab(PubSubBodyTab::Messages, cx);
                    });
                }),
        )
        .child(div().flex_1())
        .when(
            active == PubSubBodyTab::Messages && message_count > 0,
            |this| {
                this.child(
                    Button::new("redis-pubsub-clear-messages")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Delete)
                        .label(t!("RedisPubSub.clear_messages").to_string())
                        .on_click(move |_, _, cx| {
                            view_clear.update(cx, |view, cx| view.clear_received_messages(cx));
                        }),
                )
            },
        )
        .into_any_element()
}

fn render_pubsub_channels(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> AnyElement {
    let Some(table_state) = view.table_state.as_ref() else {
        return v_flex().flex_1().into_any_element();
    };
    let all_rows = view.all_rows();
    let header = build_header(view.kind, &all_rows, cx);

    v_flex()
        .size_full()
        .min_h_0()
        .gap_3()
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

fn render_received_messages(view: &RedisToolView, cx: &mut Context<RedisToolView>) -> AnyElement {
    if view.received_messages.is_empty() {
        return v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("RedisPubSub.messages_empty_title").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground.opacity(0.7))
                    .child(t!("RedisPubSub.messages_empty_detail").to_string()),
            )
            .into_any_element();
    }

    // 表头
    let header = render_message_row_header(cx);

    // 消息列表(最新在底部)
    let mut list = v_flex().w_full();
    for msg in view.received_messages.iter() {
        list = list.child(render_message_row(msg, cx));
    }

    v_flex()
        .size_full()
        .min_h_0()
        .gap_1()
        .child(header)
        .child(
            div()
                .id("redis-pubsub-messages-scroll")
                .w_full()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(4.0))
                .child(list),
        )
        .into_any_element()
}

fn render_message_row_header(cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .w(px(160.0))
                .child(t!("RedisPubSub.column_time").to_string()),
        )
        .child(
            div()
                .w(px(86.0))
                .child(t!("RedisPubSub.column_type").to_string()),
        )
        .child(
            div()
                .w(px(180.0))
                .child(t!("RedisPubSub.column_channel").to_string()),
        )
        .child(
            div()
                .w(px(140.0))
                .child(t!("RedisPubSub.column_pattern").to_string()),
        )
        .child(
            div()
                .flex_1()
                .child(t!("RedisPubSub.column_payload").to_string()),
        )
        .into_any_element()
}

fn render_message_row(msg: &PubSubMessage, cx: &App) -> AnyElement {
    let time = msg.received_at.format("%H:%M:%S.%3f").to_string();
    let kind = match msg.kind {
        PubSubMessageKind::Message => "message",
        PubSubMessageKind::PMessage => "pmessage",
        PubSubMessageKind::SMessage => "smessage",
    };
    let pattern = msg.pattern.clone().unwrap_or_default();
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .text_sm()
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.5))
        .child(
            div()
                .w(px(160.0))
                .text_color(cx.theme().muted_foreground)
                .child(time),
        )
        .child(div().w(px(86.0)).text_xs().child(kind.to_string()))
        .child(div().w(px(180.0)).truncate().child(msg.channel.clone()))
        .child(
            div()
                .w(px(140.0))
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(pattern),
        )
        .child(div().flex_1().truncate().child(msg.payload.clone()))
        .into_any_element()
}
