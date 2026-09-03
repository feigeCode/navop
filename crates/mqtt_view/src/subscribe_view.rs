//! MQTT 单连接操作页(SubscribeView)
//!
//! 布局自上而下:
//! - 订阅管理:topic filter 输入 + QoS 下拉 + 添加按钮;订阅列表行(filter/QoS/移除)
//! - 消息流:时间/topic/QoS/payload 预览(上限 1000 条,超出丢最旧;文本/Hex 切换;清空)
//! - 发布:topic 输入、payload 文本域、QoS、retain、发布按钮
//!
//! 消息来源:连接建立后经 GlobalMqttState 拿连接 open_pubsub() 得
//! MqttPubSubHandle,cx.spawn 消费循环(用 generation 防止旧任务)。

use std::collections::VecDeque;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IconSize, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _, IconButton},
    content_state::ContentState,
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    switch::Switch,
    v_flex,
};
use mqtt_runtime::{MqttMessage, MqttQos, MqttSubscription};
use one_core::gpui_tokio::Tokio;
use one_core::storage::StoredConnection;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use tracing::warn;

use crate::manager::GlobalMqttState;

/// 消息环形缓冲上限,超出丢最旧
const MAX_MESSAGES: usize = 1000;
/// payload 预览字符数上限
const PAYLOAD_PREVIEW_CHARS: usize = 100;

/// 订阅视图事件(供外层 MqttTabView 联动树视图刷新)
#[derive(Clone, Debug)]
pub enum MqttSubscribeViewEvent {
    /// 订阅列表发生变化(新增/移除)
    SubscriptionsChanged { connection_id: String },
}

/// QoS 下拉选项
#[derive(Clone, Debug, PartialEq, Eq)]
struct QosSelectItem(u8);

impl SelectItem for QosSelectItem {
    type Value = u8;

