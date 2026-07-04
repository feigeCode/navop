use crate::database_users::user_listing_sql_for_config;
use crate::database_users_list::users_list;
use crate::database_users_toolbar::render_users_toolbar;
use db::{ExecOptions, GlobalDbState, SqlResult};
use gpui::{
    AnyElement, App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, scroll::ScrollableElement as _, v_flex,
};
use one_core::{
    storage::DbConnectionConfig,
    tab_container::{TabContent, TabContentEvent},
};

const USER_COLUMN_WIDTH_PX: f32 = 180.0;
const USER_ROW_HEIGHT_PX: f32 = 32.0;

#[derive(Clone)]
struct UserColumn {
    name: SharedString,
    width: gpui::Pixels,
}

pub struct DatabaseUsersTab {
    config: DbConnectionConfig,
    focus_handle: FocusHandle,
    columns: Vec<UserColumn>,
    rows: Vec<Vec<Option<String>>>,
    selected_row: Option<usize>,
    loading: bool,
    error: Option<String>,
}

impl DatabaseUsersTab {
    pub fn new(config: DbConnectionConfig, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            config,
            focus_handle: cx.focus_handle(),
            columns: default_columns(),
            rows: Vec::new(),
            selected_row: None,
            loading: true,
            error: None,
        };
        this.reload(cx);
        this
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.selected_row = None;

        let config = self.config.clone();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let sql = cx.update(|_| {
                let registry = if config.database_type.is_external() {
                    db::ipc::IpcDriverRegistry::load_default()
                } else {
                    db::ipc::IpcDriverRegistry::empty()
                };
                user_listing_sql_for_config(&config, &registry)
            });
            let Some(sql) = sql else {
                update_error(entity, cx, "当前数据库类型暂不支持用户列表查询。").await;
                return;
            };

            let result = execute_user_query(cx, &config, sql).await;
            entity
                .update(cx, |this, cx| {
                    this.apply_query_result(result);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    fn apply_query_result(&mut self, result: Result<SqlResult, String>) {
        self.loading = false;
        match result {
            Ok(SqlResult::Query(query)) => {
                self.columns = columns_from_names(query.columns);
                self.rows = query.rows;
                self.error = None;
            }
            Ok(SqlResult::Error(error)) => self.error = Some(error.message),
            Ok(SqlResult::Exec(_)) => self.error = Some("用户查询没有返回结果集。".to_string()),
            Err(error) => self.error = Some(error),
        }
    }

    fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        render_users_toolbar(self.config.name.clone(), window, cx)
    }

    fn render_header(&self, cx: &App) -> AnyElement {
        self.columns
            .iter()
            .fold(
                h_row(cx.theme().table_head)
                    .border_b_1()
                    .border_color(cx.theme().border),
                |row, column| {
                    row.child(
                        div()
                            .w(column.width)
                            .h_full()
                            .px_2()
                            .text_sm()
                            .text_color(cx.theme().table_head_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(column.name.clone()),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn render_row(&self, row_ix: usize, cx: &App) -> AnyElement {
        let values = self.rows.get(row_ix).cloned().unwrap_or_default();
        self.columns
            .iter()
            .enumerate()
            .fold(h_row(cx.theme().background), |row, (col_ix, column)| {
                let value = values
                    .get(col_ix)
                    .and_then(Clone::clone)
                    .unwrap_or_default();
                row.child(
                    div()
                        .w(column.width)
                        .h_full()
                        .px_2()
                        .text_sm()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(value),
                )
            })
            .when(self.selected_row == Some(row_ix), |row| {
                row.bg(cx.theme().selection)
            })
            .into_any_element()
    }

    pub(super) fn select_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.selected_row = Some(row_ix);
        cx.notify();
    }

    fn table_width(&self) -> gpui::Pixels {
        px(self.columns.len().max(1) as f32 * USER_COLUMN_WIDTH_PX)
    }
}

async fn execute_user_query(
    cx: &mut AsyncApp,
    config: &DbConnectionConfig,
    sql: String,
) -> Result<SqlResult, String> {
    let global_state = cx.update(|cx| cx.global::<GlobalDbState>().clone());
    global_state
        .execute_single(
            cx,
            config.id.clone(),
            sql,
            config.database.clone(),
            Some(ExecOptions::default()),
        )
        .await
        .map_err(|error| error.to_string())
}

async fn update_error(entity: WeakEntity<DatabaseUsersTab>, cx: &mut AsyncApp, message: &str) {
    let message = message.to_string();
    entity
        .update(cx, |this, cx| {
            this.loading = false;
            this.error = Some(message);
            cx.notify();
        })
        .ok();
}

fn default_columns() -> Vec<UserColumn> {
    columns_from_names(vec![
        "名称".to_string(),
        "用户".to_string(),
        "主机".to_string(),
        "插件".to_string(),
    ])
}

fn columns_from_names(names: Vec<String>) -> Vec<UserColumn> {
    names
        .into_iter()
        .map(|name| UserColumn {
            name: name.into(),
            width: px(USER_COLUMN_WIDTH_PX),
        })
        .collect()
}

fn h_row(bg: gpui::Hsla) -> gpui::Div {
    gpui_component::h_flex()
        .h(px(USER_ROW_HEIGHT_PX))
        .items_center()
        .bg(bg)
}

impl Render for DatabaseUsersTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.rows.len();
        let table_width = self.table_width();
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(self.render_toolbar(window, cx))
            .child(
                div().flex_1().overflow_x_scrollbar().child(
                    v_flex()
                        .h_full()
                        .w(table_width)
                        .child(self.render_header(cx))
                        .when(self.loading, |this| {
                            this.child(div().p_3().child("正在加载用户..."))
                        })
                        .when_some(self.error.clone(), |this, error| {
                            this.child(div().p_3().text_color(cx.theme().danger).child(error))
                        })
                        .when(
                            !self.loading && self.error.is_none() && row_count == 0,
                            |this| {
                                this.child(
                                    div()
                                        .p_3()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("暂无用户"),
                                )
                            },
                        )
                        .child(users_list(row_count, cx)),
                ),
            )
    }
}

impl Focusable for DatabaseUsersTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for DatabaseUsersTab {}

impl TabContent for DatabaseUsersTab {
    fn content_key(&self) -> &'static str {
        "DatabaseUsers"
    }

    fn title(&self, _cx: &App) -> SharedString {
        "用户".into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::User.color().with_size(Size::Medium))
    }
}
