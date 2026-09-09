//! 中间件 Topic 管理页(RocketMQ Console Topic 页风格)。
//!
//! - 顶部筛选:名称输入 + 类型多选(标签点选切换)
//! - 表格:Topic | 类型 | 队列数 | 权限 | 消息数 | 创建时间 | 操作
//! - 操作:队列状态弹窗 / Topic 配置编辑(topic_write) / 发送消息(send_message) / 删除(topic_write)
//! - 分页:本地分页(10/20/50 可选)

use std::collections::HashSet;

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
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    notification::Notification,
    select::{Select, SelectEvent, SelectItem, SelectState},
    table::{Column, ColumnFixed, DataTable, TableDelegate, TableState},
    v_flex,
};
use middleware_runtime::{
    CreateTopicRequest, MiddlewareCapabilities, MiddlewareTopicInfo, SendMessageRequest,
    TopicDetail,
};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use tracing::warn;

use crate::MiddlewareAdminHandle;
use crate::common::{
    LoadState, PageSizeItem, PaginationSpec, notify_async, page_header, page_size_select,
    pagination_bar, render_table_empty, text_cell,
};

/// Topic 类型过滤候选(通用文本)
pub const TOPIC_TYPE_FILTERS: [&str; 7] = [
    "NORMAL",
    "FIFO",
    "DELAY",
    "TRANSACTION",
    "RETRY",
    "DLQ",
    "SYSTEM",
];

/// Topic 表格列数
const COLUMN_COUNT: usize = 7;
/// 操作列下标
const ACTIONS_COL: usize = COLUMN_COUNT - 1;

/// Topic 类型下拉选项(编辑弹窗用)
#[derive(Clone, Debug, PartialEq, Eq)]
struct TopicTypeItem(String);

impl SelectItem for TopicTypeItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.0.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// 权限下拉选项(6=读写 4=只读 2=只写)
#[derive(Clone, Debug, PartialEq, Eq)]
struct PermItem(String, String);

impl SelectItem for PermItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.1.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// Topic 页表格 delegate
struct TopicsTableDelegate {
    /// 列定义
    columns: Vec<Column>,
    /// 当前页可见行
    rows: Vec<MiddlewareTopicInfo>,
    /// 所属页面(操作按钮回调)
    page: Entity<MiddlewareTopicsPage>,
    /// 是否可写(topic_write)
    can_write: bool,
    /// 是否可发送(send_message)
    can_send: bool,
}

impl TopicsTableDelegate {
    fn columns() -> Vec<Column> {
        vec![
            Column::new("topic", t!("Middleware.col_topic").to_string())
                .width(px(280.0))
                .min_width(px(160.0))
                .max_width(px(480.0))
                .fixed(ColumnFixed::Left),
            Column::new("type", t!("Middleware.col_type").to_string())
                .width(px(110.0))
                .min_width(px(80.0))
                .max_width(px(160.0)),
            Column::new("queues", t!("Middleware.col_queues").to_string())
                .width(px(90.0))
                .min_width(px(70.0)),
            Column::new("perm", t!("Middleware.col_perm").to_string())
                .width(px(80.0))
                .min_width(px(70.0)),
            Column::new("messages", t!("Middleware.col_messages").to_string())
                .width(px(110.0))
                .min_width(px(90.0)),
            Column::new("created", t!("Middleware.col_created").to_string())
                .width(px(170.0))
                .min_width(px(140.0)),
            Column::new("actions", t!("Middleware.col_actions").to_string())
                .width(px(250.0))
                .min_width(px(220.0)),
        ]
    }
}

