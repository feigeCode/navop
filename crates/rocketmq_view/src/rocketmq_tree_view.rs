//! RocketMQ 连接树视图(参考 mqtt_view::mqtt_tree_view 的骨架)
//!
//! 节点模型:
//! - 连接节点(来自 StoredConnection,双击或按钮连接)
//!   四态:默认 / 加载中(loading_nodes)/ 已连接(connected_nodes,
//!   含 Broker 与 Topic 子节点)/ 错误(error_nodes,可重试)
//! - Broker 子节点(来自 cluster_overview,显示地址与版本)
//! - Topic 子节点(来自 list_topics,显示队列数)

use std::collections::{HashMap, HashSet};

use connection_form::credential::resolve_connection_for_runtime;
use gpui::{
    AnyElement, App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, UniformListScrollHandle, Window, div, prelude::FluentBuilder, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize, Sizable, Size, WindowExt, h_flex,
    notification::Notification, scroll::ScrollableElement, spinner::Spinner, v_flex,
};
use middleware_runtime::{BrokerInfo, MiddlewareTopicInfo};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ActiveConnections, StoredConnection};
use one_ui::{ContentState, IconButton};
use rust_i18n::t;
use tracing::{error, warn};

use crate::manager::{GlobalRocketmqState, RocketmqManager};

/// 树视图事件
#[derive(Clone, Debug)]
pub enum RocketmqTreeViewEvent {
    /// 连接已建立
    ConnectionEstablished { connection_id: String },
    /// 连接已断开
    ConnectionClosed { connection_id: String },
}

/// 扁平化的树条目
#[derive(Clone)]
struct FlatEntry {
    node_id: String,
    depth: usize,
    kind: FlatEntryKind,
}

/// 条目类别:连接 / Broker / Topic
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FlatEntryKind {
    Connection,
    Broker,
    Topic,
}

/// 重建扁平条目列表(纯函数,便于单测)
fn build_flat_entries(
    connection_order: &[String],
    brokers: &HashMap<String, Vec<BrokerInfo>>,
    topics: &HashMap<String, Vec<MiddlewareTopicInfo>>,
    expanded_nodes: &HashSet<String>,
) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for connection_id in connection_order {
        entries.push(FlatEntry {
            node_id: connection_id.clone(),
            depth: 0,
            kind: FlatEntryKind::Connection,
        });
        if !expanded_nodes.contains(connection_id) {
            continue;
        }
        if let Some(broker_list) = brokers.get(connection_id) {
            for broker in broker_list {
                entries.push(FlatEntry {
                    node_id: format!("{connection_id}:broker:{}", broker.name),
                    depth: 1,
                    kind: FlatEntryKind::Broker,
                });
            }
        }
        if let Some(topic_list) = topics.get(connection_id) {
            for topic in topic_list {
                entries.push(FlatEntry {
                    node_id: format!("{connection_id}:topic:{}", topic.name),
                    depth: 1,
                    kind: FlatEntryKind::Topic,
                });
            }
        }
    }
    entries
}

/// RocketMQ 连接树视图
pub struct RocketmqTreeView {
    /// 连接顺序(插入序)
    connection_order: Vec<String>,
    /// 存储的连接配置(node_id -> StoredConnection)
    stored_connections: HashMap<String, StoredConnection>,
    /// 已连接节点的 Broker 列表(connection_id -> brokers)
    brokers: HashMap<String, Vec<BrokerInfo>>,
    /// 已连接节点的 Topic 列表(connection_id -> topics)
    topics: HashMap<String, Vec<MiddlewareTopicInfo>>,
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

impl RocketmqTreeView {
    /// 创建空连接树视图(无任何连接节点)
    ///
    /// `_window`/`cx` 用于创建焦点句柄;连接配置后续经
    /// [`RocketmqTreeView::add_stored_connection`] 逐个加入。
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            connection_order: Vec::new(),
            stored_connections: HashMap::new(),
            brokers: HashMap::new(),
            topics: HashMap::new(),
            expanded_nodes: HashSet::new(),
            selected_node: None,
            loading_nodes: HashSet::new(),
            error_nodes: HashMap::new(),
            connected_nodes: HashSet::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 以既有连接列表构造
    pub fn new_with_connections(
        connections: &[StoredConnection],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(window, cx);
        for connection in connections {
            view.add_stored_connection(connection.clone(), cx);
        }
        view
    }

