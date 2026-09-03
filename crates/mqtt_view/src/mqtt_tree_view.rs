//! MQTT 连接树视图(简化版,参考 redis_tree_view 的骨架)
//!
//! 节点模型:
//! - 连接节点(来自 StoredConnection,双击或按钮连接)
//!   四态:默认 / 加载中(loading_nodes)/ 已连接(connected_nodes,
//!   含订阅子节点与消息计数)/ 错误(error_nodes,可重试)
//! - 订阅子节点(每个 MqttSubscription 一行,hover 可移除)

use std::collections::{HashMap, HashSet};

use connection_form::credential::resolve_connection_for_runtime;
use gpui::{
    AnyElement, App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, UniformListScrollHandle, Window, div, prelude::FluentBuilder, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize, Sizable, Size, WindowExt, button::IconButton,
    content_state::ContentState, h_flex, notification::Notification, scroll::ScrollableElement,
    spinner::Spinner, v_flex,
};
use mqtt_runtime::{MqttQos, MqttSubscription};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ActiveConnections, StoredConnection};
use rust_i18n::t;
use tracing::{error, warn};

use crate::manager::{GlobalMqttState, MqttManager};

/// 树视图事件
#[derive(Clone, Debug)]
pub enum MqttTreeViewEvent {
    /// 连接已建立
    ConnectionEstablished { connection_id: String },
    /// 连接已断开
    ConnectionClosed { connection_id: String },
    /// 订阅已添加(预留:当前订阅由 SubscribeView 直接调连接对象完成)
    SubscriptionAdded {
        connection_id: String,
        topic_filter: String,
        qos: MqttQos,
    },
    /// 订阅已移除(树内 hover 移除按钮触发)
    SubscriptionRemoved {
        connection_id: String,
        topic_filter: String,
    },
}

/// 扁平化的树条目
#[derive(Clone)]
struct FlatEntry {
    node_id: String,
    depth: usize,
    is_subscription: bool,
}

/// 重建扁平条目列表(纯函数,便于单测)
fn build_flat_entries(
    connection_order: &[String],
    subscriptions: &HashMap<String, Vec<MqttSubscription>>,
    expanded_nodes: &HashSet<String>,
) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for connection_id in connection_order {
        entries.push(FlatEntry {
            node_id: connection_id.clone(),
            depth: 0,
            is_subscription: false,
        });
        if expanded_nodes.contains(connection_id) {
            if let Some(subs) = subscriptions.get(connection_id) {
                for subscription in subs {
                    entries.push(FlatEntry {
                        node_id: format!("{connection_id}:sub:{}", subscription.topic_filter),
                        depth: 1,
                        is_subscription: true,
                    });
                }
            }
        }
    }
    entries
}

/// 返回所有能匹配到该主题的订阅过滤器
fn matched_filters<'a>(subscriptions: &'a [MqttSubscription], topic: &str) -> Vec<&'a str> {
    subscriptions
        .iter()
        .filter(|subscription| topic_matches(&subscription.topic_filter, topic))
        .map(|subscription| subscription.topic_filter.as_str())
        .collect()
}

/// MQTT 主题过滤器匹配(支持 + 与 # 通配符)
pub(crate) fn topic_matches(filter: &str, topic: &str) -> bool {
    fn match_levels(filter: &[&str], topic: &[&str]) -> bool {
        match filter.first() {
            None => topic.is_empty(),
            // "#" 匹配剩余所有层级(含父层级本身)
            Some(&"#") => true,
            Some(&"+") => !topic.is_empty() && match_levels(&filter[1..], &topic[1..]),
            Some(level) => {
                !topic.is_empty() && *level == topic[0] && match_levels(&filter[1..], &topic[1..])
            }
        }
    }

    let filter_levels: Vec<&str> = filter.split('/').collect();
    let topic_levels: Vec<&str> = topic.split('/').collect();
    match_levels(&filter_levels, &topic_levels)
}