impl TableDelegate for TopicsTableDelegate {
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
        let Some(topic) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        if col_ix == ACTIONS_COL {
            let page = self.page.clone();
            let name = topic.name.clone();
            let can_write = self.can_write;
            let can_send = self.can_send;
            let row = SharedString::from(format!("topics-row-{row_ix}"));
            let status_view = page.clone();
            let status_name = name.clone();
            let edit_view = page.clone();
            let edit_name = name.clone();
            let send_view = page.clone();
            let send_name = name.clone();
            let delete_view = page.clone();
            let delete_name = name.clone();
            return h_flex()
                .id(row)
                .gap_1()
                .items_center()
                .child(
                    Button::new(SharedString::from(format!("topics-status-{row_ix}")))
                        .xsmall()
                        .ghost()
                        .label(t!("Middleware.action_status").to_string())
                        .on_click(move |_, window, cx| {
                            status_view.update(cx, |page, cx| {
                                page.open_status_dialog(&status_name, window, cx);
                            });
                        }),
                )
                .when(can_write, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("topics-edit-{row_ix}")))
                            .xsmall()
                            .ghost()
                            .label(t!("Middleware.action_edit").to_string())
                            .on_click(move |_, window, cx| {
                                edit_view.update(cx, |page, cx| {
                                    page.open_editor_dialog(Some(&edit_name), window, cx);
                                });
                            }),
                    )
                })
                .when(can_send, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("topics-send-{row_ix}")))
                            .xsmall()
                            .ghost()
                            .label(t!("Middleware.action_send").to_string())
                            .on_click(move |_, window, cx| {
                                send_view.update(cx, |page, cx| {
                                    page.open_send_dialog(&send_name, None, window, cx);
                                });
                            }),
                    )
                })
                .when(can_write, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("topics-delete-{row_ix}")))
                            .xsmall()
                            .ghost()
                            .danger()
                            .label(t!("Middleware.action_delete").to_string())
                            .on_click(move |_, window, cx| {
                                delete_view.update(cx, |page, cx| {
                                    page.confirm_delete_topic(&delete_name, window, cx);
                                });
                            }),
                    )
                })
                .into_any_element();
        }
        let text = match col_ix {
            0 => topic.name.clone(),
            1 => topic.topic_type.clone().unwrap_or_else(|| "-".into()),
            2 => topic
                .queue_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".into()),
            3 => topic.perm.clone().unwrap_or_else(|| "-".into()),
            4 => topic
                .message_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".into()),
            5 => topic.created_at.clone().unwrap_or_else(|| "-".into()),
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
            t!("Middleware.topics_empty_detail").to_string().into(),
            cx,
        )
    }
}

/// 中间件 Topic 管理页
pub struct MiddlewareTopicsPage {
    /// 管理接口句柄
    handle: MiddlewareAdminHandle,
    /// 能力位快照
    capabilities: MiddlewareCapabilities,
    /// 全量 Topic 列表
    topics: Vec<MiddlewareTopicInfo>,
    /// 加载状态
    load_state: LoadState,
    /// 刷新代号(递增使旧在途刷新的回写失效)
    refresh_generation: u64,
    /// 名称过滤词
    filter_text: String,
    /// 激活的类型过滤(空集=全部)
    active_types: HashSet<&'static str>,
    /// 名称过滤输入
    filter_input: Entity<InputState>,
    /// 每页条数下拉
    page_size_select: Entity<SelectState<Vec<PageSizeItem>>>,
    /// 表格状态
    table: Entity<TableState<TopicsTableDelegate>>,
    /// 当前页(1 起)
    page: usize,
    /// 每页条数
    page_size: usize,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MiddlewareTopicsPage {
    /// 创建 Topic 页并立即加载
    pub fn new(handle: MiddlewareAdminHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let capabilities = handle.capabilities();
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Middleware.filter_name_placeholder"))
        });
        let page_entity = cx.entity();
        let table = cx.new(|cx| {
            TableState::new(
                TopicsTableDelegate {
                    columns: TopicsTableDelegate::columns(),
                    rows: Vec::new(),
                    page: page_entity.clone(),
                    can_write: capabilities.topic_write,
                    can_send: capabilities.send_message,
                },
                window,
                cx,
            )
        });

