//! 中间件消息查询页(RocketMQ Console Message 页风格)。
//!
//! - 查询条件三选一:Topic+时间范围 | Topic+Key | Topic+MsgId
//! - 结果表格:MsgId | Tag | Key | 存储时间 | 操作(详情/重发)
//! - 消息详情弹窗:全字段 + 属性列表 + body 文本/Hex 切换
//! - 分页:page/page_size 随查询条件传给后端(本地缓冲型后端按同语义过滤)

use chrono::{Local, NaiveDateTime};

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize, Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    notification::Notification,
    select::{Select, SelectEvent, SelectItem, SelectState},
    switch::Switch,
    table::{Column, ColumnFixed, DataTable, TableDelegate, TableState},
    v_flex,
};
use middleware_runtime::{
    MessagePage as BackendPage, MessageQuery, MiddlewareMessage, SendMessageRequest,
};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use tracing::warn;

use crate::MiddlewareAdminHandle;
use crate::common::{
    LoadState, PaginationSpec, hex_dump, notify_async, page_header, pagination_bar,
    render_table_empty, text_cell,
};

/// 查询方式
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryMode {
    /// Topic + 时间范围
    #[default]
    TimeWindow,
    /// Topic + Key
    Key,
    /// Topic + MsgId
    Id,
}

/// 查询方式在 Select 中的值键(与 [`QueryMode`] 下标一致)
const MODE_KEYS: [&str; 3] = ["time", "key", "id"];

/// 查询方式文本 -> 枚举
fn mode_from_value(value: &str) -> QueryMode {
    match value {
        "key" => QueryMode::Key,
        "id" => QueryMode::Id,
        _ => QueryMode::TimeWindow,
    }
}

impl QueryMode {
    fn label(self) -> SharedString {
        match self {
            Self::TimeWindow => t!("Middleware.mode_time").to_string().into(),
            Self::Key => t!("Middleware.mode_key").to_string().into(),
            Self::Id => t!("Middleware.mode_id").to_string().into(),
        }
    }
}

/// 查询方式下拉选项
#[derive(Clone, Debug, PartialEq, Eq)]
struct QueryModeItem(QueryMode);

impl SelectItem for QueryModeItem {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.0.label()
    }

    fn value(&self) -> &Self::Value {
        &MODE_KEYS[self.0 as usize]
    }
}

/// 每页条数下拉选项
#[derive(Clone, Debug, PartialEq, Eq)]
struct PageSizeItem(u32);

impl SelectItem for PageSizeItem {
    type Value = u32;