/// MQTT 连接树视图
pub struct MqttTreeView {
    /// 连接顺序(插入序)
    connection_order: Vec<String>,
    /// 存储的连接配置(node_id -> StoredConnection)
    stored_connections: HashMap<String, StoredConnection>,
    /// 订阅列表(connection_id -> Vec<MqttSubscription>)
    subscriptions: HashMap<String, Vec<MqttSubscription>>,
    /// 每个订阅匹配到的消息数(connection_id -> filter -> count)
    sub_message_counts: HashMap<String, HashMap<String, u64>>,
    /// 每个连接接收的消息总数
    conn_message_counts: HashMap<String, u64>,
    /// 展开的节点
    expanded_nodes: HashSet<String>,
    /// 选中的节点
    selected_node: Option<String>,
    /// 加载中的节点
    loading_nodes: HashSet<String>,
    /// 出错的节点(node_id -> 错误信息)
    error_nodes: HashMap<String, String>,
    /// 已连接的节点
    connected_nodes: HashSet<String>,
    /// 滚动句柄
    scroll_handle: UniformListScrollHandle,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MqttTreeView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            connection_order: Vec::new(),
            stored_connections: HashMap::new(),
            subscriptions: HashMap::new(),
            sub_message_counts: HashMap::new(),
            conn_message_counts: HashMap::new(),
            expanded_nodes: HashSet::new(),
            selected_node: None,
            loading_nodes: HashSet::new(),
            error_nodes: HashMap::new(),
            connected_nodes: HashSet::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 从连接列表创建树视图
    pub fn new_with_connections(
        connections: &[StoredConnection],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(window, cx);
        for connection in connections {
            this.add_stored_connection(connection.clone(), cx);
        }
        this
    }

    /// 添加存储的连接(未连接状态)
    pub fn add_stored_connection(&mut self, connection: StoredConnection, cx: &mut Context<Self>) {
        let node_id = connection
            .id
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("temp-{}", uuid::Uuid::new_v4().simple()));