    fn title(&self) -> SharedString {
        format!("QoS {}", self.0).into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// payload 预览(纯函数,便于单测)
fn payload_preview(message: &MqttMessage, hex: bool) -> String {
    let text = if hex {
        message.payload_hex()
    } else {
        // 非 UTF-8 payload 回退为 Hex 显示
        message
            .payload_text()
            .unwrap_or_else(|| message.payload_hex())
    };
    text.chars().take(PAYLOAD_PREVIEW_CHARS).collect()
}

/// 消息时间(HH:MM:SS,本地时区)
fn message_time(message: &MqttMessage) -> String {
    message
        .received_at
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string()
}

/// MQTT 订阅/消息/发布操作页
pub struct MqttSubscribeView {
    /// 当前绑定的连接 ID
    connection_id: String,
    /// 存储的连接配置
    stored_connection: StoredConnection,
    /// 消息流是否已启动
    is_streaming: bool,
    /// 消费循环代号(递增使旧任务失效)
    stream_generation: u64,
    /// 当前订阅列表
    subscriptions: Vec<MqttSubscription>,
    /// 消息环形缓冲
    messages: VecDeque<MqttMessage>,
    /// payload 显示模式:true = Hex,false = 文本
    payload_hex: bool,
    // 订阅管理输入
    topic_filter_input: Entity<InputState>,
    subscribe_qos: Entity<SelectState<Vec<QosSelectItem>>>,
    // 发布输入
    publish_topic_input: Entity<InputState>,
    publish_payload_input: Entity<InputState>,
    publish_qos: Entity<SelectState<Vec<QosSelectItem>>>,
    publish_retain: bool,
    /// 消息列表滚动句柄
    messages_scroll: UniformListScrollHandle,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MqttSubscribeView {
    pub fn new(connection: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let connection_id = connection.id.map(|id| id.to_string()).unwrap_or_default();

        let topic_filter_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("MqttSub.topic_filter_placeholder"))
        });
        let publish_topic_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("MqttSub.publish_topic_placeholder"))
        });
        let publish_payload_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("MqttSub.publish_payload_placeholder"))
                .auto_grow(2, 6)
        });

        let qos_items = qos_options();
        let subscribe_qos = cx.new(|cx| {
            let mut state = SelectState::new(qos_items.clone(), None, window, cx);
            state.set_selected_value(&0, window, cx);
            state
        });
        let publish_qos = cx.new(|cx| {
            let mut state = SelectState::new(qos_options(), None, window, cx);
            state.set_selected_value(&0, window, cx);
            state
        });

        // 回车快捷添加订阅
        cx.subscribe_in(&topic_filter_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add_subscription(window, cx);
            }
        })
        .detach();

        let mut this = Self {
            connection_id,
            stored_connection: connection,
            is_streaming: false,
            stream_generation: 0,
            subscriptions: Vec::new(),
            messages: VecDeque::new(),
            payload_hex: false,
            topic_filter_input,
            subscribe_qos,
            publish_topic_input,
            publish_payload_input,
            publish_qos,
            publish_retain: false,
            messages_scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        };
        this.refresh_subscriptions(cx);
        this
    }

    /// 当前绑定的连接 ID
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// 重新绑定连接:清理本地状态并停止旧消息流
    pub fn bind_connection(
        &mut self,
        connection_id: String,
        stored_connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        if self.connection_id == connection_id {
            return;
        }
        self.connection_id = connection_id;
        self.stored_connection = stored_connection;
        self.subscriptions.clear();
        self.messages.clear();
        self.stop_message_stream(cx);
        self.refresh_subscriptions(cx);
    }

    /// 刷新订阅列表(list_subscriptions)
    pub fn refresh_subscriptions(&mut self, cx: &mut Context<Self>) {
        if self.connection_id.is_empty() {
            return;
        }
        let global_state = cx.global::<GlobalMqttState>().clone();
        let connection_id = self.connection_id.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard.list_subscriptions().await.map_err(anyhow::Error::new)
                }
            })
            .await;

            match result {
                Ok(subscriptions) => {
                    _ = this.update(cx, |view, cx| {
                        view.subscriptions = subscriptions;
                        cx.notify();
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    warn!(%message, "刷新 MQTT 订阅列表失败");
                }
            }
        })
        .detach();
    }

    /// 启动消息消费循环(仅在未启动时)
    pub fn start_message_stream(&mut self, cx: &mut Context<Self>) {
        if self.is_streaming || self.connection_id.is_empty() {
            return;
        }
        self.is_streaming = true;
        let generation = self.stream_generation.wrapping_add(1);
        self.stream_generation = generation;

        let global_state = cx.global::<GlobalMqttState>().clone();
        let connection_id = self.connection_id.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 在 Tokio 运行时中打开消息流句柄
            let handle = match Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard.open_pubsub().map_err(anyhow::Error::new)
                }
            })
            .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    let message = format!("{err:#}");
                    warn!(%message, "打开 MQTT 消息流失败");
                    _ = this.update(cx, |view, cx| {
                        if view.stream_generation == generation {
                            view.is_streaming = false;
                            cx.notify();
                        }
                    });
                    return;
                }
            };

            // 消费循环:generation 不匹配(已停止/重绑)则退出
            let mut handle = handle;
            while let Some(message) = handle.recv().await {
                let alive = this
                    .update(cx, |view, cx| {
                        if view.stream_generation != generation {
                            return false;
                        }
                        view.push_message(message, cx);
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }

            // 通道关闭(连接断开)
            _ = this.update(cx, |view, cx| {
                if view.stream_generation == generation {
                    view.is_streaming = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 停止消息消费循环
    pub fn stop_message_stream(&mut self, cx: &mut Context<Self>) {
        self.stream_generation = self.stream_generation.wrapping_add(1);
        self.is_streaming = false;
        cx.notify();
    }

    /// 追加消息,超过上限时丢弃最旧的
    fn push_message(&mut self, message: MqttMessage, cx: &mut Context<Self>) {
        self.messages.push_back(message);
        while self.messages.len() > MAX_MESSAGES {
            self.messages.pop_front();
        }
        cx.notify();
    }

    /// 当前选中的订阅 QoS
    fn selected_qos(&self, state: &Entity<SelectState<Vec<QosSelectItem>>>, cx: &App) -> MqttQos {
        let value = state.read(cx).selected_value().copied().unwrap_or(0);
        MqttQos::from_u8(value).unwrap_or(MqttQos::AtMostOnce)
    }

    /// 添加订阅:直接调连接对象,成功后刷新列表并广播事件
    fn add_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input_text = self.topic_filter_input.read(cx).text().to_string();
        let topic_filter = input_text.trim().to_string();
        if topic_filter.is_empty() {
            return;
        }
        // 先清空输入(乐观清空,失败会有错误通知)
        self.topic_filter_input
            .update(cx, |state, cx| state.set_value("", window, cx));

        let qos = self.selected_qos(&self.subscribe_qos, cx);
        let global_state = cx.global::<GlobalMqttState>().clone();
        let connection_id = self.connection_id.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                let filter = topic_filter.clone();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard
                        .subscribe(&filter, qos)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            match result {
                Ok(()) => {
                    _ = this.update(cx, |view, cx| {
                        view.refresh_subscriptions(cx);
                        cx.emit(MqttSubscribeViewEvent::SubscriptionsChanged {
                            connection_id: connection_id.clone(),
                        });
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    warn!(%message, "添加 MQTT 订阅失败");
                    push_notification_error(
                        cx,
                        t!("MqttSub.subscribe_failed", error = message).to_string(),
                    );
                }
            }
        })
        .detach();
    }

    /// 移除订阅(按索引):直接调连接对象
    fn remove_subscription_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(subscription) = self.subscriptions.get(index).cloned() else {
            return;
        };
        let global_state = cx.global::<GlobalMqttState>().clone();
        let connection_id = self.connection_id.clone();
        let topic_filter = subscription.topic_filter.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                let filter = topic_filter.clone();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard.unsubscribe(&filter).await.map_err(anyhow::Error::new)
                }
            })
            .await;

            match result {
                Ok(()) => {
                    _ = this.update(cx, |view, cx| {
                        view.refresh_subscriptions(cx);
                        cx.emit(MqttSubscribeViewEvent::SubscriptionsChanged {
                            connection_id: connection_id.clone(),
                        });
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    warn!(%message, "取消 MQTT 订阅失败");
                    push_notification_error(
                        cx,
                        t!("MqttSub.unsubscribe_failed", error = message).to_string(),
                    );
                }
            }
        })
        .detach();
    }

    /// 清空消息列表
    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        cx.notify();
    }

    /// 发布消息
    fn publish_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let topic = self
            .publish_topic_input
            .read(cx)
            .text()
            .to_string()
            .trim()
            .to_string();
        if topic.is_empty() {
            window.push_notification(
                Notification::error(t!("MqttSub.topic_required").to_string()).autohide(true),
                cx,
            );
            return;
        }
        let payload = self.publish_payload_input.read(cx).text().to_string();
        let qos = self.selected_qos(&self.publish_qos, cx);
        let retain = self.publish_retain;
        let global_state = cx.global::<GlobalMqttState>().clone();
        let connection_id = self.connection_id.clone();

        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                let topic = topic.clone();
                let payload = payload.clone().into_bytes();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard
                        .publish(&topic, &payload, qos, retain)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            let _ = cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    _ = window.update(cx, |_, window, cx| {
                        let notification = match result {
                            Ok(()) => Notification::success(
                                t!("MqttSub.publish_success", topic = topic).to_string(),
                            ),
                            Err(ref err) => Notification::error(
                                t!("MqttSub.publish_failed", error = format!("{err:#}"))
                                    .to_string(),
                            ),
                        };
                        window.push_notification(notification.autohide(true), cx);
                    });
                }
            });
        })
        .detach();
    }

    /// 渲染订阅管理区
    fn render_subscriptions_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let subscription_rows = self
            .subscriptions
            .iter()
            .enumerate()
            .map(|(index, subscription)| {
                let view = cx.entity().clone();
                h_flex()
                    .id(SharedString::from(format!("mqtt-sub-row-{index}")))
                    .group("mqtt-sub-row")
                    .w_full()
                    .py_1()
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(cx.theme().geometry.radius.xs)
                    .hover(|style| style.bg(cx.theme().list_hover))
                    .child(
                        Icon::new(IconName::Bell)
                            .with_size(Size::XSmall)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .truncate()
                            .child(subscription.topic_filter.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(subscription.qos.label().to_string()),
                    )
                    .child(
                        h_flex()
                            .invisible()
                            .group_hover("mqtt-sub-row", |this| this.visible())
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("mqtt-sub-remove-{index}")),
                                    Icon::new(IconName::Remove),
                                )
                                .hit_size(Size::XSmall)
                                .glyph_size(IconSize::Small)
                                .tooltip(t!("MqttSub.remove_subscription").to_string())
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |view, cx| {
                                        view.remove_subscription_at(index, cx);
                                    });
                                }),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        let is_connected = !self.connection_id.is_empty();

        v_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(t!("MqttView.subscriptions").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.topic_filter_input)))
                    .child(
                        div()
                            .w(px(96.0))
                            .child(Select::new(&self.subscribe_qos).w_full()),
                    )
                    .child(
                        Button::new("mqtt-add-subscription")
                            .small()
                            .label(t!("MqttSub.add_subscription").to_string())
                            .disabled(!is_connected)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_subscription(window, cx);
                            })),
                    ),
            )
            .when(self.subscriptions.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("MqttSub.no_subscriptions").to_string()),
                )
            })
            .when(!self.subscriptions.is_empty(), |this| {
                this.child(v_flex().gap_0p5().children(subscription_rows))
            })
    }

    /// 渲染单条消息行
    fn render_message_item(
        &self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        let payload_hex = self.payload_hex;

        h_flex()
            .id(SharedString::from(format!("mqtt-message-{ix}")))
            .w_full()
            .py_1()
            .px_2()
            .gap_2()
            .items_center()
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(
                div()
                    .w(px(72.0))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(message_time(message)),
            )
            .child(
                div()
                    .w(px(160.0))
                    .flex_shrink_0()
                    .text_sm()
                    .truncate()
                    .child(message.topic.clone()),
            )
            .child(
                div()
                    .w(px(44.0))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(message.qos.label().to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(payload_preview(message, payload_hex)),
            )
            .into_any_element()
    }

    /// 渲染消息流区
    fn render_messages_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.messages.len();
        let payload_hex = self.payload_hex;
        let view = cx.entity().clone();

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_1()
            .py_2()
            .child(
                h_flex()
                    .px_2()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("{} ({count})", t!("MqttView.messages"))),
                    )
                    .child(
                        Button::new("mqtt-payload-mode")
                            .small()
                            .ghost()
                            .label(if payload_hex {
                                t!("MqttSub.payload_hex").to_string()
                            } else {
                                t!("MqttSub.payload_text").to_string()
                            })
                            .tooltip(t!("MqttSub.toggle_payload_mode").to_string())
                            .on_click(move |_, _, cx| {
                                view.update(cx, |view, cx| {
                                    view.payload_hex = !view.payload_hex;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("mqtt-clear-messages")
                            .small()
                            .ghost()
                            .label(t!("MqttSub.clear_messages").to_string())
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_messages(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .vertical_scrollbar(&self.messages_scroll)
                    .when(count == 0, |this| {
                        this.child(
                            ContentState::empty(t!("MqttSub.no_messages").to_string()).compact(),
                        )
                    })
                    .when(count > 0, |this| {
                        this.child(
                            uniform_list(
                                "mqtt-messages-list",
                                count,
                                cx.processor(
                                    move |this: &mut Self,
                                          visible_range: std::ops::Range<usize>,
                                          window,
                                          cx| {
                                        visible_range
                                            .map(|ix| this.render_message_item(ix, window, cx))
                                            .collect()
                                    },
                                ),
                            )
                            .size_full()
                            .track_scroll(&self.messages_scroll),
                        )
                    }),
            )
    }

    /// 渲染发布区
    fn render_publish_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let retain = self.publish_retain;

        v_flex()
            .gap_2()
            .p_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(t!("MqttView.publish").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.publish_topic_input)))
                    .child(
                        div()
                            .w(px(96.0))
                            .child(Select::new(&self.publish_qos).w_full()),
                    )
                    .child(
                        Switch::new("mqtt-publish-retain")
                            .checked(retain)
                            .label(t!("MqttSub.retain").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.publish_retain = !this.publish_retain;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("mqtt-publish")
                            .small()
                            .primary()
                            .label(t!("MqttView.publish").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.publish_message(window, cx);
                            })),
                    ),
            )
            .child(Input::new(&self.publish_payload_input))
    }
}

