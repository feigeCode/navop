//! 中间件订阅组管理页(RocketMQ Console Consumer 页风格)。
//!
//! - 表格:订阅组 | 客户端数量 | 消费类型 | 消息模型 | TPS | 堆积 | 更新时间 | 操作
//! - 操作:客户端详情弹窗(clients 能力位) / 消费详情弹窗 / 刷新
//! - 分页:本地分页(10/20/50 可选)

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
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    table::{Column, ColumnFixed, DataTable, TableDelegate, TableState},
    v_flex,
};
use middleware_runtime::{GroupConsumeDetail, MiddlewareClientInfo, MiddlewareGroupInfo};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use tracing::warn;

use crate::MiddlewareAdminHandle;
use crate::common::{
    LoadState, PageSizeItem, PaginationSpec, notify_async, page_header, page_size_select,
    pagination_bar, render_table_empty, text_cell,
};

/// 订阅组表格列数
const COLUMN_COUNT: usize = 8;
/// 操作列下标
const ACTIONS_COL: usize = COLUMN_COUNT - 1;

/// 订阅组页表格 delegate
struct GroupsTableDelegate {
    /// 列定义
    columns: Vec<Column>,
    /// 当前页可见行
    rows: Vec<MiddlewareGroupInfo>,
    /// 所属页面(操作按钮回调)
    page: Entity<MiddlewareGroupsPage>,
    /// 是否可查客户端(clients 能力位)
    can_clients: bool,
}

impl GroupsTableDelegate {
    fn columns() -> Vec<Column> {
        vec![
            Column::new("group", t!("Middleware.col_group").to_string())
                .width(px(220.0))
                .min_width(px(140.0))
                .max_width(px(380.0))
                .fixed(ColumnFixed::Left),
            Column::new("count", t!("Middleware.col_count").to_string())
                .width(px(80.0))
                .min_width(px(60.0)),
            Column::new("type", t!("Middleware.col_type").to_string())
                .width(px(100.0))
                .min_width(px(80.0)),
            Column::new("model", t!("Middleware.col_model").to_string())
                .width(px(120.0))
                .min_width(px(100.0)),
            Column::new("tps", t!("Middleware.col_tps").to_string())
                .width(px(90.0))
                .min_width(px(70.0)),
            Column::new("diff", t!("Middleware.col_diff").to_string())
                .width(px(110.0))
                .min_width(px(90.0)),
            Column::new("updated", t!("Middleware.col_updated").to_string())
                .width(px(170.0))
                .min_width(px(140.0)),
            Column::new("actions", t!("Middleware.col_actions").to_string())
                .width(px(200.0))
                .min_width(px(180.0)),
        ]
    }
}

impl TableDelegate for GroupsTableDelegate {
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
        let Some(group) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        if col_ix == ACTIONS_COL {
            let page = self.page.clone();
            let group_name = group.group.clone();
            let clients_view = page.clone();
            let clients_group = group_name.clone();
            let consume_view = page.clone();
            let consume_group = group_name.clone();
            return h_flex()
                .id(SharedString::from(format!("groups-row-{row_ix}")))
                .gap_1()
                .items_center()
                .when(self.can_clients, |this| {
                    this.child(
                        Button::new(SharedString::from(format!("groups-clients-{row_ix}")))
                            .xsmall()
                            .ghost()
                            .label(t!("Middleware.action_clients").to_string())
                            .on_click(move |_, window, cx| {
                                clients_view.update(cx, |page, cx| {
                                    page.open_clients_dialog(&clients_group, window, cx);
                                });
                            }),
                    )
                })
                .child(
                    Button::new(SharedString::from(format!("groups-consume-{row_ix}")))
                        .xsmall()
                        .ghost()
                        .label(t!("Middleware.action_consume").to_string())
                        .on_click(move |_, window, cx| {
                            consume_view.update(cx, |page, cx| {
                                page.open_consume_dialog(&consume_group, window, cx);
                            });
                        }),
                )
                .into_any_element();
        }
        let text = match col_ix {
            0 => group.group.clone(),
            1 => group
                .client_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".into()),
            2 => group.consume_type.clone().unwrap_or_else(|| "-".into()),
            3 => group.message_model.clone().unwrap_or_else(|| "-".into()),
            4 => group
                .tps
                .map(|tps| format!("{tps:.1}"))
                .unwrap_or_else(|| "-".into()),
            5 => group
                .total_diff
                .map(|diff| diff.to_string())
                .unwrap_or_else(|| "-".into()),
            6 => group.update_time.clone().unwrap_or_else(|| "-".into()),
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
            t!("Middleware.groups_empty_detail").to_string().into(),
            cx,
        )
    }
}