        self.connection_order.push(node_id.clone());
        self.stored_connections.insert(node_id, connection);
        cx.notify();
    }

    /// 获取存储的连接配置
    pub fn get_stored_connection(&self, node_id: &str) -> Option<&StoredConnection> {
        self.stored_connections.get(node_id)
    }

    /// 检查节点是否已连接
    pub fn is_connected(&self, node_id: &str) -> bool {
        self.connected_nodes.contains(node_id)
    }

    /// 节点是否展开
    pub fn is_node_expanded(&self, node_id: &str) -> bool {
        self.expanded_nodes.contains(node_id)
    }

    /// 激活连接并自动连接
    pub fn active_connection(&mut self, connection_id: String, cx: &mut Context<Self>) {
        if !self.stored_connections.contains_key(&connection_id) {
            return;
        }
        self.selected_node = Some(connection_id.clone());
        self.connect_node(connection_id, cx);
    }

    /// 连接到 MQTT 节点(状态机:默认 -> 加载中 -> 已连接/错误)
    pub fn connect_node(&mut self, node_id: String, cx: &mut Context<Self>) {
        // 已连接或加载中,直接跳过
        if self.connected_nodes.contains(&node_id) || self.loading_nodes.contains(&node_id) {
            return;
        }

        let Some(connection) = self.stored_connections.get(&node_id).cloned() else {
            warn!(node_id, "MQTT 连接配置缺失");
            return;
        };

        // 解析密码本引用,得到运行时可用的连接
        let connection = match resolve_connection_for_runtime(connection, cx) {
            Ok(connection) => connection,
            Err(err) => {
                warn!(node_id, %err, "解析 MQTT 凭据失败");
                self.error_nodes.insert(node_id, err);
                cx.notify();
                return;
            }
        };

        let numeric_id = connection.id;
        let global_state = cx.global::<GlobalMqttState>().clone();

        self.loading_nodes.insert(node_id.clone());
        self.error_nodes.remove(&node_id);
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let config = match MqttManager::config_from_stored(&connection) {
                Ok(config) => config,
                Err(err) => {
                    let message = err.to_string();
                    error!(node_id, %message, "MQTT 配置映射失败");
                    _ = this.update(cx, |view, cx| {
                        view.loading_nodes.remove(&node_id);
                        view.error_nodes.insert(node_id, message);
                        cx.notify();
                    });
                    return;
                }
            };

            let connect_result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let config = config.clone();
                async move {
                    global_state
                        .create_connection(config)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            match connect_result {
                Ok(_) => {
                    _ = this.update(cx, |view, cx| {
                        view.loading_nodes.remove(&node_id);
                        view.connected_nodes.insert(node_id.clone());
                        view.expanded_nodes.insert(node_id.clone());
                        if let Some(id) = numeric_id {
                            cx.global_mut::<ActiveConnections>().add(id);
                        }
                        cx.emit(MqttTreeViewEvent::ConnectionEstablished {
                            connection_id: node_id.clone(),
                        });
                        // 加载订阅列表并启动消息计数
                        view.load_subscriptions(node_id.clone(), cx);
                        view.start_message_counter(node_id, cx);
                        cx.notify();
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    error!(node_id, %message, "MQTT 连接失败");
                    _ = this.update(cx, |view, cx| {
                        view.loading_nodes.remove(&node_id);
                        view.error_nodes.insert(node_id, message);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 断开连接(带确认对话框)
    pub fn disconnect_connection(
        &mut self,
        node_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.stored_connections.get(node_id) else {
            return;
        };
        let connection_name = connection.name.clone();
        let connection_id = node_id.to_string();
        let global_state = cx.global::<GlobalMqttState>().clone();
        let tree = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let conn_id = connection_id.clone();
            let conn_name = connection_name.clone();
            let state = global_state.clone();
            let tree = tree.clone();

            dialog
                .overlay(false)
                .title(t!("MqttTree.confirm_disconnect_title").to_string())
                .confirm()
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("MqttTree.confirm_disconnect", name = conn_name).to_string())
                        .child(t!("MqttTree.disconnect_warning").to_string()),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let conn_id = conn_id.clone();
                    let state = state.clone();
                    let tree = tree.clone();
                    let task = Tokio::spawn_result(cx, {
                        let state = state.clone();
                        let conn_id = conn_id.clone();
                        async move {
                            state
                                .remove_connection(&conn_id)
                                .await
                                .map_err(anyhow::Error::new)
                        }
                    });

                    cx.spawn(async move |cx: &mut gpui::AsyncApp| match task.await {
                        Ok(_) => {
                            let _ = cx.update(|cx| {
                                tree.update(cx, |view, cx| {
                                    view.on_connection_removed(&conn_id, cx);
                                });
                            });
                        }
                        Err(err) => {
                            let message = format!("{err:#}");
                            let _ = cx.update(|cx| {
                                if let Some(window) = cx.active_window() {
                                    _ = window.update(cx, |_, window, cx| {
                                        window.push_notification(
                                            Notification::error(
                                                t!("MqttTree.disconnect_failed", error = message)
                                                    .to_string(),
                                            )
                                            .autohide(true),
                                            cx,
                                        );
                                    });
                                }
                            });
                        }
                    })
                    .detach();
                    true
                })
        });
    }

    /// 连接移除后的本地状态清理
    fn on_connection_removed(&mut self, connection_id: &str, cx: &mut Context<Self>) {
        self.connected_nodes.remove(connection_id);
        self.loading_nodes.remove(connection_id);
        self.error_nodes.remove(connection_id);
        self.subscriptions.remove(connection_id);
        self.sub_message_counts.remove(connection_id);
        self.conn_message_counts.remove(connection_id);
        self.expanded_nodes.remove(connection_id);

        // 从活跃连接表中移除
        if let Ok(numeric_id) = connection_id.parse::<i64>() {
            cx.global_mut::<ActiveConnections>().remove(numeric_id);
        }

        cx.emit(MqttTreeViewEvent::ConnectionClosed {
            connection_id: connection_id.to_string(),
        });
        cx.notify();
    }

    /// 加载/刷新连接的订阅列表
    pub fn load_subscriptions(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalMqttState>().clone();

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
                        view.subscriptions.insert(connection_id, subscriptions);
                        cx.notify();
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    warn!(%message, "加载 MQTT 订阅列表失败");
                }
            }
        })
        .detach();
    }

    /// 启动消息计数任务:消费实时消息流,按主题过滤器匹配计数
    fn start_message_counter(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalMqttState>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
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
                    return;
                }
            };

            let mut handle = handle;
            while let Some(message) = handle.recv().await {
                let topic = message.topic.clone();
                let alive = this
                    .update(cx, |view, cx| {
                        // 连接已断开则退出计数循环
                        if !view.connected_nodes.contains(&connection_id) {
                            return false;
                        }
                        view.record_message(&connection_id, &topic);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// 记录一条消息:更新连接总数与各订阅的匹配计数
    fn record_message(&mut self, connection_id: &str, topic: &str) {
        *self
            .conn_message_counts
            .entry(connection_id.to_string())
            .or_insert(0) += 1;

        let matched = self
            .subscriptions
            .get(connection_id)
            .map(|subscriptions| matched_filters(subscriptions, topic))
            .unwrap_or_default();
        if matched.is_empty() {
            return;
        }
        let counts = self
            .sub_message_counts
            .entry(connection_id.to_string())
            .or_default();
        for filter in matched {
            *counts.entry(filter.to_string()).or_insert(0) += 1;
        }
    }

    /// 移除订阅:取消订阅后刷新列表并发出事件
    fn remove_subscription(&mut self, node_id: &str, cx: &mut Context<Self>) {
        let Some((connection_id, topic_filter)) = node_id.split_once(":sub:") else {
            return;
        };
        let connection_id = connection_id.to_string();
        let topic_filter = topic_filter.to_string();
        let global_state = cx.global::<GlobalMqttState>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                let topic_filter = topic_filter.clone();
                async move {
                    let connection = global_state
                        .get_connection(&connection_id)
                        .ok_or_else(|| anyhow::anyhow!(t!("MqttTree.connection_missing")))?;
                    let guard = connection.read().await;
                    guard
                        .unsubscribe(&topic_filter)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;

            _ = this.update(cx, |view, cx| match result {
                Ok(_) => {
                    cx.emit(MqttTreeViewEvent::SubscriptionRemoved {
                        connection_id: connection_id.clone(),
                        topic_filter: topic_filter.clone(),
                    });
                    view.load_subscriptions(connection_id, cx);
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    error!(%message, "取消 MQTT 订阅失败");
                }
            });
        })
        .detach();
    }

    /// 重建扁平条目列表
    fn rebuild_flat_entries(&self) -> Vec<FlatEntry> {
        build_flat_entries(
            &self.connection_order,
            &self.subscriptions,
            &self.expanded_nodes,
        )
    }

    /// 切换展开状态
    fn toggle_node(&mut self, node_id: &str, cx: &mut Context<Self>) {
        if self.expanded_nodes.contains(node_id) {
            self.expanded_nodes.remove(node_id);
        } else {
            self.expanded_nodes.insert(node_id.to_string());
        }
        cx.notify();
    }

    /// 双击处理:错误重试 / 未连接则连接 / 已连接则确认断开
    fn handle_double_click(&mut self, node_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.error_nodes.contains_key(node_id) {
            self.error_nodes.remove(node_id);
            self.connect_node(node_id.to_string(), cx);
            return;
        }

        if self.connected_nodes.contains(node_id) {
            self.disconnect_connection(node_id, window, cx);
        } else {
            self.connect_node(node_id.to_string(), cx);
        }
    }

    /// 工具栏:刷新按钮(刷新所有已连接节点的订阅)
    fn render_toolbar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let connected: Vec<String> = self.connected_nodes.iter().cloned().collect();

        h_flex()
            .w_full()
            .p_1()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(t!("MqttView.connections").to_string()),
            )
            .child(
                IconButton::new("mqtt-tree-refresh", Icon::new(IconName::Refresh))
                    .hit_size(Size::XSmall)
                    .glyph_size(IconSize::Small)
                    .tooltip(t!("Common.refresh").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |view, cx| {
                            for connection_id in &connected {
                                view.load_subscriptions(connection_id.clone(), cx);
                            }
                        });
                    }),
            )
    }

    /// 渲染单个树条目
    fn render_item(&self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.rebuild_flat_entries();
        let Some(entry) = entries.get(ix) else {
            return div().into_any_element();
        };
        let node_id = entry.node_id.clone();
        let is_selected = self.selected_node.as_ref() == Some(&node_id);
        let is_loading = self.loading_nodes.contains(&node_id);
        let error_msg = (!is_loading)
            .then(|| self.error_nodes.get(&node_id).cloned())
            .flatten();
        let is_connected = self.connected_nodes.contains(&node_id);

        let view = cx.entity().clone();
        let view_for_dbl = cx.entity().clone();
        let node_id_for_dbl = node_id.clone();
        let tree = cx.theme().geometry.tree;

        // 订阅子节点行
        if entry.is_subscription {
            let Some((connection_id, topic_filter)) = node_id.split_once(":sub:") else {
                return div().into_any_element();
            };
            let connection_id = connection_id.to_string();
            let topic_filter = topic_filter.to_string();
            let count = self
                .sub_message_counts
                .get(&connection_id)
                .and_then(|counts| counts.get(&topic_filter).copied())
                .unwrap_or(0);
            let qos_label = self
                .subscriptions
                .get(&connection_id)
                .and_then(|subs| {
                    subs.iter()
                        .find(|sub| sub.topic_filter == topic_filter)
                        .map(|sub| sub.qos.label())
                })
                .unwrap_or("QoS 0");
            let view_for_remove = cx.entity().clone();
            let node_id_for_remove = node_id.clone();
            let topic_label = topic_filter.clone();

            return h_flex()
                .id(SharedString::from(format!("mqtt-sub-node-{ix}")))
                .group("mqtt-tree-item")
                .w_full()
                .h(tree.row_height)
                .pl(tree.base_padding + tree.indent * entry.depth)
                .pr(gpui::px(4.0))
                .gap_1()
                .items_center()
                .rounded(cx.theme().geometry.radius.xs)
                .when(is_selected, |this| this.bg(cx.theme().list_active))
                .when(!is_selected, |this| {
                    this.hover(|style| style.bg(cx.theme().list_hover))
                })
                .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                    if event.click_count == 2 {
                        cx.stop_propagation();
                        return;
                    }
                    view.update(cx, |view, cx| {
                        view.selected_node = Some(node_id.clone());
                        cx.notify();
                    });
                })
                .child(div().w(tree.disclosure_size).flex().flex_shrink_0())
                .child(
                    Icon::new(IconName::Bell)
                        .with_size(Size::XSmall)
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().flex_1().text_sm().truncate().child(topic_label))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(qos_label.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("({count})")),
                )
                .child(
                    h_flex()
                        .gap_0p5()
                        .invisible()
                        .group_hover("mqtt-tree-item", |this| this.visible())
                        .child(
                            IconButton::new(
                                SharedString::from(format!("mqtt-sub-remove-{ix}")),
                                Icon::new(IconName::Remove),
                            )
                            .hit_size(Size::XSmall)
                            .glyph_size(IconSize::Small)
                            .tooltip(t!("MqttSub.remove_subscription").to_string())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                view_for_remove.update(cx, |view, cx| {
                                    view.remove_subscription(&node_id_for_remove, cx);
                                });
                            }),
                        ),
                )
                .into_any_element();
        }

        // 连接节点行
        let Some(connection) = self.stored_connections.get(&node_id) else {
            return div().into_any_element();
        };
        let name = connection.name.clone();
        let total_count = self.conn_message_counts.get(&node_id).copied().unwrap_or(0);
        let has_children = self
            .subscriptions
            .get(&node_id)
            .is_some_and(|subs| !subs.is_empty());
        let is_expanded = self.expanded_nodes.contains(&node_id);
        let view_for_arrow = cx.entity().clone();
        let node_id_for_arrow = node_id.clone();
        let view_for_refresh = cx.entity().clone();
        let node_id_for_refresh = node_id.clone();
        let view_for_disconnect = cx.entity().clone();
        let node_id_for_disconnect = node_id.clone();
        let view_for_retry = cx.entity().clone();
        let node_id_for_retry = node_id.clone();
        let node_id_for_click = node_id.clone();

        h_flex()
            .id(SharedString::from(format!("mqtt-conn-node-{ix}")))
            .group("mqtt-tree-item")
            .w_full()
            .h(tree.row_height)
            .pl(tree.base_padding)
            .pr(gpui::px(4.0))
            .gap_1()
            .items_center()
            .rounded(cx.theme().geometry.radius.xs)
            .when(is_selected, |this| this.bg(cx.theme().list_active))
            .when(!is_selected, |this| {
                this.hover(|style| style.bg(cx.theme().list_hover))
            })
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                if event.click_count == 2 {
                    view_for_dbl.update(cx, |view, cx| {
                        view.handle_double_click(&node_id_for_dbl, window, cx);
                    });
                } else {
                    view.update(cx, |view, cx| {
                        view.selected_node = Some(node_id_for_click.clone());
                        cx.notify();
                    });
                }
            })
            // 展开/折叠箭头
            .child(
                div()
                    .id(SharedString::from(format!("mqtt-arrow-{ix}")))
                    .w(tree.disclosure_size)
                    .h(tree.disclosure_size)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .when(has_children && is_connected, |this| {
                        this.cursor_pointer()
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                view_for_arrow.update(cx, |view, cx| {
                                    view.toggle_node(&node_id_for_arrow, cx);
                                });
                            })
                            .child(
                                Icon::new(if is_expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .with_size(Size::XSmall)
                                .text_color(cx.theme().muted_foreground),
                            )
                    }),
            )
            // 连接图标
            .child(
                Icon::new(IconName::Mqtt)
                    .with_size(IconSize::Medium)
                    .when(!is_connected && error_msg.is_none(), |icon| {
                        icon.text_color(cx.theme().muted_foreground)
                    }),
            )
            // 名称 + 消息计数
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .truncate()
                    .when(!is_connected && error_msg.is_none(), |el| {
                        el.text_color(cx.theme().muted_foreground)
                    })
                    .child(name),
            )
            .when(is_connected && total_count > 0, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{total_count}")),
                )
            })
            // 加载中指示器
            .when(is_loading, |this| {
                this.child(
                    Spinner::new()
                        .with_size(IconSize::Small)
                        .color(cx.theme().muted_foreground),
                )
            })
            // 错误提示 + 重试按钮
            .when_some(error_msg.clone(), |this, message| {
                this.child(
                    IconButton::new(
                        SharedString::from(format!("mqtt-error-{ix}")),
                        Icon::new(IconName::TriangleAlert),
                    )
                    .hit_size(Size::XSmall)
                    .glyph_size(IconSize::Small)
                    .text_color(cx.theme().warning)
                    .tooltip(message),
                )
                .child(
                    IconButton::new(
                        SharedString::from(format!("mqtt-retry-{ix}")),
                        Icon::new(IconName::Refresh),
                    )
                    .hit_size(Size::XSmall)
                    .glyph_size(IconSize::Small)
                    .tooltip(t!("MqttTree.retry").to_string())
                    .on_click(move |_, _, cx| {
                        view_for_retry.update(cx, |view, cx| {
                            view.error_nodes.remove(&node_id_for_retry);
                            view.connect_node(node_id_for_retry.clone(), cx);
                        });
                    }),
                )
            })
            // 已连接时 hover 操作:刷新订阅 / 断开
            .when(is_connected && !is_loading, |this| {
                this.child(
                    h_flex()
                        .gap_0p5()
                        .invisible()
                        .group_hover("mqtt-tree-item", |this| this.visible())
                        .child(
                            IconButton::new(
                                SharedString::from(format!("mqtt-refresh-{ix}")),
                                Icon::new(IconName::Refresh),
                            )
                            .hit_size(Size::XSmall)
                            .glyph_size(IconSize::Small)
                            .tooltip(t!("MqttTree.refresh_subscriptions").to_string())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                view_for_refresh.update(cx, |view, cx| {
                                    view.load_subscriptions(node_id_for_refresh.clone(), cx);
                                });
                            }),
                        )
                        .child(
                            IconButton::new(
                                SharedString::from(format!("mqtt-disconnect-{ix}")),
                                Icon::new(IconName::Close),
                            )
                            .hit_size(Size::XSmall)
                            .glyph_size(IconSize::Small)
                            .tooltip(t!("MqttTree.disconnect").to_string())
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                view_for_disconnect.update(cx, |view, cx| {
                                    view.disconnect_connection(&node_id_for_disconnect, window, cx);
                                });
                            }),
                        ),
                )
            })
            .into_any_element()
    }
}