/// QoS 选项列表
fn qos_options() -> Vec<QosSelectItem> {
    vec![QosSelectItem(0), QosSelectItem(1), QosSelectItem(2)]
}

/// 在异步上下文中推送错误通知(通过激活窗口)
fn push_notification_error(cx: &mut AsyncApp, message: String) {
    let _ = cx.update(|cx| {
        if let Some(window) = cx.active_window() {
            _ = window.update(cx, |_, window, cx| {
                window.push_notification(Notification::error(message).autohide(true), cx);
            });
        }
    });
}

impl EventEmitter<MqttSubscribeViewEvent> for MqttSubscribeView {}
impl EventEmitter<TabContentEvent> for MqttSubscribeView {}

impl TabContent for MqttSubscribeView {
    fn content_key(&self) -> &'static str {
        "mqtt-messages"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("MqttView.tab_title").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Bell).with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        false
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui::Task<bool> {
        gpui::Task::ready(true)
    }
}

impl Focusable for MqttSubscribeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MqttSubscribeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_subscriptions_panel(cx))
            .child(self.render_messages_panel(cx))
            .child(self.render_publish_panel(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(payload: Vec<u8>) -> MqttMessage {
        use chrono::TimeZone;
        MqttMessage {
            topic: "a/b".to_string(),
            payload,
            qos: MqttQos::AtLeastOnce,
            retain: false,
            received_at: chrono::Utc.timestamp_opt(1700000000, 0).unwrap(),
        }
    }

    #[test]
    fn payload_preview_truncates_to_limit() {
        let long = vec![b'a'; 300];
        let preview = payload_preview(&message(long), false);
        assert_eq!(PAYLOAD_PREVIEW_CHARS, preview.chars().count());
    }

    #[test]
    fn payload_preview_falls_back_to_hex_for_binary() {
        let preview = payload_preview(&message(vec![0xFF, 0x01]), false);
        assert_eq!("FF 01", preview);
    }

    #[test]
    fn payload_preview_hex_mode_uses_hex_text() {
        let preview = payload_preview(&message(b"hi".to_vec()), true);
        assert_eq!("68 69", preview);
    }

    #[test]
    fn message_time_uses_local_hms_format() {
        let time = message_time(&message(Vec::new()));
        assert_eq!(8, time.len());
        assert!(time.ends_with(|c: char| c.is_ascii_digit()));
    }
}