/// 中间件订阅组管理页
pub struct MiddlewareGroupsPage {
    /// 管理接口句柄
    handle: MiddlewareAdminHandle,
    /// 全量订阅组列表
    groups: Vec<MiddlewareGroupInfo>,
    /// 加载状态
    load_state: LoadState,
    /// 刷新代号(递增使旧在途刷新的回写失效)
    refresh_generation: u64,
    /// 每页条数下拉
    page_size_select: Entity<SelectState<Vec<PageSizeItem>>>,
    /// 表格状态
    table: Entity<TableState<GroupsTableDelegate>>,
    /// 当前页(1 起)
    page: usize,
    /// 每页条数
    page_size: usize,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MiddlewareGroupsPage {
    /// 创建订阅组页并立即加载
    pub fn new(handle: MiddlewareAdminHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let capabilities = handle.capabilities();
        let page_entity = cx.entity();
        let table = cx.new(|cx| {
            TableState::new(
                GroupsTableDelegate {
                    columns: GroupsTableDelegate::columns(),
                    rows: Vec::new(),
                    page: page_entity.clone(),
                    can_clients: capabilities.clients,
                },
                window,
                cx,
            )
        });

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
            groups: Vec::new(),
            load_state: LoadState::Idle,
            refresh_generation: 0,
            page_size_select,
            table,
            page: 1,
            page_size: 10,
            focus_handle: cx.focus_handle(),
        };
        this.refresh(cx);
        this
    }

    /// 总页数
    fn pages(&self, total: usize) -> usize {
        total.div_ceil(self.page_size).max(1)
    }

    /// 把分页后的行写入表格
    fn sync_table(&mut self, cx: &mut Context<Self>) {
        let total = self.groups.len();
        self.page = self.page.min(self.pages(total));
        let start = self.page.saturating_sub(1) * self.page_size;
        let rows: Vec<MiddlewareGroupInfo> = self
            .groups
            .iter()
            .skip(start)
            .take(self.page_size)
            .cloned()
            .collect();
        let table = self.table.clone();
        table.update(cx, |state, cx| {
            state.delegate_mut().rows = rows;
            cx.notify();
        });
    }

