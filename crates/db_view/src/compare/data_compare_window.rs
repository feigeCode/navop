use std::collections::HashSet;
use std::sync::Arc;

use db::{DbNode, GlobalDbState};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Window, div,
};
use gpui_component::{
    Disableable, StyledExt,
    button::{Button, ButtonVariants as _},
    input::InputState,
    select::SelectState,
    v_flex,
};

use crate::compare::sync_statement_picker::{
    default_selected_statement_ids, selected_sync_sql_text_for_ids,
};
use crate::compare::target_picker::{StringSelect, selected_string, string_select_state};
use crate::compare::window_params::data_compare_params;
use crate::compare::window_ui::{
    ConnectionSelectItem, close_button, connection_select_state, selected_connection_id,
    start_sync_sql_execution,
};
use crate::compare::{
    CompareTargetScope, DataCompareParams, execute_data_compare, generate_data_sync_plan,
};
use db::compare::{DataCompareResult, SyncPlan};

pub struct DataCompareWindow {
    pub(super) source_node: DbNode,
    pub(super) target_connection_id: Entity<InputState>,
    pub(super) target_connection_select: Entity<SelectState<Vec<ConnectionSelectItem>>>,
    pub(super) target_database: Entity<InputState>,
    pub(super) target_database_select: StringSelect,
    pub(super) target_schema: Entity<InputState>,
    pub(super) target_schema_select: StringSelect,
    pub(super) target_table: Entity<InputState>,
    pub(super) target_table_select: StringSelect,
    pub(super) key_columns: Entity<InputState>,
    pub(super) result: Entity<Option<DataCompareResult>>,
    pub(super) sync_plan: Entity<Option<SyncPlan>>,
    pub(super) selected_statement_ids: Entity<HashSet<String>>,
    compare_target: Entity<Option<CompareTargetScope>>,
    pub(super) status: Entity<String>,
    is_running: Entity<bool>,
    is_executing: Entity<bool>,
    focus_handle: FocusHandle,
}

impl DataCompareWindow {
    pub fn new(source_node: DbNode, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let target_connection_id = cx
            .new(|cx| InputState::new(window, cx).default_value(source_node.connection_id.clone()));
        let target_connection_select =
            connection_select_state(&source_node.connection_id, window, cx);
        let target_database = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(source_node.get_database_name().unwrap_or_default())
        });
        let target_database_select = string_select_state(
            source_node.get_database_name().unwrap_or_default(),
            window,
            cx,
        );
        let target_schema = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(source_node.get_schema_name().unwrap_or_default())
        });
        let target_schema_select = string_select_state(
            source_node.get_schema_name().unwrap_or_default(),
            window,
            cx,
        );
        let target_table =
            cx.new(|cx| InputState::new(window, cx).default_value(source_node.name.clone()));
        let target_table_select = string_select_state(source_node.name.clone(), window, cx);
        let key_columns = cx.new(|cx| InputState::new(window, cx).placeholder("id, tenant_id"));

        cx.new(|cx| Self {
            source_node,
            target_connection_id,
            target_connection_select,
            target_database,
            target_database_select,
            target_schema,
            target_schema_select,
            target_table,
            target_table_select,
            key_columns,
            result: cx.new(|_| None),
            sync_plan: cx.new(|_| None),
            selected_statement_ids: cx.new(|_| HashSet::new()),
            compare_target: cx.new(|_| None),
            status: cx.new(|_| "Ready".to_string()),
            is_running: cx.new(|_| false),
            is_executing: cx.new(|_| false),
            focus_handle: cx.focus_handle(),
        })
    }

    pub fn popup_title_for(source_node: &DbNode) -> String {
        format!("数据比较 - {}", source_node.name)
    }

    fn start_compare(&mut self, cx: &mut Context<Self>) {
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(message) => {
                self.set_status(message, cx);
                return;
            }
        };
        let compare_target = CompareTargetScope::from_data_params(&params);
        let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
        self.is_running.update(cx, |running, cx| {
            *running = true;
            cx.notify();
        });
        self.set_status("Comparing data...", cx);

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = execute_data_compare(params, db_state, cx).await;
            let _ = this.update(cx, |view, cx| {
                view.is_running.update(cx, |running, cx| {
                    *running = false;
                    cx.notify();
                });
                match result {
                    Ok(result) => {
                        let plan = generate_data_sync_plan(&result);
                        let selected_ids = default_selected_statement_ids(&plan);
                        view.result.update(cx, |slot, cx| {
                            *slot = Some(result);
                            cx.notify();
                        });
                        view.sync_plan.update(cx, |slot, cx| {
                            *slot = Some(plan);
                            cx.notify();
                        });
                        view.selected_statement_ids.update(cx, |slot, cx| {
                            *slot = selected_ids;
                            cx.notify();
                        });
                        view.compare_target.update(cx, |slot, cx| {
                            *slot = Some(compare_target);
                            cx.notify();
                        });
                        view.set_status("Data compare finished", cx);
                    }
                    Err(error) => view.set_status(format!("Compare failed: {error}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn build_params(&self, cx: &mut Context<Self>) -> Result<DataCompareParams, &'static str> {
        let target_connection_id = selected_connection_id(
            &self.target_connection_select,
            &self.target_connection_id,
            cx,
        );
        data_compare_params(
            &self.source_node,
            target_connection_id,
            selected_string(&self.target_database_select, &self.target_database, cx),
            selected_string(&self.target_schema_select, &self.target_schema, cx),
            selected_string(&self.target_table_select, &self.target_table, cx),
            self.key_columns.read(cx).text().to_string(),
        )
    }

    fn start_execute_sync_sql(&mut self, cx: &mut Context<Self>) {
        start_sync_sql_execution(
            self.compare_target.read(cx).clone(),
            self.selected_sync_sql(cx),
            self.status.clone(),
            self.is_executing.clone(),
            cx,
        );
    }

    fn selected_sync_sql(&self, cx: &mut Context<Self>) -> String {
        let selected_ids = self.selected_statement_ids.read(cx);
        self.sync_plan
            .read(cx)
            .as_ref()
            .map_or_else(String::new, |plan| {
                selected_sync_sql_text_for_ids(plan, &selected_ids)
            })
    }

    fn has_selected_sync_sql(&self, cx: &mut Context<Self>) -> bool {
        !self.selected_sync_sql(cx).trim().is_empty()
    }

    fn set_status(&self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.status.update(cx, |value, cx| {
            *value = status.into();
            cx.notify();
        });
    }
}

impl Focusable for DataCompareWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DataCompareWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_running = *self.is_running.read(cx);
        let is_executing = *self.is_executing.read(cx);
        let has_sync_sql = self.has_selected_sync_sql(cx);
        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .child(div().font_semibold().child("数据比较"))
            .child(self.render_source(cx))
            .child(self.render_target(cx))
            .child(self.render_result(cx))
            .child(div().flex_1().child(""))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(close_button())
                    .child(
                        Button::new("execute-sync-sql")
                            .child("Execute SQL")
                            .loading(is_executing)
                            .disabled(is_running || is_executing || !has_sync_sql)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.start_execute_sync_sql(cx);
                            })),
                    )
                    .child(
                        Button::new("compare")
                            .primary()
                            .loading(is_running)
                            .disabled(is_running || is_executing)
                            .child("Compare")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.start_compare(cx);
                            })),
                    ),
            )
    }
}
