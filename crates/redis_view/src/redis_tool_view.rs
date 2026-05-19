//! Redis 连接级工具标签页视图。

use crate::GlobalRedisState;
use crate::redis_pubsub::{PubSubMessage, SubscriptionCommand};
use crate::redis_tool_data::{RedisToolKind, ToolRow, load_tool_rows};
use crate::redis_tool_pages::build_table_rows;
use crate::redis_tool_table::RedisToolTableDelegate;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
};
use gpui_component::{
    Icon, Sizable, Size,
    input::{InputEvent, InputState},
    table::TableState,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use std::collections::VecDeque;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) const AUTO_REFRESH_SECONDS: u64 = 5;
const MAX_CHART_SAMPLES: usize = 60;
pub(crate) const MAX_RECEIVED_MESSAGES: usize = 1000;

#[derive(Clone, Debug)]
pub(crate) enum LoadState {
    Empty,
    Loading,
    Loaded(Vec<ToolRow>),
    Error(String),
}

#[derive(Clone, Debug)]
pub(crate) enum ActionState {
    Idle,
    Pending(String),
    Success(String),
    Error(String),
}

/// 发布订阅页签的 Body 视图切换。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PubSubBodyTab {
    /// 服务器现有频道/订阅数监控表格
    #[default]
    Channels,
    /// 本地订阅收到的消息流
    Messages,
}

pub struct RedisToolView {
    pub(crate) kind: RedisToolKind,
    pub(crate) connection_id: Option<String>,
    pub(crate) db_index: u8,
    pub(crate) load_state: LoadState,
    pub(crate) action_state: ActionState,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) publish_channel_input: Entity<InputState>,
    pub(crate) publish_message_input: Entity<InputState>,
    pub(crate) subscribe_input: Entity<InputState>,
    pub(crate) filter_text: String,
    pub(crate) auto_refresh: bool,
    pub(crate) auto_refresh_scheduled: bool,
    pub(crate) refresh_generation: u64,
    pub(crate) chart_history: Vec<Vec<ToolRow>>,
    pub(crate) table_state: Option<Entity<TableState<RedisToolTableDelegate>>>,
    pub(crate) focus_handle: FocusHandle,
    /// 已订阅的频道列表
    pub(crate) subscribed_channels: Vec<String>,
    /// 已订阅的模式列表
    pub(crate) subscribed_patterns: Vec<String>,
    /// 收到的消息环形缓冲(FIFO,上限 [`MAX_RECEIVED_MESSAGES`])
    pub(crate) received_messages: VecDeque<PubSubMessage>,
    /// 用于把指令投递到后台监听 task;None 表示尚未启动
    pub(crate) subscribe_cmd_tx: Option<UnboundedSender<SubscriptionCommand>>,
    /// 监听任务的代次,切换连接时递增以让旧 task 内的 update 闭包失效
    pub(crate) subscribe_generation: u64,
    /// 订阅相关的最近错误(如启动失败)
    pub(crate) subscribe_error: Option<String>,
    /// Body 视图(仅 PubSub 页签有效)
    pub(crate) body_tab: PubSubBodyTab,
}