    /// 刷新订阅组列表(回写带 generation 防护,旧请求晚到不覆盖新数据)
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_state = LoadState::Loading;
        // 递增代号使旧的在途刷新回写失效
        let generation = self.refresh_generation.wrapping_add(1);
        self.refresh_generation = generation;
        let handle = self.handle.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, async move {
                handle.list_groups().await.map_err(anyhow::Error::new)
            })
            .await;
            // generation 不匹配说明期间又发起了新刷新,过期数据直接丢弃
            let stale = |view: &Self| view.refresh_generation != generation;
            match result {
                Ok(groups) => {
                    _ = this.update(cx, |view, cx| {
                        if stale(view) {
                            return;
                        }
                        view.groups = groups;
                        view.load_state = LoadState::Loaded;
                        view.page = 1;
                        view.sync_table(cx);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "订阅组列表加载失败");
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

    /// 打开客户端详情弹窗(先拉取 group_clients)
    fn open_clients_dialog(&mut self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle.clone();
        let group = group.to_string();
        let _ = window;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let group = group.clone();
                async move {
                    handle
                        .group_clients(&group)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            match result {
                Ok(clients) => {
                    let group = group.clone();
                    let _ = cx.update(|cx| {
                        if let Some(window) = cx.active_window() {
                            _ = window.update(cx, |_, window, cx| {
                                Self::show_clients_dialog(&group, &clients, window, cx)
                            });
                        }
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "订阅组客户端加载失败");
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

    /// 渲染客户端详情弹窗
    fn show_clients_dialog(
        group: &str,
        clients: &[MiddlewareClientInfo],
        window: &mut Window,
        cx: &mut App,
    ) {
        let title = t!("Middleware.clients_title", group = group).to_string();
        let clients = clients.to_vec();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog.title(title.clone()).child(
                v_flex()
                    .min_w(px(520.0))
                    .gap_2()
                    .when(clients.is_empty(), |this| {
                        this.child(
                            div()
                                .py_2()
                                .text_sm()
                                .child(t!("Middleware.no_clients").to_string()),
                        )
                    })
                    .when(!clients.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .id("clients-scroll")
                                .gap_2()
                                .max_h(px(360.0))
                                .overflow_y_scroll()
                                .children(
                                    clients
                                        .iter()
                                        .enumerate()
                                        .map(|(index, client)| {
                                            v_flex()
                                                .id(SharedString::from(format!("client-{index}")))
                                                .gap_1()
                                                .p_2()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .truncate()
                                                        .child(client.client_id.clone()),
                                                )
                                                .child(client_row(
                                                    t!("Middleware.col_client_addr").to_string(),
                                                    client
                                                        .client_addr
                                                        .clone()
                                                        .unwrap_or_else(|| "-".into()),
                                                ))
                                                .child(client_row(
                                                    t!("Middleware.col_language").to_string(),
                                                    client
                                                        .language
                                                        .clone()
                                                        .unwrap_or_else(|| "-".into()),
                                                ))
                                                .child(client_row(
                                                    t!("Middleware.col_version").to_string(),
                                                    client
                                                        .version
                                                        .clone()
                                                        .unwrap_or_else(|| "-".into()),
                                                ))
                                                .child(client_row(
                                                    t!("Middleware.col_subscriptions").to_string(),
                                                    if client.subscriptions.is_empty() {
                                                        "-".to_string()
                                                    } else {
                                                        client.subscriptions.join(", ")
                                                    },
                                                ))
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                        )
                    }),
            )
        });
    }

    /// 打开消费详情弹窗(先拉取 group_detail)
    fn open_consume_dialog(&mut self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle.clone();
        let group = group.to_string();
        let _ = window;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = Tokio::spawn_result(cx, {
                let handle = handle.clone();
                let group = group.clone();
                async move {
                    handle
                        .group_detail(&group)
                        .await
                        .map_err(anyhow::Error::new)
                }
            })
            .await;
            match result {
                Ok(detail) => {
                    let group = group.clone();
                    let _ = cx.update(|cx| {
                        if let Some(window) = cx.active_window() {
                            _ = window.update(cx, |_, window, cx| {
                                Self::show_consume_dialog(&group, &detail, window, cx)
                            });
                        }
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "订阅组消费详情加载失败");
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

    /// 渲染消费详情弹窗(队列位点/堆积列表)
    fn show_consume_dialog(
        group: &str,
        detail: &GroupConsumeDetail,
        window: &mut Window,
        cx: &mut App,
    ) {
        let title = t!("Middleware.consume_title", group = group).to_string();
        let queues = detail.queues.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog.title(title.clone()).child(
                v_flex()
                    .min_w(px(560.0))
                    .gap_2()
                    .when(queues.is_empty(), |this| {
                        this.child(
                            div()
                                .py_2()
                                .text_sm()
                                .child(t!("Middleware.no_consume").to_string()),
                        )
                    })
                    .when(!queues.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .id("consume-scroll")
                                .gap_1()
                                .max_h(px(360.0))
                                .overflow_y_scroll()
                                .children(
                                    queues
                                        .iter()
                                        .enumerate()
                                        .map(|(index, queue)| {
                                            h_flex()
                                                .id(SharedString::from(format!("cq-{index}")))
                                                .gap_2()
                                                .py_1()
                                                .child(
                                                    div()
                                                        .w(px(170.0))
                                                        .flex_shrink_0()
                                                        .text_sm()
                                                        .truncate()
                                                        .child(format!(
                                                            "{} · {} #{}",
                                                            queue.topic,
                                                            queue.broker,
                                                            queue.queue_id
                                                        )),
                                                )
                                                .child(div().flex_1().min_w_0().text_xs().child(
                                                    format!(
                                                        "{}: {} · {}: {}",
                                                        t!("Middleware.col_boffset"),
                                                        queue.broker_offset,
                                                        t!("Middleware.col_coffset"),
                                                        queue.consumer_offset
                                                    ),
                                                ))
                                                .child(div().text_xs().child(format!(
                                                    "{}: {}",
                                                    t!("Middleware.col_cdiff"),
                                                    queue.diff
                                                )))
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                        )
                    }),
            )
        });
    }
}

/// 弹窗内的"标签: 值"行
fn client_row(label: String, value: String) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(div().text_xs().child(format!("{label}: {value}")))
}

impl Focusable for MiddlewareGroupsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for MiddlewareGroupsPage {}

impl TabContent for MiddlewareGroupsPage {
    fn content_key(&self) -> &'static str {
        "middleware-groups"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("Middleware.groups_tab").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Bell).with_size(IconSize::Medium))
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

impl Render for MiddlewareGroupsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        let view = cx.entity().clone();
        let page = self.page;
        let page_size = self.page_size;
        let total = self.groups.len() as u64;
        let loading = self.load_state == LoadState::Loading;

        v_flex()
            .id("middleware-groups-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .child(page_header(
                "middleware-groups",
                IconName::Bell,
                t!("Middleware.groups_tab").to_string().into(),
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
                    .border_b_1()
                    .border_color(border_color)
                    .child(div().flex_1())
                    .child(div().w(px(90.0)).child(Select::new(&self.page_size_select)))
                    .when(loading, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Middleware.loading").to_string()),
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
                "groups",
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
                            if view.page < view.pages(view.groups.len()) {
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