    fn title(&self) -> SharedString {
        self.0.to_string().into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// 时间输入解析("YYYY-MM-DD HH:MM",本地时区 -> Unix 毫秒)
fn parse_local_datetime(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|naive| naive.and_local_timezone(Local).single())
        .map(|datetime| datetime.timestamp_millis())
}

/// 默认开始时间(现在 - 1 小时,"YYYY-MM-DD HH:MM")
fn default_begin_time() -> String {
    (Local::now() - chrono::Duration::hours(1))
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// 默认结束时间(现在)
fn default_end_time() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// 消息结果表格列数
const COLUMN_COUNT: usize = 5;
/// 操作列下标
const ACTIONS_COL: usize = COLUMN_COUNT - 1;

/// 消息结果表格 delegate
struct MessagesTableDelegate {
    /// 列定义
    columns: Vec<Column>,
    /// 当前结果行
    rows: Vec<MiddlewareMessage>,
    /// 所属页面(操作按钮回调)
    page: Entity<MiddlewareMessagesPage>,
    /// 是否可重发(send_message 能力位)
    can_resend: bool,
}

impl MessagesTableDelegate {
    fn columns() -> Vec<Column> {
        vec![
            Column::new("msgid", t!("Middleware.col_msgid").to_string())
                .width(px(220.0))
                .min_width(px(140.0))
                .max_width(px(320.0))
                .fixed(ColumnFixed::Left),
            Column::new("tag", t!("Middleware.col_tag").to_string())
                .width(px(110.0))
                .min_width(px(80.0)),
            Column::new("key", t!("Middleware.col_key").to_string())
                .width(px(140.0))
                .min_width(px(100.0)),
            Column::new("store_time", t!("Middleware.col_store_time").to_string())
                .width(px(170.0))
                .min_width(px(140.0)),
            Column::new("actions", t!("Middleware.col_actions").to_string())
                .width(px(170.0))
                .min_width(px(150.0)),
        ]
    }
}

impl TableDelegate for MessagesTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(message) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        if col_ix == ACTIONS_COL {
            let detail_view = self.page.clone();
            let detail_message = message.clone();
            let resend_view = self.page.clone();
            let resend_message = message.clone();
            return h_flex()
                .id(SharedString::from(format!("messages-row-{row_ix}")))
                .gap_1()
                .items_center()
                .child(
                    Button::new(SharedString::from(format!("message-detail-{row_ix}")))
                        .xsmall()
                        .ghost()
                        .label(t!("Middleware.action_detail").to_string())
                        .on_click(move |_, window, cx| {
                            detail_view.update(cx, |page, cx| {
                                page.open_detail_dialog(&detail_message, window, cx);
                            });
                        }),
                )
                .when(self.can_resend, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("message-resend-{row_ix}")))
                            .xsmall()
                            .ghost()
                            .label(t!("Middleware.action_resend").to_string())
                            .on_click(move |_, window, cx| {
                                resend_view.update(cx, |page, cx| {
                                    page.open_resend_dialog(&resend_message, window, cx);
                                });
                            }),
                    )
                })
                .into_any_element();
        }
        let text = match col_ix {
            0 => message.message_id.clone(),
            1 => message.tag.clone().unwrap_or_else(|| "-".into()),
            2 => message.key.clone().unwrap_or_else(|| "-".into()),
            3 => message.store_time.clone().unwrap_or_else(|| "-".into()),
            _ => String::new(),
        };
        text_cell(text).into_any_element()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        render_table_empty(
            t!("Middleware.empty_title").to_string().into(),
            t!("Middleware.no_results").to_string().into(),
            cx,
        )
    }
}

/// 消息详情 body 面板(文本/Hex 切换;独立 Entity 以支持弹窗内状态刷新)
struct MessageBodyPanel {
    /// 原始 body
    body: Vec<u8>,
    /// 是否 Hex 模式
    hex: bool,
}

impl MessageBodyPanel {
    fn current_text(&self) -> String {
        if self.hex {
            hex_dump(&self.body)
        } else {
            String::from_utf8(self.body.clone()).unwrap_or_else(|_| hex_dump(&self.body))
        }
    }
}

impl Render for MessageBodyPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hex = self.hex;
        v_flex()
            .id("message-body-panel")
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(t!("Middleware.d_body").to_string()),
                    )
                    .child(
                        Switch::new("message-body-hex")
                            .small()
                            .checked(hex)
                            .label(if hex {
                                t!("Middleware.body_hex").to_string()
                            } else {
                                t!("Middleware.body_text").to_string()
                            })
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.hex = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("message-body-scroll")
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded(cx.theme().geometry.radius.sm)
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .child(self.current_text()),
            )
    }
}