    /// 添加连接配置(不自动连接)
    pub fn add_stored_connection(&mut self, connection: StoredConnection, cx: &mut Context<Self>) {
        let node_id = connection.id.map(|id| id.to_string()).unwrap_or_default();
        if node_id.is_empty() {
            warn!(name = connection.name, "RocketMQ 连接缺少数字 ID,跳过入树");
            return;
        }
        if !self.stored_connections.contains_key(&node_id) {
            self.connection_order.push(node_id.clone());
        }
        self.stored_connections.insert(node_id, connection);
        cx.notify();
    }

    /// 读取连接配置(供页签联动查询)
    pub fn get_stored_connection(&self, node_id: &str) -> Option<&StoredConnection> {
        self.stored_connections.get(node_id)
    }

    /// 是否已连接
    pub fn is_connected(&self, node_id: &str) -> bool {
        self.connected_nodes.contains(node_id)
    }

    /// 激活连接并自动连接
    pub fn active_connection(&mut self, connection_id: String, cx: &mut Context<Self>) {
        if !self.stored_connections.contains_key(&connection_id) {
            return;
        }
        self.selected_node = Some(connection_id.clone());
        self.connect_node(connection_id, cx);
    }

    /// 连接到 RocketMQ 节点(状态机:默认 -> 加载中 -> 已连接/错误)
    pub fn connect_node(&mut self, node_id: String, cx: &mut Context<Self>) {
        // 已连接或加载中,直接跳过
        if self.connected_nodes.contains(&node_id) || self.loading_nodes.contains(&node_id) {
            return;
        }

        let Some(connection) = self.stored_connections.get(&node_id).cloned() else {
            warn!(node_id, "RocketMQ 连接配置缺失");
            return;
        };

        // 解析密码本引用与引用式 SSH 隧道,得到运行时可用的连接
        let connection = match resolve_connection_for_runtime(connection, cx) {
            Ok(connection) => connection,
            Err(err) => {
                warn!(node_id, %err, "解析 RocketMQ 凭据失败");
                self.error_nodes.insert(node_id, err);
                cx.notify();
                return;
            }
        };

        let numeric_id = connection.id;
        let global_state = cx.global::<GlobalRocketmqState>().clone();

        self.loading_nodes.insert(node_id.clone());
        self.error_nodes.remove(&node_id);
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let params = match RocketmqManager::params_from_stored(&connection) {
                Ok(params) => params,
                Err(err) => {
                    let message = err.to_string();
                    error!(node_id, %message, "RocketMQ 参数映射失败");
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
                let node_id = node_id.clone();
                async move {
                    global_state
                        .create_connection(&node_id, params)
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
                        cx.emit(RocketmqTreeViewEvent::ConnectionEstablished {
                            connection_id: node_id.clone(),
                        });
                        // 连接建立后异步加载 Broker 与 Topic 子节点
                        view.load_cluster_metadata(node_id, cx);
                        cx.notify();
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    error!(node_id, %message, "RocketMQ 连接失败");
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

    /// 加载/刷新连接的集群元数据(Broker + Topic 列表)
    pub fn load_cluster_metadata(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let Some(handle) = cx
            .global::<GlobalRocketmqState>()
            .get_admin_handle(&connection_id)
        else {
            return;
        };

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                async move {
                    let overview = handle.cluster_overview().await?;
                    let topics = handle.list_topics().await?;
                    anyhow::Ok((overview, topics))
                }
            })
            .await;

            match result {
                Ok((overview, topics)) => {
                    let brokers: Vec<BrokerInfo> = overview
                        .clusters
                        .iter()
                        .flat_map(|cluster| cluster.brokers.iter().cloned())
                        .collect();
                    _ = this.update(cx, |view, cx| {
                        view.brokers.insert(connection_id.clone(), brokers);
                        view.topics.insert(connection_id.clone(), topics);
                        cx.notify();
                    });
                }
                Err(err) => {
                    // 元数据加载失败不影响已连接状态,仅记录日志
                    warn!(connection_id, %err, "RocketMQ 集群元数据加载失败");
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
        let global_state = cx.global::<GlobalRocketmqState>().clone();
        let tree = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let conn_id = connection_id.clone();
            let conn_name = connection_name.clone();
            let state = global_state.clone();
            let tree = tree.clone();

            dialog
                .overlay(false)
                .title(t!("RocketmqTree.confirm_disconnect_title").to_string())
                .confirm()
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("RocketmqTree.confirm_disconnect", name = conn_name).to_string()),
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
                                                t!(
                                                    "RocketmqTree.disconnect_failed",
                                                    error = message
                                                )
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
        self.brokers.remove(connection_id);
        self.topics.remove(connection_id);
        self.expanded_nodes.remove(connection_id);

        // 从活跃连接表中移除
        if let Ok(numeric_id) = connection_id.parse::<i64>() {
            cx.global_mut::<ActiveConnections>().remove(numeric_id);
        }

        cx.emit(RocketmqTreeViewEvent::ConnectionClosed {
            connection_id: connection_id.to_string(),
        });
        cx.notify();
    }

    /// 展开/折叠节点
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

    /// 工具栏:刷新按钮(刷新所有已连接节点的集群元数据)
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
                    .child(t!("RocketmqView.connections").to_string()),
            )
            .child(
                IconButton::new("rocketmq-tree-refresh", Icon::new(IconName::Refresh))
                    .hit_size(Size::XSmall)
                    .glyph_size(one_ui::IconSize::Small)
                    .tooltip(t!("Common.refresh").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |view, cx| {
                            for connection_id in &connected {
                                view.load_cluster_metadata(connection_id.clone(), cx);
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

        // 子节点行(Broker / Topic)
        if entry.kind != FlatEntryKind::Connection {
            let (connection_id, rest) = match node_id
                .split_once(':')
                .and_then(|(conn, rest)| rest.split_once(':').map(|(_, name)| (conn, name)))
            {
                Some(parts) => parts,
                None => return div().into_any_element(),
            };
            let connection_id = connection_id.to_string();

            let (icon, label, badge) = if entry.kind == FlatEntryKind::Broker {
                let broker = self
                    .brokers
                    .get(&connection_id)
                    .and_then(|brokers| brokers.iter().find(|b| b.name == rest));
                let label = broker
                    .map(|b| format!("{} ({})", b.name, b.address))
                    .unwrap_or_else(|| rest.to_string());
                let badge = broker.and_then(|b| b.version.clone()).unwrap_or_default();
                (IconName::Server, label, badge)
            } else {
                let topic = self
                    .topics
                    .get(&connection_id)
                    .and_then(|topics| topics.iter().find(|t| t.name == rest));
                let label = rest.to_string();
                let badge = topic
                    .and_then(|t| t.queue_count)
                    .map(|count| format!("{count}Q"))
                    .unwrap_or_default();
                (IconName::Inbox, label, badge)
            };

            return h_flex()
                .id(SharedString::from(format!("rocketmq-child-node-{ix}")))
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
                    Icon::new(icon)
                        .with_size(Size::XSmall)
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().flex_1().text_sm().truncate().child(label))
                .when(!badge.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(badge),
                    )
                })
                .into_any_element();
        }

        // 连接节点行
        let Some(connection) = self.stored_connections.get(&node_id) else {
            return div().into_any_element();
        };
        let name = connection.name.clone();
        let broker_count = self.brokers.get(&node_id).map(Vec::len).unwrap_or(0);
        let topic_count = self.topics.get(&node_id).map(Vec::len).unwrap_or(0);
        let has_children = is_connected && (broker_count + topic_count) > 0;
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
            .id(SharedString::from(format!("rocketmq-conn-node-{ix}")))
            .group("rocketmq-tree-item")
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
                    .id(SharedString::from(format!("rocketmq-arrow-{ix}")))
                    .w(tree.disclosure_size)
                    .h(tree.disclosure_size)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .when(has_children, |this| {
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
            // 连接图标(RocketMQ 品牌图标)
            .child(
                Icon::default()
                    .path(one_core::storage::NAVOP_ROCKETMQ_COLOR_ICON)
                    .with_size(IconSize::Medium)
                    .when(!is_connected && error_msg.is_none(), |icon| {
                        icon.text_color(cx.theme().muted_foreground)
                    }),
            )
            // 名称 + 元数据计数
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
            .when(is_connected, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{broker_count}B/{topic_count}T")),
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
                        SharedString::from(format!("rocketmq-error-{ix}")),
                        Icon::new(IconName::TriangleAlert),
                    )
                    .hit_size(Size::XSmall)
                    .glyph_size(one_ui::IconSize::Small)
                    .text_color(cx.theme().warning)
                    .tooltip(message),
                )
                .child(
                    IconButton::new(
                        SharedString::from(format!("rocketmq-retry-{ix}")),
                        Icon::new(IconName::Refresh),
                    )
                    .hit_size(Size::XSmall)
                    .glyph_size(one_ui::IconSize::Small)
                    .tooltip(t!("RocketmqTree.retry").to_string())
                    .on_click(move |_, _, cx| {
                        view_for_retry.update(cx, |view, cx| {
                            view.error_nodes.remove(&node_id_for_retry);
                            view.connect_node(node_id_for_retry.clone(), cx);
                        });
                    }),
                )
            })
            // 已连接时 hover 操作:刷新元数据 / 断开
            .when(is_connected && !is_loading, |this| {
                this.child(
                    h_flex()
                        .gap_0p5()
                        .invisible()
                        .group_hover("rocketmq-tree-item", |this| this.visible())
                        .child(
                            IconButton::new(
                                SharedString::from(format!("rocketmq-refresh-{ix}")),
                                Icon::new(IconName::Refresh),
                            )
                            .hit_size(Size::XSmall)
                            .glyph_size(one_ui::IconSize::Small)
                            .tooltip(t!("RocketmqTree.refresh_metadata").to_string())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                view_for_refresh.update(cx, |view, cx| {
                                    view.load_cluster_metadata(node_id_for_refresh.clone(), cx);
                                });
                            }),
                        )
                        .child(
                            IconButton::new(
                                SharedString::from(format!("rocketmq-disconnect-{ix}")),
                                Icon::new(IconName::Close),
                            )
                            .hit_size(Size::XSmall)
                            .glyph_size(one_ui::IconSize::Small)
                            .tooltip(t!("RocketmqTree.disconnect").to_string())
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

    /// 重建扁平条目(渲染与计数共用)
    fn rebuild_flat_entries(&self) -> Vec<FlatEntry> {
        build_flat_entries(
            &self.connection_order,
            &self.brokers,
            &self.topics,
            &self.expanded_nodes,
        )
    }
}

impl EventEmitter<RocketmqTreeViewEvent> for RocketmqTreeView {}

impl Focusable for RocketmqTreeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RocketmqTreeView {
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
                            ContentState::empty(t!("RocketmqTree.no_connections").to_string())
                                .icon(
                                    Icon::default()
                                        .path(one_core::storage::NAVOP_ROCKETMQ_COLOR_ICON)
                                        .color()
                                        .with_size(IconSize::Large),
                                )
                                .compact(),
                        )
                    })
                    .when(entry_count > 0, |this| {
                        this.child(
                            uniform_list(
                                "rocketmq-tree-list",
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

    fn broker(name: &str) -> BrokerInfo {
        BrokerInfo {
            name: name.to_string(),
            address: format!("{name}:10911"),
            ..Default::default()
        }
    }

    fn topic(name: &str) -> MiddlewareTopicInfo {
        MiddlewareTopicInfo {
            name: name.to_string(),
            queue_count: Some(8),
            ..Default::default()
        }
    }

    #[test]
    fn build_flat_entries_expands_children_only_when_expanded() {
        let order = vec!["1".to_string()];
        let brokers = HashMap::from([("1".to_string(), vec![broker("broker-a")])]);
        let topics = HashMap::from([(
            "1".to_string(),
            vec![topic("order-topic"), topic("pay-topic")],
        )]);

        let expanded = HashSet::from(["1".to_string()]);
        let entries = build_flat_entries(&order, &brokers, &topics, &expanded);
        assert_eq!(4, entries.len());
        assert_eq!(entries[0].node_id, "1");
        assert_eq!(entries[0].kind, FlatEntryKind::Connection);
        assert_eq!(entries[1].node_id, "1:broker:broker-a");
        assert_eq!(entries[1].kind, FlatEntryKind::Broker);
        assert_eq!(entries[1].depth, 1);
        assert_eq!(entries[2].node_id, "1:topic:order-topic");
        assert_eq!(entries[3].node_id, "1:topic:pay-topic");

        // 折叠后只剩连接行
        let collapsed = HashSet::new();
        assert_eq!(
            1,
            build_flat_entries(&order, &brokers, &topics, &collapsed).len()
        );
    }

    #[test]
    fn build_flat_entries_handles_empty_metadata() {
        let order = vec!["1".to_string(), "2".to_string()];
        let entries = build_flat_entries(&order, &HashMap::new(), &HashMap::new(), &HashSet::new());
        assert_eq!(2, entries.len());
        assert!(entries.iter().all(|e| e.kind == FlatEntryKind::Connection));
    }
}