impl EventEmitter<MqttTreeViewEvent> for MqttTreeView {}

impl Focusable for MqttTreeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MqttTreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entry_count = self.rebuild_flat_entries().len();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_toolbar(window, cx))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .vertical_scrollbar(&self.scroll_handle)
                    .when(entry_count == 0, |this| {
                        this.child(
                            ContentState::empty(t!("MqttTree.no_connections").to_string())
                                .icon(Icon::new(IconName::Mqtt).color().with_size(IconSize::Large))
                                .compact(),
                        )
                    })
                    .when(entry_count > 0, |this| {
                        this.child(
                            uniform_list(
                                "mqtt-tree-list",
                                entry_count,
                                cx.processor(
                                    move |this: &mut Self,
                                          visible_range: std::ops::Range<usize>,
                                          window,
                                          cx| {
                                        visible_range
                                            .map(|ix| this.render_item(ix, window, cx))
                                            .collect()
                                    },
                                ),
                            )
                            .size_full()
                            .track_scroll(&self.scroll_handle),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(filter: &str) -> MqttSubscription {
        MqttSubscription {
            topic_filter: filter.to_string(),
            qos: MqttQos::AtLeastOnce,
        }
    }

    #[test]
    fn topic_matches_supports_wildcards() {
        // 精确匹配
        assert!(topic_matches("a/b", "a/b"));
        assert!(!topic_matches("a/b", "a/c"));
        // + 单层通配
        assert!(topic_matches("a/+/c", "a/b/c"));
        assert!(!topic_matches("a/+/c", "a/b/d"));
        assert!(!topic_matches("a/+", "a/b/c"));
        // # 多层通配(含父层级)
        assert!(topic_matches("a/#", "a"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(!topic_matches("a/#", "b/a"));
        // 根通配
        assert!(topic_matches("#", "anything/at/all"));
    }

    #[test]
    fn build_flat_entries_expands_subscriptions_only_when_expanded() {
        let order = vec!["1".to_string()];
        let subscriptions = HashMap::from([("1".to_string(), vec![sub("a/#"), sub("b")])]);

        let expanded = HashSet::from(["1".to_string()]);
        let entries = build_flat_entries(&order, &subscriptions, &expanded);
        assert_eq!(3, entries.len());
        assert_eq!("1", entries[0].node_id);
        assert!(!entries[0].is_subscription);
        assert_eq!("1:sub:a/#", entries[1].node_id);
        assert_eq!(1, entries[1].depth);
        assert!(entries[1].is_subscription);
        assert_eq!("1:sub:b", entries[2].node_id);

        // 折叠后只剩连接行
        let collapsed = HashSet::new();
        assert_eq!(
            1,
            build_flat_entries(&order, &subscriptions, &collapsed).len()
        );
    }

    #[test]
    fn matched_filters_returns_only_matching_subscriptions() {
        let subscriptions = vec![sub("sensor/+"), sub("alarm/#"), sub("other")];

        let matched = matched_filters(&subscriptions, "sensor/temperature");
        assert_eq!(vec!["sensor/+"], matched);

        let matched = matched_filters(&subscriptions, "alarm/home/kitchen");
        assert_eq!(vec!["alarm/#"], matched);

        assert!(matched_filters(&subscriptions, "unknown/topic").is_empty());
    }
}