/// 中间件消息查询页
pub struct MiddlewareMessagesPage {
    /// 管理接口句柄
    handle: MiddlewareAdminHandle,
    /// 查询方式
    mode: QueryMode,
    /// 查询结果
    result: BackendPage,
    /// 加载状态
    load_state: LoadState,
    /// 查询代号(递增使旧在途查询的回写失效)
    refresh_generation: u64,
    /// 当前页(1 起,查询时传给后端)
    page: u32,
    /// 每页条数(10/20/50)
    page_size: u32,
    // 查询输入
    topic_input: Entity<InputState>,
    begin_input: Entity<InputState>,
    end_input: Entity<InputState>,
    key_input: Entity<InputState>,
    msgid_input: Entity<InputState>,
    mode_select: Entity<SelectState<Vec<QueryModeItem>>>,
    page_size_select: Entity<SelectState<Vec<PageSizeItem>>>,
    /// 结果表格
    table: Entity<TableState<MessagesTableDelegate>>,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MiddlewareMessagesPage {
    /// 创建消息查询页
    pub fn new(handle: MiddlewareAdminHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let capabilities = handle.capabilities();
        let topic_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Middleware.field_topic").to_string())
        });
        let begin_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("Middleware.time_hint").to_string());
            state.set_value(default_begin_time(), window, cx);
            state
        });
        let end_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("Middleware.time_hint").to_string());
            state.set_value(default_end_time(), window, cx);
            state
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Middleware.field_key").to_string())
        });
        let msgid_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Middleware.field_msgid").to_string())
        });
        let mode_select = cx.new(|cx| {
            let items = vec![
                QueryModeItem(QueryMode::TimeWindow),
                QueryModeItem(QueryMode::Key),
                QueryModeItem(QueryMode::Id),
            ];
            let mut state = SelectState::new(items, None, window, cx);
            state.set_selected_value(&"time", window, cx);
            state
        });
        let page_size_select = cx.new(|cx| {
            let items = vec![PageSizeItem(10), PageSizeItem(20), PageSizeItem(50)];
            let mut state = SelectState::new(items, None, window, cx);
            state.set_selected_value(&10, window, cx);
            state
        });
        let page_entity = cx.entity();
        let table = cx.new(|cx| {
            TableState::new(
                MessagesTableDelegate {
                    columns: MessagesTableDelegate::columns(),
                    rows: Vec::new(),
                    page: page_entity.clone(),
                    can_resend: capabilities.send_message,
                },
                window,
                cx,
            )
        });

        // 查询方式切换时刷新输入区可见性
        cx.subscribe(
            &mode_select,
            |this, _entity, event: &SelectEvent<Vec<QueryModeItem>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.mode = mode_from_value(value);
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            handle,
            mode: QueryMode::TimeWindow,
            result: BackendPage::default(),
            load_state: LoadState::Idle,
            refresh_generation: 0,
            page: 1,
            page_size: 10,
            topic_input,
            begin_input,
            end_input,
            key_input,
            msgid_input,
            mode_select,
            page_size_select,
            table,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 组装当前查询条件(校验失败时弹通知并返回 None)
    fn build_query(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<MessageQuery> {
        let topic = self
            .topic_input
            .read(cx)
            .text()
            .to_string()
            .trim()
            .to_string();
        if topic.is_empty() {
            window.push_notification(
                Notification::error(t!("Middleware.topic_required").to_string()).autohide(true),
                cx,
            );
            return None;
        }
        match self.mode {
            QueryMode::TimeWindow => {
                let begin_ok = parse_local_datetime(&self.begin_input.read(cx).text().to_string());
                let end_ok = parse_local_datetime(&self.end_input.read(cx).text().to_string());
                match (begin_ok, end_ok) {
                    (Some(begin), Some(end)) => Some(MessageQuery::ByTimeWindow {
                        topic,
                        begin_unix_ms: begin,
                        end_unix_ms: end,
                        page: self.page,
                        page_size: self.page_size,
                    }),
                    _ => {
                        window.push_notification(
                            Notification::error(t!("Middleware.invalid_time").to_string())
                                .autohide(true),
                            cx,
                        );
                        None
                    }
                }
            }
            QueryMode::Key => {
                let key = self
                    .key_input
                    .read(cx)
                    .text()
                    .to_string()
                    .trim()
                    .to_string();
                if key.is_empty() {
                    window.push_notification(
                        Notification::error(t!("Middleware.key_required").to_string())
                            .autohide(true),
                        cx,
                    );
                    return None;
                }
                Some(MessageQuery::ByKey { topic, key })
            }
            QueryMode::Id => {
                let message_id = self
                    .msgid_input
                    .read(cx)
                    .text()
                    .to_string()
                    .trim()
                    .to_string();
                if message_id.is_empty() {
                    window.push_notification(
                        Notification::error(t!("Middleware.msgid_required").to_string())
                            .autohide(true),
                        cx,
                    );
                    return None;
                }
                Some(MessageQuery::ById { topic, message_id })
            }
        }
    }

    /// 按当前条件查询(重置到第 1 页)
    fn run_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = 1;
        self.read_page_size(cx);
        if let Some(query) = self.build_query(window, cx) {
            self.execute_query(query, cx);
        }
    }

    /// 翻页后按当前条件查询
    fn run_query_page(&mut self, new_page: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.page = new_page;
        if let Some(query) = self.build_query(window, cx) {
            self.execute_query(query, cx);
        }
    }

    /// 读取每页条数下拉当前值
    fn read_page_size(&mut self, cx: &Context<Self>) {
        if let Some(size) = self.page_size_select.read(cx).selected_value() {
            self.page_size = *size;
        }
    }

    /// 执行查询并回填结果(回写带 generation 防护,旧查询晚到不覆盖新结果)
    fn execute_query(&mut self, query: MessageQuery, cx: &mut Context<Self>) {
        self.load_state = LoadState::Loading;
        // 递增代号使旧的在途查询回写失效
        let generation = self.refresh_generation.wrapping_add(1);
        self.refresh_generation = generation;
        let handle = self.handle.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                handle
                    .query_messages(query)
                    .await
                    .map_err(anyhow::Error::new)
            })
            .await;
            // generation 不匹配说明期间又发起了新查询,过期结果直接丢弃
            let stale = |view: &Self| view.refresh_generation != generation;
            match result {
                Ok(page) => {
                    _ = this.update(cx, |view, cx| {
                        if stale(view) {
                            return;
                        }
                        view.result = page;
                        view.load_state = LoadState::Loaded;
                        view.sync_table(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "消息查询失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.query_failed", error = message).to_string(),
                        ),
                    );
                    _ = this.update(cx, |view, cx| {
                        if stale(view) {
                            return;
                        }
                        view.load_state = LoadState::Failed(message);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 把结果写入表格
    fn sync_table(&mut self, cx: &mut Context<Self>) {
        let rows = self.result.messages.clone();
        let table = self.table.clone();
        table.update(cx, |state, cx| {
            state.delegate_mut().rows = rows;
            cx.notify();
        });
    }

    /// 打开消息详情弹窗(优先用后端 message_detail 拉全量字段,失败回退行数据)
    fn open_detail_dialog(
        &mut self,
        message: &MiddlewareMessage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.handle.clone();
        let topic = message.topic.clone();
        let message_id = message.message_id.clone();
        let fallback = message.clone();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let topic = topic.clone();
                let message_id = message_id.clone();
                async move {
                    handle
                        .message_detail(&topic, &message_id)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            // 详情拉取失败时回退到列表行数据(列表已含核心字段)
            let detail = result.unwrap_or(fallback);
            let _ = cx.update(|cx| {
                if let Some(window) = cx.active_window() {
                    _ = window.update(cx, |_, window, cx| {
                        Self::show_detail_dialog(&detail, window, cx)
                    });
                }
            });
        })
        .detach();
    }

    /// 渲染消息详情弹窗
    fn show_detail_dialog(message: &MiddlewareMessage, window: &mut Window, cx: &mut App) {
        let body = message.body.clone().unwrap_or_default();
        let body_panel = cx.new(|_| MessageBodyPanel { body, hex: false });
        let rows = vec![
            (
                t!("Middleware.col_msgid").to_string(),
                message.message_id.clone(),
            ),
            (
                t!("Middleware.field_topic").to_string(),
                message.topic.clone(),
            ),
            (
                t!("Middleware.col_tag").to_string(),
                message.tag.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.col_key").to_string(),
                message.key.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.d_store_host").to_string(),
                message.store_host.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.d_born_host").to_string(),
                message.born_host.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.col_store_time").to_string(),
                message.store_time.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.d_born_time").to_string(),
                message.born_time.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.d_retry").to_string(),
                message
                    .retry_times
                    .map(|times| times.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
        ];
        let properties = message.properties.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("Middleware.detail_title").to_string())
                .child(
                    v_flex()
                        .id("message-detail-scroll")
                        .min_w(px(520.0))
                        .gap_2()
                        .max_h(px(480.0))
                        .overflow_y_scroll()
                        .child(
                            v_flex().gap_1().children(
                                rows.iter()
                                    .map(|(label, value)| {
                                        h_flex().gap_2().child(
                                            div().text_xs().child(format!("{label}: {value}")),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(t!("Middleware.d_props").to_string()),
                                )
                                .when(properties.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .child(t!("Middleware.no_props").to_string()),
                                    )
                                })
                                .when(!properties.is_empty(), |this| {
                                    this.child(
                                        v_flex().gap_0p5().children(
                                            properties
                                                .iter()
                                                .map(|(key, value)| {
                                                    div().text_xs().child(format!("{key}: {value}"))
                                                })
                                                .collect::<Vec<_>>(),
                                        ),
                                    )
                                }),
                        )
                        .child(body_panel.clone()),
                )
        });
    }

    /// 打开重发弹窗(topic/tag/key/body 预填)
    fn open_resend_dialog(
        &mut self,
        message: &MiddlewareMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().clone();
        let topic = message.topic.clone();
        let tag = message.tag.clone().unwrap_or_default();
        let key = message.key.clone().unwrap_or_default();
        let body = message.body.clone().unwrap_or_default();
        window.open_dialog(cx, move |dialog, window, cx| {
            let topic_input = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .placeholder(t!("Middleware.field_topic").to_string());
                state.set_value(topic.clone(), window, cx);
                state
            });
            let tag_input = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder(t!("Middleware.field_tag").to_string());
                state.set_value(tag.clone(), window, cx);
                state
            });
            let key_input = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder(t!("Middleware.field_key").to_string());
                state.set_value(key.clone(), window, cx);
                state
            });
            let body_input = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx)
                    .placeholder(t!("Middleware.body_placeholder").to_string())
                    .auto_grow(3, 8);
                if let Ok(text) = std::str::from_utf8(&body) {
                    state.set_value(text.to_string(), window, cx);
                }
                state
            });
            let submit_view = view.clone();
            dialog
                .title(t!("Middleware.resend_title").to_string())
                .child(
                    v_flex()
                        .min_w(px(460.0))
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .child(t!("Middleware.field_topic").to_string()),
                                )
                                .child(div().flex_1().child(Input::new(&topic_input))),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&tag_input))),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&key_input))),
                        )
                        .child(Textarea::new(&body_input)),
                )
                .footer(
                    gpui_component::dialog::DialogFooter::new()
                        .child(
                            Button::new("resend-cancel")
                                .label(t!("Middleware.cancel").to_string())
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child({
                            let topic_input = topic_input.clone();
                            let tag_input = tag_input.clone();
                            let key_input = key_input.clone();
                            let body_input = body_input.clone();
                            Button::new("resend-submit")
                                .primary()
                                .label(t!("Middleware.action_resend").to_string())
                                .on_click(move |_, window, cx| {
                                    let topic =
                                        topic_input.read(cx).text().to_string().trim().to_string();
                                    let tag =
                                        tag_input.read(cx).text().to_string().trim().to_string();
                                    let key =
                                        key_input.read(cx).text().to_string().trim().to_string();
                                    let body = body_input.read(cx).text().to_string().into_bytes();
                                    window.close_dialog(cx);
                                    submit_view.update(cx, |page, cx| {
                                        page.perform_send(topic, tag, key, body, cx);
                                    });
                                })
                        }),
                )
        });
    }

    /// 执行发送/重发
    fn perform_send(
        &mut self,
        topic: String,
        tag: String,
        key: String,
        body: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        if topic.is_empty() {
            return;
        }
        let request = SendMessageRequest {
            topic,
            tag: (!tag.is_empty()).then_some(tag),
            key: (!key.is_empty()).then_some(key),
            body,
            properties: Vec::new(),
        };
        let handle = self.handle.clone();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let request = request.clone();
                async move {
                    handle
                        .send_message(request)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            match result {
                Ok(result) => {
                    notify_async(
                        cx,
                        Notification::success(
                            t!("Middleware.send_ok", id = result.message_id).to_string(),
                        ),
                    );
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "消息发送失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.send_failed", error = message).to_string(),
                        ),
                    );
                }
            }
        })
        .detach();
    }
}

