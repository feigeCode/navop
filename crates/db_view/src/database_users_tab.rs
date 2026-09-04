use crate::database_table_columns::{
    render_table_column_resize_handle, resize_table_column, table_columns_width,
    ui_columns_from_object_columns,
};
use crate::database_users_list::users_list;
use crate::database_users_toolbar::{DatabaseUsersToolbarAction, render_users_toolbar};
use crate::database_view_plugin::create_user_editor_view_for;
use db::plugin::DatabaseUserOperationRequest;
use db::plugin_manifest::DatabaseFormKind;
use db::{ExecOptions, GlobalDbState, SqlResult, types::ObjectView};
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt, dialog::DialogButtonProps,
    notification::Notification, scroll::ScrollableElement as _, table::Column, v_flex,
};
use one_core::{
    storage::DbConnectionConfig,
    tab_container::{TabContent, TabContentEvent},
};
use rust_i18n::t;
use std::collections::HashMap;

const USER_COLUMN_WIDTH_PX: f32 = 180.0;
const USER_ROW_HEIGHT_PX: f32 = 32.0;

pub struct DatabaseUsersTab {
    config: DbConnectionConfig,
    focus_handle: FocusHandle,
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
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
            let result = load_user_view(cx, &config).await;
            entity
                .update(cx, |this, cx| {
                    this.apply_view_result(result);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    fn apply_view_result(&mut self, result: Result<ObjectView, String>) {
        self.loading = false;
        match result {
            Ok(view) => {
                self.columns = ui_columns_from_object_columns(&view.columns);
                self.rows = view.rows;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let capabilities = cx
            .global::<GlobalDbState>()
            .get_plugin(&self.config.database_type)
            .map(|plugin| plugin.ui_manifest().capabilities)
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
                t!("DatabaseUsers.select_edit_user").to_string(),
                window,
                cx,
            ),
            DatabaseUsersToolbarAction::Delete => self.open_selected_user_editor(
                DatabaseFormKind::DeleteUser,
                t!("DatabaseUsers.select_delete_user").to_string(),
                window,
                cx,
            ),
            DatabaseUsersToolbarAction::Privileges => self.open_selected_user_editor(
                DatabaseFormKind::UserPrivileges,
                t!("DatabaseUsers.select_privilege_user").to_string(),
                window,
                cx,
            ),
        }
    }

    fn open_selected_user_editor(
        &self,
        operation: DatabaseFormKind,
        empty_message: String,
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
            window.push_notification(
                Notification::info(t!("DatabaseUsers.unsupported_operation").to_string()),
                cx,
            );
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
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("DatabaseUsers.execute").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let sql = editor_ok.read(cx).get_sql(cx);
                    if sql.trim().is_empty() || sql.trim_start().starts_with("--") {
                        editor_ok.update(cx, |editor, cx| {
                            editor.set_save_error(
                                t!("DatabaseUsers.empty_operation_sql").to_string(),
                                cx,
                            );
                        });
                        return false;
                    }
                    execute_user_operation(sql, config.clone(), tab.clone(), editor_ok.clone(), cx);
                    false
                })
        });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        self.columns
            .iter()
            .enumerate()
            .fold(
                h_row(cx.theme().table_head)
                    .border_b_1()
                    .border_color(cx.theme().border),
                |row, (col_ix, column)| {
                    row.child(
                        div()
                            .relative()
                            .w(column.width)
                            .h_full()
                            .px_2()
                            .text_sm()
                            .text_color(cx.theme().table_head_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(column.name.clone())
                            .child(render_table_column_resize_handle(
                                "database-user-column-resize",
                                "database-user-column-resize",
                                col_ix,
                                column,
                                cx,
                                |this: &Self, col_ix| {
                                    this.columns.get(col_ix).map(|column| column.width)
                                },
                                |this: &mut Self, col_ix, width| {
                                    resize_table_column(&mut this.columns, col_ix, width);
                                },
                            )),
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
                let value = values.get(col_ix).cloned().unwrap_or_default();
                row.child(
                    div()
                        .w(column.width)
                        .h_full()
                        .px_2()
                        .text_sm()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
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

    fn column_value(&self, row: &[String], names: &[&str]) -> Option<String> {
        self.columns
            .iter()
            .position(|column| {
                let key = column.key.as_ref().to_ascii_lowercase();
                let label = column.name.as_ref().to_ascii_lowercase();
                names.iter().any(|name| key == *name || label == *name)
            })
            .and_then(|index| row.get(index).cloned())
            .filter(|value| !value.trim().is_empty())
    }

    fn table_width(&self) -> gpui::Pixels {
        table_columns_width(&self.columns)
    }
}

fn user_operation_title(operation: DatabaseFormKind) -> String {
    match operation {
        DatabaseFormKind::CreateUser => t!("DatabaseUsers.create_title").to_string(),
        DatabaseFormKind::EditUser => t!("DatabaseUsers.edit_title").to_string(),
        DatabaseFormKind::DeleteUser => t!("DatabaseUsers.delete_title").to_string(),
        DatabaseFormKind::UserPrivileges => t!("DatabaseUsers.privileges_title").to_string(),
        _ => t!("DatabaseUsers.operation_title").to_string(),
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
                    Notification::success(t!("DatabaseUsers.operation_success").to_string())
                        .autohide(true),
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
        editor.set_save_error(
            t!("DatabaseUsers.operation_failed", error = error).to_string(),
            cx,
        );
    });
}

async fn load_user_view(
    cx: &mut AsyncApp,
    config: &DbConnectionConfig,
) -> Result<ObjectView, String> {
    let global_state = cx.update(|cx| cx.global::<GlobalDbState>().clone());
    global_state
        .list_users_view(cx, config.id.clone(), config.database.clone())
        .await
        .map_err(|error| error.to_string())
}

fn default_columns() -> Vec<Column> {
    columns_from_pairs(vec![
        ("user", translate_db_label("DatabaseUser.columns.user")),
        ("host", translate_db_label("DatabaseUser.columns.host")),
        (
            "authentication_plugin",
            translate_db_label("DatabaseUser.columns.authentication_plugin"),
        ),
    ])
}

fn columns_from_pairs(columns: Vec<(&'static str, String)>) -> Vec<Column> {
    columns
        .into_iter()
        .map(|(key, name)| Column::new(key, name).width(px(USER_COLUMN_WIDTH_PX)))
        .collect()
}

fn translate_db_label(key: &str) -> String {
    db::translate_or_raw_for_locale(rust_i18n::locale().as_ref(), key)
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
                            this.child(div().p_3().child(t!("DatabaseUsers.loading").to_string()))
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
                                        .child(t!("DatabaseUsers.empty").to_string()),
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
        t!("DatabaseUsers.title").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::User.color().with_size(Size::Medium))
    }
}
