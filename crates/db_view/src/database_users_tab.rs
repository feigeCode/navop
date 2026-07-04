use crate::database_users_list::users_list;
use crate::database_users_toolbar::{DatabaseUsersToolbarAction, render_users_toolbar};
use crate::database_view_plugin::create_user_editor_view_for;
use db::plugin::DatabaseUserOperationRequest;
use db::plugin_manifest::DatabaseFormKind;
use db::{ExecOptions, GlobalDbState, SqlResult};
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt, dialog::DialogButtonProps,
    notification::Notification, scroll::ScrollableElement as _, v_flex,
};
use one_core::{
    storage::DbConnectionConfig,
    tab_container::{TabContent, TabContentEvent},
};
use std::collections::HashMap;

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
            let sql = cx.update(|cx| {
                let global_state = cx.global::<GlobalDbState>().clone();
                global_state
                    .get_plugin(&config.database_type)
                    .map_err(|error| error.to_string())
                    .map(|plugin| plugin.build_list_users_sql(config.database.as_deref()))
            });
            let sql = match sql {
                Ok(Some(sql)) => sql,
                Ok(None) => {
                    update_error(entity, cx, "当前数据库类型暂不支持用户列表查询。").await;
                    return;
                }
                Err(error) => {
                    update_error(entity, cx, &error).await;
                    return;
                }
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
        let capabilities = cx
            .global::<GlobalDbState>()
            .get_plugin(&self.config.database_type)
            .map(|plugin| plugin.capabilities())
            .unwrap_or_default();
        render_users_toolbar(self.config.name.clone(), capabilities, window, cx)
    }

    pub(super) fn handle_toolbar_action(
        &mut self,
        action: DatabaseUsersToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DatabaseUsersToolbarAction::Refresh => self.reload(cx),
            DatabaseUsersToolbarAction::Add => {
                self.open_user_editor(DatabaseFormKind::CreateUser, None, window, cx)
            }
            DatabaseUsersToolbarAction::Edit => self.open_selected_user_editor(
                DatabaseFormKind::EditUser,
                "请先选择要编辑的用户。",
                window,
                cx,
            ),
            DatabaseUsersToolbarAction::Delete => self.open_selected_user_editor(
                DatabaseFormKind::DeleteUser,
                "请先选择要删除的用户。",
                window,
                cx,
            ),
            DatabaseUsersToolbarAction::Privileges => self.open_selected_user_editor(
                DatabaseFormKind::UserPrivileges,
                "请先选择要授权的用户。",
                window,
                cx,
            ),
        }
    }

    fn open_selected_user_editor(
        &self,
        operation: DatabaseFormKind,
        empty_message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(initial) = self.selected_user_request() else {
            window.push_notification(Notification::warning(empty_message).autohide(true), cx);
            return;
        };
        self.open_user_editor(operation, Some(initial), window, cx);
    }

    fn open_user_editor(
        &self,
        operation: DatabaseFormKind,
        initial: Option<DatabaseUserOperationRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = create_user_editor_view_for(
            self.config.database_type.clone(),
            operation,
            initial,
            window,
            cx,
        ) else {
            window.push_notification(Notification::info("当前数据库类型暂不支持该用户操作。"), cx);
            return;
        };

        let config = self.config.clone();
        let tab = cx.entity();
        let editor_for_ok = editor.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor = editor.clone();
            let editor_ok = editor_for_ok.clone();
            let config = config.clone();
            let tab = tab.clone();
            dialog
                .title(user_operation_title(operation))
                .overlay(false)
                .width(px(700.0))
                .child(editor.clone())
                .button_props(DialogButtonProps::default().ok_text("执行".to_string()))
                .footer(|ok, cancel, window, cx| vec![cancel(window, cx), ok(window, cx)])
                .on_ok(move |_, _window, cx| {
                    let sql = editor_ok.read(cx).get_sql(cx);
                    if sql.trim().is_empty() || sql.trim_start().starts_with("--") {
                        editor_ok.update(cx, |editor, cx| {
                            editor.set_save_error("没有可执行的用户操作 SQL。".to_string(), cx);
                        });
                        return false;
                    }
                    execute_user_operation(sql, config.clone(), tab.clone(), editor_ok.clone(), cx);
                    false
                })
        });
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

    fn selected_user_request(&self) -> Option<DatabaseUserOperationRequest> {
        let row = self.rows.get(self.selected_row?)?;
        let user_name = self.column_value(row, &["user", "rolname", "name", "username"])?;
        let host = self.column_value(row, &["host"]);
        Some(DatabaseUserOperationRequest {
            user_name,
            host,
            database: self.config.database.clone(),
            field_values: HashMap::new(),
        })
    }

    fn column_value(&self, row: &[Option<String>], names: &[&str]) -> Option<String> {
        self.columns
            .iter()
            .position(|column| {
                let current = column.name.as_ref().to_ascii_lowercase();
                names.iter().any(|name| current == *name)
            })
            .and_then(|index| row.get(index).and_then(Clone::clone))
            .filter(|value| !value.trim().is_empty())
    }

    fn table_width(&self) -> gpui::Pixels {
        px(self.columns.len().max(1) as f32 * USER_COLUMN_WIDTH_PX)
    }
}

fn user_operation_title(operation: DatabaseFormKind) -> &'static str {
    match operation {
        DatabaseFormKind::CreateUser => "新增用户",
        DatabaseFormKind::EditUser => "编辑用户",
        DatabaseFormKind::DeleteUser => "删除用户",
        DatabaseFormKind::UserPrivileges => "用户权限",
        _ => "用户操作",
    }
}

fn execute_user_operation(
    sql: String,
    config: DbConnectionConfig,
    tab: gpui::Entity<DatabaseUsersTab>,
    editor: gpui::Entity<crate::common::UserEditorView>,
    cx: &mut App,
) {
    let global_state = cx.global::<GlobalDbState>().clone();
    let window_id = cx.active_window();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = global_state
            .execute_single(
                cx,
                config.id.clone(),
                sql,
                config.database.clone(),
                Some(ExecOptions::default()),
            )
            .await;
        apply_user_operation_result(
            result.map_err(|error| error.to_string()),
            tab,
            editor,
            window_id,
            cx,
        )
        .await;
    })
    .detach();
}

async fn apply_user_operation_result(
    result: Result<SqlResult, String>,
    tab: gpui::Entity<DatabaseUsersTab>,
    editor: gpui::Entity<crate::common::UserEditorView>,
    window_id: Option<gpui::AnyWindowHandle>,
    cx: &mut AsyncApp,
) {
    match result {
        Ok(SqlResult::Error(error)) => show_user_operation_error(editor, error.message, cx),
        Err(error) => show_user_operation_error(editor, error, cx),
        Ok(SqlResult::Exec(_)) | Ok(SqlResult::Query(_)) => {
            let Some(window_id) = window_id else { return };
            let _ = cx.update_window(window_id, |_entity, window, cx| {
                window.close_dialog(cx);
                window.push_notification(
                    Notification::success("用户操作已执行。").autohide(true),
                    cx,
                );
                tab.update(cx, |this, cx| this.reload(cx));
            });
        }
    }
}

fn show_user_operation_error(
    editor: gpui::Entity<crate::common::UserEditorView>,
    error: String,
    cx: &mut AsyncApp,
) {
    let _ = editor.update(cx, |editor, cx| {
        editor.set_save_error(format!("用户操作失败: {error}"), cx);
    });
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