impl Focusable for MiddlewareMessagesPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for MiddlewareMessagesPage {}

impl TabContent for MiddlewareMessagesPage {
    fn content_key(&self) -> &'static str {
        "middleware-messages"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("Middleware.messages_tab").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Monitor).with_size(IconSize::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        Task::ready(true)
    }
}

impl Render for MiddlewareMessagesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        let view = cx.entity().clone();
        let mode = self.mode;
        let page = self.page;
        let page_size = self.page_size;
        let total = self.result.total;
        let has_more = self.result.has_more;
        let loading = self.load_state == LoadState::Loading;

        v_flex()
            .id("middleware-messages-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .child(page_header(
                "middleware-messages",
                IconName::Monitor,
                t!("Middleware.messages_tab").to_string().into(),
                border_color,
                {
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |view, cx| view.run_query(window, cx));
                    }
                },
            ))
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .border_b_1()
                    .border_color(border_color)
                    .child(div().w(px(140.0)).child(Select::new(&self.mode_select)))
                    .child(div().w(px(220.0)).child(Input::new(&self.topic_input)))
                    .when(mode == QueryMode::TimeWindow, |this| {
                        this.child(div().w(px(150.0)).child(Input::new(&self.begin_input)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("~"),
                            )
                            .child(div().w(px(150.0)).child(Input::new(&self.end_input)))
                    })
                    .when(mode == QueryMode::Key, |this| {
                        this.child(div().w(px(180.0)).child(Input::new(&self.key_input)))
                    })
                    .when(mode == QueryMode::Id, |this| {
                        this.child(div().w(px(220.0)).child(Input::new(&self.msgid_input)))
                    })
                    .child(div().w(px(90.0)).child(Select::new(&self.page_size_select)))
                    .child(
                        Button::new("messages-query")
                            .small()
                            .primary()
                            .label(t!("Middleware.query_btn").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run_query(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .when(loading, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Middleware.loading").to_string()),
                        )
                    })
                    .when(has_more, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Middleware.more_results").to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(DataTable::new(&self.table).stripe(true)),
            )
            .child(pagination_bar(
                "messages",
                PaginationSpec {
                    total,
                    page: page as usize,
                    page_size: page_size as usize,
                    has_more,
                },
                border_color,
                {
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |view, cx| {
                            if view.page > 1 {
                                view.run_query_page(view.page - 1, window, cx);
                            }
                        });
                    }
                },
                {
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |view, cx| {
                            view.run_query_page(view.page + 1, window, cx);
                        });
                    }
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local_datetime_accepts_expected_format() {
        let ms = parse_local_datetime("2025-01-02 08:00");
        assert!(ms.is_some());
        // 对应本地 2025-01-02 08:00 的毫秒时间戳应与 chrono 反算一致
        let back = NaiveDateTime::parse_from_str("2025-01-02 08:00", "%Y-%m-%d %H:%M")
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(ms, Some(back));
    }

    #[test]
    fn parse_local_datetime_rejects_invalid() {
        assert_eq!(parse_local_datetime(""), None);
        assert_eq!(parse_local_datetime("2025/01/02"), None);
        assert_eq!(parse_local_datetime("not-a-time"), None);
    }

    #[test]
    fn mode_roundtrip_from_value() {
        assert_eq!(mode_from_value("time"), QueryMode::TimeWindow);
        assert_eq!(mode_from_value("key"), QueryMode::Key);
        assert_eq!(mode_from_value("id"), QueryMode::Id);
        assert_eq!(mode_from_value("unknown"), QueryMode::TimeWindow);
    }

    #[test]
    fn default_time_window_is_one_hour() {
        let begin = parse_local_datetime(&default_begin_time()).expect("默认开始时间应可解析");
        let end = parse_local_datetime(&default_end_time()).expect("默认结束时间应可解析");
        // 间隔应接近 1 小时(允许分界处 1 分钟误差)
        let diff = end - begin;
        assert!((3_540_000..=3_660_000).contains(&diff), "间隔异常: {diff}");
    }
}