        cx.subscribe_in(&filter_input, window, |this, _, event, _window, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter_text = this.filter_input.read(cx).text().to_string();
                this.page = 1;
                this.sync_table(cx);
                cx.notify();
            }
        })
        .detach();

        let page_size_select = page_size_select(window, cx);
        cx.subscribe(
            &page_size_select,
            |this, _, event: &SelectEvent<Vec<PageSizeItem>>, cx| {
                if let SelectEvent::Confirm(Some(size)) = event {
                    this.page_size = *size;
                    this.page = 1;
                    this.sync_table(cx);
                    cx.notify();
                }
            },
        )
        .detach();

        let mut this = Self {
            handle,
            capabilities,
            topics: Vec::new(),
            load_state: LoadState::Idle,
            refresh_generation: 0,
            filter_text: String::new(),
            active_types: HashSet::new(),
            filter_input,
            page_size_select,
            table,
            page: 1,
            page_size: 10,
            focus_handle: cx.focus_handle(),
        };
        this.refresh(cx);
        this
    }

    /// 名称/类型过滤后的全量行
    fn filtered_topics(&self) -> Vec<&MiddlewareTopicInfo> {
        let needle = self.filter_text.trim().to_lowercase();
        self.topics
            .iter()
            .filter(|topic| {
                let name_match = needle.is_empty() || topic.name.to_lowercase().contains(&needle);
                let type_match = self.active_types.is_empty()
                    || topic
                        .topic_type
                        .as_deref()
                        .is_some_and(|ty| self.active_types.contains(ty));
                name_match && type_match
            })
            .collect()
    }

    /// 总页数
    fn pages(&self, total: usize) -> usize {
        total.div_ceil(self.page_size).max(1)
    }

    /// 把过滤+分页后的行写入表格
    fn sync_table(&mut self, cx: &mut Context<Self>) {
        let filtered: Vec<MiddlewareTopicInfo> =
            self.filtered_topics().into_iter().cloned().collect();
        let total = filtered.len();
        self.page = self.page.min(self.pages(total));
        let start = self.page.saturating_sub(1) * self.page_size;
        let rows: Vec<MiddlewareTopicInfo> = filtered
            .into_iter()
            .skip(start)
            .take(self.page_size)
            .collect();
        let table = self.table.clone();
        table.update(cx, |state, cx| {
            state.delegate_mut().rows = rows;
            cx.notify();
        });
    }

    /// 刷新 Topic 列表(回写带 generation 防护,旧请求晚到不覆盖新数据)
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_state = LoadState::Loading;
        // 递增代号使旧的在途刷新回写失效
        let generation = self.refresh_generation.wrapping_add(1);
        self.refresh_generation = generation;
        let handle = self.handle.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                handle.list_topics().await.map_err(anyhow::Error::new)
            })
            .await;
            // generation 不匹配说明期间又发起了新刷新,过期数据直接丢弃
            let stale = |view: &Self| view.refresh_generation != generation;
            match result {
                Ok(topics) => {
                    _ = this.update(cx, |view, cx| {
                        if stale(view) {
                            return;
                        }
                        view.topics = topics;
                        view.load_state = LoadState::Loaded;
                        view.page = 1;
                        view.sync_table(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "Topic 列表加载失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.load_failed", error = message).to_string(),
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

    /// 打开队列状态弹窗(先拉取 topic_detail)
    fn open_status_dialog(&mut self, topic: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle.clone();
        let topic = topic.to_string();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let topic = topic.clone();
                async move {
                    handle
                        .topic_detail(&topic)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            match result {
                Ok(detail) => {
                    let topic = topic.clone();
                    let _ = cx.update(|cx| {
                        if let Some(window) = cx.active_window() {
                            _ = window.update(cx, |_, window, cx| {
                                Self::show_status_dialog(&topic, &detail, window, cx)
                            });
                        }
                    });
                    _ = this.update(cx, |_, cx| cx.notify());
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "队列状态加载失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.load_failed", error = message).to_string(),
                        ),
                    );
                }
            }
        })
        .detach();
    }

    /// 渲染队列状态弹窗内容
    fn show_status_dialog(topic: &str, detail: &TopicDetail, window: &mut Window, cx: &mut App) {
        let title = t!("Middleware.status_title", topic = topic).to_string();
        let stats = detail.stats.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog.title(title.clone()).child(
                v_flex()
                    .min_w(px(460.0))
                    .gap_2()
                    .when(stats.is_empty(), |this| {
                        this.child(
                            div()
                                .py_2()
                                .text_sm()
                                .child(t!("Middleware.no_stats").to_string()),
                        )
                    })
                    .when(!stats.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .id("topic-stats-scroll")
                                .gap_1()
                                .max_h(px(360.0))
                                .overflow_y_scroll()
                                .children(
                                    stats
                                        .iter()
                                        .enumerate()
                                        .map(|(index, stat)| {
                                            h_flex()
                                                .id(SharedString::from(format!("stat-{index}")))
                                                .gap_2()
                                                .py_1()
                                                .child(
                                                    div()
                                                        .w(px(150.0))
                                                        .flex_shrink_0()
                                                        .text_sm()
                                                        .truncate()
                                                        .child(format!(
                                                            "{} #{}",
                                                            stat.broker, stat.queue_id
                                                        )),
                                                )
                                                .child(div().flex_1().min_w_0().text_xs().child(
                                                    format!(
                                                        "{}: {} · {}: {}",
                                                        t!("Middleware.stat_min"),
                                                        stat.min_offset,
                                                        t!("Middleware.stat_max"),
                                                        stat.max_offset
                                                    ),
                                                ))
                                                .child(div().text_xs().child(
                                                    stat.last_update.clone().unwrap_or_default(),
                                                ))
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                        )
                    }),
            )
        });
    }

    /// 打开新建/编辑 Topic 弹窗
    fn open_editor_dialog(
        &mut self,
        topic: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = topic.map(str::to_string);
        let existing =
            topic.and_then(|name| self.topics.iter().find(|item| item.name == name).cloned());
        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let topic_input = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .placeholder(t!("Middleware.field_topic").to_string());
                if let Some(name) = &editing {
                    state.set_value(name.clone(), window, cx);
                }
                state
            });
            let type_items: Vec<TopicTypeItem> = TOPIC_TYPE_FILTERS
                .iter()
                .map(|ty| TopicTypeItem((*ty).to_string()))
                .chain([TopicTypeItem("SUBSCRIPTION".to_string())])
                .collect();
            let selected_type = existing
                .as_ref()
                .and_then(|item| item.topic_type.clone())
                .unwrap_or_else(|| "NORMAL".to_string());
            let type_select = cx.new(|cx| {
                let mut state = SelectState::new(type_items, None, window, cx);
                state.set_selected_value(&selected_type, window, cx);
                state
            });
            let queues_input = cx.new(|cx| {
                let mut state = InputState::new(window, cx)
                    .placeholder(t!("Middleware.field_queues").to_string());
                let initial = existing
                    .as_ref()
                    .and_then(|item| item.queue_count)
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "8".to_string());
                state.set_value(initial, window, cx);
                state
            });
            let selected_perm = existing
                .as_ref()
                .and_then(|item| item.perm.clone())
                .unwrap_or_else(|| "6".to_string());
            let perm_select = cx.new(|cx| {
                let items = vec![
                    PermItem("6".into(), t!("Middleware.perm_6").to_string()),
                    PermItem("4".into(), t!("Middleware.perm_4").to_string()),
                    PermItem("2".into(), t!("Middleware.perm_2").to_string()),
                ];
                let mut state = SelectState::new(items, None, window, cx);
                state.set_selected_value(&selected_perm, window, cx);
                state
            });

            let title = if editing.is_some() {
                t!("Middleware.editor_edit_title").to_string()
            } else {
                t!("Middleware.editor_new_title").to_string()
            };
            let submit_label = if editing.is_some() {
                t!("Middleware.save").to_string()
            } else {
                t!("Middleware.create").to_string()
            };

            let submit_view = view.clone();
            let submit_editing = editing.clone();
            dialog
                .title(title)
                .child(
                    v_flex()
                        .min_w(px(420.0))
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
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
                                .items_center()
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .child(t!("Middleware.field_type").to_string()),
                                )
                                .child(div().flex_1().child(Select::new(&type_select).w_full())),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .child(t!("Middleware.field_queues").to_string()),
                                )
                                .child(div().flex_1().child(Input::new(&queues_input))),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(80.0))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .child(t!("Middleware.field_perm").to_string()),
                                )
                                .child(div().flex_1().child(Select::new(&perm_select).w_full())),
                        ),
                )
                .footer(
                    gpui_component::dialog::DialogFooter::new()
                        .child(
                            Button::new("topic-editor-cancel")
                                .label(t!("Middleware.cancel").to_string())
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child({
                            let topic_input = topic_input.clone();
                            let type_select = type_select.clone();
                            let queues_input = queues_input.clone();
                            let perm_select = perm_select.clone();
                            Button::new("topic-editor-submit")
                                .primary()
                                .label(submit_label)
                                .on_click(move |_, window, cx| {
                                    let topic =
                                        topic_input.read(cx).text().to_string().trim().to_string();
                                    let topic_type = type_select
                                        .read(cx)
                                        .selected_value()
                                        .cloned()
                                        .filter(|value| !value.is_empty());
                                    let queues_text = queues_input.read(cx).text().to_string();
                                    let queue_count = if queues_text.trim().is_empty() {
                                        Some(8)
                                    } else {
                                        queues_text.trim().parse::<u32>().ok()
                                    };
                                    let perm = perm_select
                                        .read(cx)
                                        .selected_value()
                                        .cloned()
                                        .filter(|value| !value.is_empty());
                                    window.close_dialog(cx);
                                    submit_view.update(cx, |page, cx| {
                                        page.submit_topic_editor(
                                            submit_editing.as_deref(),
                                            topic,
                                            topic_type,
                                            queue_count,
                                            perm,
                                            window,
                                            cx,
                                        );
                                    });
                                })
                        }),
                )
        });
    }

    /// 提交新建/编辑 Topic
    #[allow(clippy::too_many_arguments)]
    fn submit_topic_editor(
        &mut self,
        editing: Option<&str>,
        topic: String,
        topic_type: Option<String>,
        queue_count: Option<u32>,
        perm: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if topic.is_empty() {
            window.push_notification(
                Notification::error(t!("Middleware.topic_required").to_string()).autohide(true),
                cx,
            );
            return;
        }
        if queue_count.is_none() {
            window.push_notification(
                Notification::error(t!("Middleware.queues_invalid").to_string()).autohide(true),
                cx,
            );
            return;
        }
        let request = CreateTopicRequest {
            topic: topic.clone(),
            topic_type,
            queue_count,
            perm,
            attributes: Vec::new(),
        };
        let handle = self.handle.clone();
        let is_edit = editing.is_some();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let request = request.clone();
                async move {
                    if is_edit {
                        handle
                            .update_topic(request)
                            .await
                            .map_err(anyhow::Error::new)
                    } else {
                        handle
                            .create_topic(request)
                            .await
                            .map_err(anyhow::Error::new)
                    }
                }
            })
            .await;
            match result {
                Ok(()) => {
                    notify_async(
                        cx,
                        Notification::success(if is_edit {
                            t!("Middleware.update_ok").to_string()
                        } else {
                            t!("Middleware.create_ok").to_string()
                        }),
                    );
                    _ = this.update(cx, |view, cx| view.refresh(cx));
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "Topic 保存失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.save_failed", error = message).to_string(),
                        ),
                    );
                }
            }
        })
        .detach();
    }

    /// 删除确认弹窗
    fn confirm_delete_topic(&mut self, topic: &str, window: &mut Window, cx: &mut Context<Self>) {
        let topic = topic.to_string();
        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let confirm_view = view.clone();
            let confirm_topic = topic.clone();
            dialog
                .title(t!("Middleware.delete_title").to_string())
                .child(t!("Middleware.delete_warning", topic = confirm_topic).to_string())
                .footer(
                    gpui_component::dialog::DialogFooter::new()
                        .child(
                            Button::new("topic-delete-cancel")
                                .label(t!("Middleware.cancel").to_string())
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("topic-delete-confirm")
                                .danger()
                                .label(t!("Middleware.confirm").to_string())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    confirm_view.update(cx, |page, cx| {
                                        page.perform_delete_topic(confirm_topic.clone(), cx);
                                    });
                                }),
                        ),
                )
        });
    }

    /// 执行删除
    fn perform_delete_topic(&mut self, topic: String, cx: &mut Context<Self>) {
        let handle = self.handle.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let topic = topic.clone();
                async move {
                    handle
                        .delete_topic(&topic)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            match result {
                Ok(()) => {
                    notify_async(
                        cx,
                        Notification::success(t!("Middleware.delete_ok").to_string()),
                    );
                    _ = this.update(cx, |view, cx| view.refresh(cx));
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "Topic 删除失败");
                    notify_async(
                        cx,
                        Notification::error(
                            t!("Middleware.delete_failed", error = message).to_string(),
                        ),
                    );
                }
            }
        })
        .detach();
    }

    /// 打开发送消息弹窗(可预填 Tag/Key/Body,供重发复用)
    pub fn open_send_dialog(
        &mut self,
        topic: &str,
        prefill: Option<(&str, &str, &[u8])>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().clone();
        let topic = topic.to_string();
        // 转为 owned,避免引用逃逸进 'static 弹窗构建器
        let prefill =
            prefill.map(|(tag, key, body)| (tag.to_string(), key.to_string(), body.to_vec()));
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
                if let Some((tag, _, _)) = &prefill {
                    state.set_value((*tag).to_string(), window, cx);
                }
                state
            });
            let key_input = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder(t!("Middleware.field_key").to_string());
                if let Some((_, key, _)) = &prefill {
                    state.set_value((*key).to_string(), window, cx);
                }
                state
            });
            let body_input = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx)
                    .placeholder(t!("Middleware.body_placeholder").to_string())
                    .auto_grow(3, 8);
                if let Some((_, _, body)) = &prefill {
                    if let Ok(text) = std::str::from_utf8(body) {
                        state.set_value(text.to_string(), window, cx);
                    }
                }
                state
            });

            let submit_view = view.clone();
            dialog
                .title(t!("Middleware.send_title").to_string())
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
                            Button::new("send-cancel")
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
                            Button::new("send-submit")
                                .primary()
                                .label(t!("Middleware.send").to_string())
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
                                        page.submit_send_message(topic, tag, key, body, window, cx);
                                    });
                                })
                        }),
                )
        });
    }

    /// 提交发送消息
    fn submit_send_message(
        &mut self,
        topic: String,
        tag: String,
        key: String,
        body: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if topic.is_empty() {
            window.push_notification(
                Notification::error(t!("Middleware.topic_required").to_string()).autohide(true),
                cx,
            );
            return;
        }
        let request = SendMessageRequest {
            topic,
            tag: (!tag.is_empty()).then(|| tag.clone()),
            key: (!key.is_empty()).then(|| key.clone()),
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

    /// 渲染类型过滤标签(点选切换)
    fn render_type_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("topics-type-filters")
            .gap_1()
            .flex_wrap()
            .children(TOPIC_TYPE_FILTERS.iter().map(|ty| {
                let ty = *ty;
                let is_active = self.active_types.contains(ty);
                Button::new(SharedString::from(format!("type-filter-{ty}")))
                    .xsmall()
                    .when(is_active, |this| this.primary())
                    .when(!is_active, |this| this.ghost())
                    .label(ty.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.active_types.contains(ty) {
                            this.active_types.remove(ty);
                        } else {
                            this.active_types.insert(ty);
                        }
                        this.page = 1;
                        this.sync_table(cx);
                        cx.notify();
                    }))
            }))
            .when(!self.active_types.is_empty(), |this| {
                this.child(
                    Button::new("type-filter-clear")
                        .xsmall()
                        .ghost()
                        .label(t!("Middleware.clear_type_filter").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.active_types.clear();
                            this.page = 1;
                            this.sync_table(cx);
                            cx.notify();
                        })),
                )
            })
    }
}