impl RedisToolView {
    pub fn new(
        kind: RedisToolKind,
        connection_id: Option<String>,
        db_index: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("RedisTool.filter_placeholder").to_string())
        });
        let publish_channel_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("RedisPubSub.channel_placeholder").to_string())
        });
        let publish_message_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("RedisPubSub.message_placeholder").to_string())
        });
        let subscribe_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("RedisPubSub.subscribe_placeholder").to_string())
        });
        cx.subscribe_in(&filter_input, window, Self::on_filter_changed)
            .detach();
        cx.subscribe_in(&subscribe_input, window, Self::on_subscribe_input_event)
            .detach();

        let table_state = if kind == RedisToolKind::Chart {
            None
        } else {
            Some(cx.new(|cx| TableState::new(RedisToolTableDelegate::new(kind), window, cx)))
        };

        Self {
            kind,
            connection_id,
            db_index,
            load_state: LoadState::Empty,
            action_state: ActionState::Idle,
            filter_input,
            publish_channel_input,
            publish_message_input,
            subscribe_input,
            filter_text: String::new(),
            auto_refresh: kind == RedisToolKind::Chart,
            auto_refresh_scheduled: false,
            refresh_generation: 0,
            chart_history: Vec::new(),
            table_state,
            focus_handle: cx.focus_handle(),
            subscribed_channels: Vec::new(),
            subscribed_patterns: Vec::new(),
            received_messages: VecDeque::new(),
            subscribe_cmd_tx: None,
            subscribe_generation: 0,
            subscribe_error: None,
            body_tab: PubSubBodyTab::Channels,
        }
    }

    fn on_filter_changed(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.filter_text = self.filter_input.read(cx).text().to_string();
            self.sync_table(cx);
            cx.notify();
        }
    }

    /// Enter 触发订阅:含通配符 (*?[) 走 PSUBSCRIBE,否则普通 SUBSCRIBE。
    fn on_subscribe_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::PressEnter { .. }) {
            return;
        }
        let text = self.subscribe_input.read(cx).text().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if has_glob_metachar(trimmed) {
            self.subscribe_pattern(trimmed.to_string(), window, cx);
        } else {
            self.subscribe_channel(trimmed.to_string(), window, cx);
        }
    }

    pub fn set_connection(&mut self, connection_id: Option<String>, db_index: u8) {
        if self.connection_id == connection_id && self.db_index == db_index {
            return;
        }
        self.connection_id = connection_id;
        self.db_index = db_index;
        self.load_state = LoadState::Empty;
        self.action_state = ActionState::Idle;
        self.chart_history.clear();
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        // 连接切换时,自动断开旧订阅
        self.reset_subscription_state();
    }

    /// 清理订阅状态,使下一次订阅时重新连接。同时让消息消费 task 在
    /// 检测到代次不匹配时自动退出。
    pub(crate) fn reset_subscription_state(&mut self) {
        self.subscribe_generation = self.subscribe_generation.wrapping_add(1);
        self.subscribe_cmd_tx = None; // drop -> handle 在后台被 drop -> task 退出
        self.subscribed_channels.clear();
        self.subscribed_patterns.clear();
        self.received_messages.clear();
        self.subscribe_error = None;
        self.body_tab = PubSubBodyTab::Channels;
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id.clone() else {
            self.load_state = LoadState::Empty;
            self.sync_table(cx);
            cx.notify();
            return;
        };

        let kind = self.kind;
        let db_index = self.db_index;
        let global_state = cx.global::<GlobalRedisState>().clone();
        self.load_state = LoadState::Loading;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                let conn = global_state.get_connection(&connection_id).ok_or_else(|| {
                    anyhow::anyhow!(t!("RedisTool.connection_not_ready").to_string())
                })?;
                let guard = conn.read().await;
                load_tool_rows(kind, db_index, &**guard).await
            })
            .await;

            _ = this.update(cx, |view, cx| {
                view.load_state = match result {
                    Ok(rows) => {
                        if view.kind == RedisToolKind::Chart {
                            view.chart_history.push(rows.clone());
                            if view.chart_history.len() > MAX_CHART_SAMPLES {
                                view.chart_history.remove(0);
                            }
                        }
                        LoadState::Loaded(rows)
                    }
                    Err(error) => LoadState::Error(format!("{error:#}")),
                };
                view.sync_table(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 把当前 load_state + filter_text 转换为表格单元格,并写入共享 TableState。
    pub(crate) fn sync_table(&mut self, cx: &mut Context<Self>) {
        let Some(table_state) = self.table_state.clone() else {
            return;
        };
        let filtered = self.filtered_rows();
        let cells = build_table_rows(self.kind, &filtered);
        table_state.update(cx, |state, cx| {
            state.delegate_mut().set_rows(cells);
            cx.notify();
        });
    }

    /// 全量行(未过滤),供 header 汇总指标使用。
    pub(crate) fn all_rows(&self) -> Vec<ToolRow> {
        let LoadState::Loaded(rows) = &self.load_state else {
            return Vec::new();
        };
        rows.clone()
    }

    /// 获取经过 filter_text 过滤后的 ToolRow 列表;表格单元格使用此函数,
    /// header 渲染请改用 [`all_rows`]。
    pub(crate) fn filtered_rows(&self) -> Vec<ToolRow> {
        let LoadState::Loaded(rows) = &self.load_state else {
            return Vec::new();
        };
        let filter = self.filter_text.trim().to_lowercase();
        if filter.is_empty() {
            return rows.clone();
        }
        rows.iter()
            .filter(|row| {
                row.category.to_lowercase().contains(&filter)
                    || row.key.to_lowercase().contains(&filter)
                    || row.value.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect()
    }

    /// 切换 PubSub body tab。
    pub(crate) fn set_pubsub_body_tab(&mut self, tab: PubSubBodyTab, cx: &mut Context<Self>) {
        if self.body_tab != tab {
            self.body_tab = tab;
            cx.notify();
        }
    }
}

/// 频道字符串是否包含 Redis pattern 通配符。
pub(crate) fn has_glob_metachar(value: &str) -> bool {
    let mut prev_escape = false;
    for ch in value.chars() {
        if prev_escape {
            prev_escape = false;
            continue;
        }
        match ch {
            '\\' => prev_escape = true,
            '*' | '?' | '[' => return true,
            _ => {}
        }
    }
    false
}

impl Focusable for RedisToolView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for RedisToolView {}

impl TabContent for RedisToolView {
    fn content_key(&self) -> &'static str {
        "RedisTool"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.kind.title().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(self.icon_name()).with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.load_state, LoadState::Empty) {
            self.refresh(cx);
        }
        if self.auto_refresh {
            self.schedule_auto_refresh(cx);
        }
    }
}

impl Render for RedisToolView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("redis-tool-view").size_full().child(
            v_flex()
                .size_full()
                .child(self.render_toolbar(cx))
                .when(self.kind == RedisToolKind::PubSub, |this| {
                    this.child(self.render_pubsub_actions(cx))
                })
                .child(self.render_action_status(cx))
                .child(self.render_body(cx)),
        )
    }
}