impl Focusable for MiddlewareTopicsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for MiddlewareTopicsPage {}

impl TabContent for MiddlewareTopicsPage {
    fn content_key(&self) -> &'static str {
        "middleware-topics"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("Middleware.topics_tab").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Network).with_size(IconSize::Medium))
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

impl Render for MiddlewareTopicsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        let view = cx.entity().clone();
        let page = self.page;
        let page_size = self.page_size;
        let total = self.filtered_topics().len() as u64;
        let loading = self.load_state == LoadState::Loading;
        let can_write = self.capabilities.topic_write;

        v_flex()
            .id("middleware-topics-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .child(page_header(
                "middleware-topics",
                IconName::Network,
                t!("Middleware.topics_tab").to_string().into(),
                border_color,
                {
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |view, cx| view.refresh(cx));
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
                    .child(div().w(px(220.0)).child(Input::new(&self.filter_input)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Middleware.filter_type_label").to_string()),
                    )
                    .child(self.render_type_filters(cx))
                    .child(div().w(px(90.0)).child(Select::new(&self.page_size_select)))
                    .child(div().flex_1())
                    .when(loading, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Middleware.loading").to_string()),
                        )
                    })
                    .when(can_write, |this| {
                        this.child(
                            Button::new("topics-create")
                                .small()
                                .primary()
                                .label(t!("Middleware.editor_new_title").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_editor_dialog(None, window, cx);
                                })),
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
                "topics",
                PaginationSpec {
                    total,
                    page,
                    page_size,
                    has_more: false,
                },
                border_color,
                {
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |view, cx| {
                            if view.page > 1 {
                                view.page -= 1;
                                view.sync_table(cx);
                                cx.notify();
                            }
                        });
                    }
                },
                {
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |view, cx| {
                            if view.page < view.pages(view.filtered_topics().len()) {
                                view.page += 1;
                                view.sync_table(cx);
                                cx.notify();
                            }
                        });
                    }
                },
            ))
    }
}
