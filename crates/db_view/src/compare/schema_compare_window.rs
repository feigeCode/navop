use std::collections::HashSet;
use std::sync::Arc;

use db::{DbNode, DbNodeType, GlobalDbState};
use extension_component::DbSelectorKind;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, ScrollHandle, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::InputState,
    select::{SearchableVec, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use rust_i18n::t;
use tokio::sync::mpsc;

use crate::compare::sync_statement_picker::{
    SyncStatementListState, clear_sync_statement_list, default_selected_statement_ids,
    refresh_sync_statement_list, selected_sync_sql_text_for_ids, sync_statement_list_state,
};
use crate::compare::table_picker::{
    TableSelectionListState, replace_table_selection_list, table_selection_list_state,
    table_selection_list_tables, table_selection_panel,
};
use crate::compare::target_picker::{
    StringSelect, selected_string, set_connection_select, set_string_select, string_select_state,
};
use crate::compare::window_params::{
    SchemaCompareSelection, SchemaCompareSettings, schema_compare_params,
};
use crate::compare::window_ui::{
    CompareStep, ConnectionSelectItem, SyncSqlExecutionLogEntry, clear_sync_sql_execution_log,
    close_button, connection_select_state, ignore_identifier_case_option,
    register_connection_for_compare, reset_sync_sql_execution_log, section_title,
    selected_connection_id, sql_editor_panel, start_sync_sql_execution, sync_sql_editor_state,
    sync_sql_execution_continue_on_error_row, sync_sql_execution_log_panel,
    sync_sql_execution_start_log_entries,
};
use crate::compare::{
    CompareProgress, CompareSyncExecutionOptions, CompareTargetScope, SchemaCompareParams,
    execute_schema_compare, generate_schema_sync_plan_for_target,
};
use crate::db_object_selector::{
    DbObjectSelectorPolicy, db_object_selector_panel, effective_database_schema,
    policy_for_connection,
};
use db::compare::{SchemaCompareResult, SyncPlan};

/// 结构比较弹出窗口
pub struct SchemaCompareWindow {
    pub(super) source_connection_id: Entity<InputState>,
    pub(super) source_connection_select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    pub(super) source_database: Entity<InputState>,
    pub(super) source_database_select: StringSelect,
    pub(super) source_schema: Entity<InputState>,
    pub(super) source_schema_select: StringSelect,
    pub(super) source_table: Entity<InputState>,
    pub(super) selected_source_tables: Entity<HashSet<String>>,
    pub(super) source_table_list: TableSelectionListState,
    pub(super) target_connection_id: Entity<InputState>,
    pub(super) target_connection_select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    pub(super) target_database: Entity<InputState>,
    pub(super) target_database_select: StringSelect,
    pub(super) target_schema: Entity<InputState>,
    pub(super) target_schema_select: StringSelect,
    pub(super) target_table: Entity<InputState>,
    pub(super) selected_target_tables: Entity<HashSet<String>>,
    pub(super) target_table_list: TableSelectionListState,
    pub(super) ignore_identifier_case: Entity<bool>,
    compare_indexes: Entity<bool>,
    compare_foreign_keys: Entity<bool>,
    ignore_comments: Entity<bool>,
    ignore_auto_increment: Entity<bool>,
    ignore_charset_collation: Entity<bool>,
    ignore_table_options: Entity<bool>,
    pub(super) result: Entity<Option<SchemaCompareResult>>,
    pub(super) sync_plan: Entity<Option<SyncPlan>>,
    pub(super) selected_statement_ids: Entity<HashSet<String>>,
    pub(super) sync_statement_list: SyncStatementListState,
    pub(super) sync_sql_editor: Entity<InputState>,
    pub(super) execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    pub(super) execution_log_scroll: ScrollHandle,
    continue_on_error: Entity<bool>,
    pub(super) progress: Entity<Option<CompareProgress>>,
    compare_target: Entity<Option<CompareTargetScope>>,
    pub(super) status: Entity<String>,
    current_step: CompareStep,
    is_running: Entity<bool>,
    is_executing: Entity<bool>,
    compare_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SchemaCompareWindow {
    pub fn new(source_node: DbNode, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let default_database = source_node.get_database_name().unwrap_or_default();
        let default_schema = source_node.get_schema_name().unwrap_or_default();
        let default_policy = db_object_policy_for_source(&source_node, cx);
        let default_database = if default_policy.schema_as_database {
            default_schema.clone()
        } else {
            default_database
        };
        let default_schema = if default_policy.show_schema {
            default_schema
        } else {
            String::new()
        };
        let default_selected_tables = Self::initial_selected_tables_for_node(&source_node);
        let default_table = default_selected_tables
            .iter()
            .next()
            .cloned()
            .unwrap_or_default();

        let source_connection_id = cx
            .new(|cx| InputState::new(window, cx).default_value(source_node.connection_id.clone()));
        let source_connection_select =
            connection_select_state(&source_node.connection_id, window, cx);
        let source_database =
            cx.new(|cx| InputState::new(window, cx).default_value(default_database.clone()));
        let source_database_select = string_select_state(default_database.clone(), window, cx);
        let source_schema =
            cx.new(|cx| InputState::new(window, cx).default_value(default_schema.clone()));
        let source_schema_select = string_select_state(default_schema.clone(), window, cx);
        let source_table =
            cx.new(|cx| InputState::new(window, cx).default_value(default_table.clone()));
        let target_connection_id = cx
            .new(|cx| InputState::new(window, cx).default_value(source_node.connection_id.clone()));
        let target_connection_select =
            connection_select_state(&source_node.connection_id, window, cx);
        let target_database =
            cx.new(|cx| InputState::new(window, cx).default_value(default_database.clone()));
        let target_database_select = string_select_state(default_database.clone(), window, cx);
        let target_schema =
            cx.new(|cx| InputState::new(window, cx).default_value(default_schema.clone()));
        let target_schema_select = string_select_state(default_schema.clone(), window, cx);
        let target_table =
            cx.new(|cx| InputState::new(window, cx).default_value(default_table.clone()));
        let ignore_identifier_case = cx.new(|_| true);
        let compare_indexes = cx.new(|_| true);
        let compare_foreign_keys = cx.new(|_| true);
        let ignore_comments = cx.new(|_| false);
        let ignore_auto_increment = cx.new(|_| false);
        let ignore_charset_collation = cx.new(|_| false);
        let ignore_table_options = cx.new(|_| false);
        let sync_sql_editor = sync_sql_editor_state(window, cx);
        let execution_log_scroll = ScrollHandle::new();

        let view = cx.new(|cx: &mut Context<Self>| {
            let selected_statement_ids = cx.new(|_| HashSet::new());
            let sync_statement_list =
                sync_statement_list_state(selected_statement_ids.clone(), window, cx);
            let selected_source_tables = cx.new({
                let default_selected_tables = default_selected_tables.clone();
                move |_| default_selected_tables.clone()
            });
            let source_table_list =
                table_selection_list_state(selected_source_tables.clone(), window, cx);
            let selected_target_tables = cx.new({
                let default_selected_tables = default_selected_tables.clone();
                move |_| default_selected_tables.clone()
            });
            let target_table_list =
                table_selection_list_state(selected_target_tables.clone(), window, cx);
            let mut window_state = Self {
                source_connection_id,
                source_connection_select,
                source_database,
                source_database_select,
                source_schema,
                source_schema_select,
                source_table,
                selected_source_tables,
                source_table_list,
                target_connection_id,
                target_connection_select,
                target_database,
                target_database_select,
                target_schema,
                target_schema_select,
                target_table,
                selected_target_tables,
                target_table_list,
                ignore_identifier_case,
                compare_indexes,
                compare_foreign_keys,
                ignore_comments,
                ignore_auto_increment,
                ignore_charset_collation,
                ignore_table_options,
                sync_sql_editor,
                result: cx.new(|_| None),
                sync_plan: cx.new(|_| None),
                selected_statement_ids,
                sync_statement_list,
                execution_log: cx.new(|_| Vec::new()),
                execution_log_scroll,
                continue_on_error: cx
                    .new(|_| CompareSyncExecutionOptions::default().continue_on_error),
                progress: cx.new(|_| None),
                compare_target: cx.new(|_| None),
                status: cx.new(|_| t!("Compare.ready").to_string()),
                current_step: CompareStep::Objects,
                is_running: cx.new(|_| false),
                is_executing: cx.new(|_| false),
                compare_task: None,
                focus_handle: cx.focus_handle(),
                _subscriptions: Vec::new(),
            };
            // 选中语句变化时(比较完成、勾选、批量选择)刷新 SQL 编辑器内容
            let sub = cx.observe_in(
                &window_state.selected_statement_ids,
                window,
                |this, _, window, cx| {
                    this.refresh_sync_editor(window, cx);
                    this.sync_statement_list.update(cx, |_, cx| cx.notify());
                },
            );
            window_state._subscriptions.push(sub);
            // 源级联:连接 → 数据库 → Schema
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_connection_select,
                |this, _, _event: &SelectEvent<SearchableVec<ConnectionSelectItem>>, cx| {
                    this.load_source_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_source_after_database_change(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.source_schema_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_source_after_schema_change(cx);
                },
            ));
            // 目标级联:连接 → 数据库 → Schema
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_connection_select,
                |this, _, _event: &SelectEvent<SearchableVec<ConnectionSelectItem>>, cx| {
                    this.load_target_databases(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_database_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_target_after_database_change(cx);
                },
            ));
            window_state._subscriptions.push(cx.subscribe(
                &window_state.target_schema_select,
                |this, _, _event: &SelectEvent<SearchableVec<String>>, cx| {
                    this.load_target_after_schema_change(cx);
                },
            ));
            window_state
        });
        view.update(cx, |this, cx| {
            if !selected_connection_id(
                &this.source_connection_select,
                &this.source_connection_id,
                cx,
            )
            .trim()
            .is_empty()
            {
                this.load_source_initial_cascade(cx);
            }
            if !selected_connection_id(
                &this.target_connection_select,
                &this.target_connection_id,
                cx,
            )
            .trim()
            .is_empty()
            {
                this.load_target_initial_cascade(cx);
            }
        });
        view
    }

    pub(crate) fn initial_selected_tables_for_node(source_node: &DbNode) -> HashSet<String> {
        if source_node.node_type != DbNodeType::Table {
            return HashSet::new();
        }
        source_node
            .get_table_name()
            .filter(|table| !table.trim().is_empty())
            .into_iter()
            .collect()
    }

    pub fn popup_title_for(source_node: &DbNode) -> String {
        t!(
            "Compare.schema_compare_title",
            name = source_node.name.clone()
        )
        .to_string()
    }

    fn start_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(message) => {
                self.set_status(message, cx);
                return;
            }
        };
        let compare_target = CompareTargetScope::from_schema_params(&params);
        clear_sync_sql_execution_log(&self.execution_log, &self.execution_log_scroll, cx);
        self.clear_compare_preview(window, cx);
        register_connection_for_compare(&params.source_connection_id, cx);
        register_connection_for_compare(&params.target_connection_id, cx);
        let target_connection_id = params.target_connection_id.clone();
        let target_database = params.target_database.clone();
        let target_schema = params.target_schema.clone();
        let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
        self.is_running.update(cx, |running, cx| {
            *running = true;
            cx.notify();
        });
        self.set_progress(
            Some(CompareProgress::phase(
                t!("Compare.preparing_compare").to_string(),
            )),
            cx,
        );
        self.set_status(t!("Compare.comparing_schema").to_string(), cx);

        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<CompareProgress>();

        // 进度接收循环:随执行器任务结束(发送端关闭)而退出
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Some(progress) = progress_rx.recv().await {
                if this
                    .update(cx, |view, cx| view.set_progress(Some(progress), cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result =
                match execute_schema_compare(params, db_state.clone(), progress_tx, cx).await {
                    Ok(result) => generate_schema_sync_plan_for_target(
                        &result,
                        &db_state,
                        &target_connection_id,
                        &target_database,
                        target_schema.as_deref(),
                    )
                    .map(|plan| (result, plan)),
                    Err(error) => Err(error),
                };
            let _ = this.update(cx, |view, cx| {
                view.is_running.update(cx, |running, cx| {
                    *running = false;
                    cx.notify();
                });
                view.set_progress(None, cx);
                match result {
                    Ok((result, plan)) => {
                        let selected_ids = default_selected_statement_ids(&plan);
                        refresh_sync_statement_list(&view.sync_statement_list, &plan, cx);
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
                        view.current_step = CompareStep::SqlPreview;
                        view.set_status(t!("Compare.schema_compare_complete").to_string(), cx);
                    }
                    Err(error) => view.set_status(
                        t!("Compare.compare_failed", error = error.to_string()).to_string(),
                        cx,
                    ),
                }
                cx.notify();
            });
        });
        self.compare_task = Some(task);
    }

    fn swap_source_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source_selection(cx);
        let target = self.target_selection(cx);
        let source_tables = source.tables.clone();
        let target_tables = target.tables.clone();
        let source_list_tables = table_selection_list_tables(&self.source_table_list, cx);
        let target_list_tables = table_selection_list_tables(&self.target_table_list, cx);
        let source_list_tables = table_items_or_selection(source_list_tables, &source_tables);
        let target_list_tables = table_items_or_selection(target_list_tables, &target_tables);

        set_connection_select(
            &self.source_connection_select,
            &self.source_connection_id,
            &target.connection_id,
            window,
            cx,
        );
        set_connection_select(
            &self.target_connection_select,
            &self.target_connection_id,
            &source.connection_id,
            window,
            cx,
        );
        set_string_select(
            &self.source_database_select,
            &self.source_database,
            target.database,
            window,
            cx,
        );
        set_string_select(
            &self.target_database_select,
            &self.target_database,
            source.database,
            window,
            cx,
        );
        set_string_select(
            &self.source_schema_select,
            &self.source_schema,
            target.schema,
            window,
            cx,
        );
        set_string_select(
            &self.target_schema_select,
            &self.target_schema,
            source.schema,
            window,
            cx,
        );
        self.source_table.update(cx, |state, cx| {
            state.set_value(first_table_name(&target_tables), window, cx);
        });
        self.target_table.update(cx, |state, cx| {
            state.set_value(first_table_name(&source_tables), window, cx);
        });
        replace_table_selection_list(
            &self.source_table_list,
            &self.selected_source_tables,
            target_list_tables,
            target_tables.iter().cloned().collect(),
            cx,
        );
        replace_table_selection_list(
            &self.target_table_list,
            &self.selected_target_tables,
            source_list_tables,
            source_tables.iter().cloned().collect(),
            cx,
        );

        self.result.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.sync_plan.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.compare_target.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        clear_sync_sql_execution_log(&self.execution_log, &self.execution_log_scroll, cx);
        self.current_step = CompareStep::Objects;
        self.set_status(t!("Compare.swapped_source_target").to_string(), cx);
    }

    fn cancel_compare(&mut self, cx: &mut Context<Self>) {
        // 丢弃任务句柄即取消执行器 future,并关闭进度通道
        self.compare_task = None;
        self.is_running.update(cx, |running, cx| {
            *running = false;
            cx.notify();
        });
        self.set_progress(None, cx);
        self.set_status(t!("Compare.cancelled").to_string(), cx);
        cx.notify();
    }

    /// 根据当前选中的语句刷新 SQL 编辑器内容(可被用户后续手动编辑)
    fn refresh_sync_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.selected_sync_sql(cx);
        self.sync_sql_editor.update(cx, |state, cx| {
            state.set_value(sql, window, cx);
        });
    }

    fn build_params(&self, cx: &mut Context<Self>) -> Result<SchemaCompareParams, &'static str> {
        schema_compare_params(
            self.source_selection(cx),
            self.target_selection(cx),
            self.schema_compare_settings(cx),
        )
    }

    fn schema_compare_settings(&self, cx: &Context<Self>) -> SchemaCompareSettings {
        SchemaCompareSettings {
            case_sensitive_identifiers: !*self.ignore_identifier_case.read(cx),
            compare_indexes: *self.compare_indexes.read(cx),
            compare_foreign_keys: *self.compare_foreign_keys.read(cx),
            ignore_comments: *self.ignore_comments.read(cx),
            ignore_auto_increment: *self.ignore_auto_increment.read(cx),
            ignore_charset_collation: *self.ignore_charset_collation.read(cx),
            ignore_table_options: *self.ignore_table_options.read(cx),
        }
    }

    fn start_execute_sync_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        start_sync_sql_execution(
            self.compare_target.read(cx).clone(),
            self.editor_sql(cx),
            self.sync_execution_options(cx),
            self.status.clone(),
            self.is_executing.clone(),
            self.execution_log.clone(),
            self.execution_log_scroll.clone(),
            window,
            cx,
        );
    }

    fn go_previous_step(&mut self, cx: &mut Context<Self>) {
        if let Some(step) = self.current_step.previous() {
            self.current_step = step;
            cx.notify();
        }
    }

    fn go_preview_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_step == CompareStep::Objects {
            self.clear_compare_preview(window, cx);
            self.current_step = CompareStep::SqlPreview;
            self.set_status(t!("Compare.ready").to_string(), cx);
            cx.notify();
        }
    }

    fn go_execute_step(&mut self, cx: &mut Context<Self>) {
        if self.current_step == CompareStep::SqlPreview {
            let entries = sync_sql_execution_start_log_entries(&self.editor_sql(cx));
            if let Some(entry) = entries.first() {
                self.set_status(entry.message.clone(), cx);
            }
            reset_sync_sql_execution_log(
                &self.execution_log,
                &self.execution_log_scroll,
                entries,
                cx,
            );
            self.current_step = CompareStep::SqlExecute;
            cx.notify();
        }
    }

    /// 编辑器中实际待执行的 SQL(用户可能已手动修改)
    fn editor_sql(&self, cx: &Context<Self>) -> String {
        self.sync_sql_editor.read(cx).text().to_string()
    }

    /// 由选中语句生成的 SQL,用于填充编辑器
    fn selected_sync_sql(&self, cx: &Context<Self>) -> String {
        let selected_ids = self.selected_statement_ids.read(cx);
        self.sync_plan
            .read(cx)
            .as_ref()
            .map_or_else(String::new, |plan| {
                selected_sync_sql_text_for_ids(plan, selected_ids)
            })
    }

    fn has_editor_sql(&self, cx: &Context<Self>) -> bool {
        !self.editor_sql(cx).trim().is_empty()
    }

    fn clear_compare_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.result.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.sync_plan.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.compare_target.update(cx, |slot, cx| {
            *slot = None;
            cx.notify();
        });
        self.selected_statement_ids.update(cx, |slot, cx| {
            slot.clear();
            cx.notify();
        });
        clear_sync_statement_list(&self.sync_statement_list, cx);
        self.sync_sql_editor.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
        });
    }

    fn sync_execution_options(&self, cx: &Context<Self>) -> CompareSyncExecutionOptions {
        CompareSyncExecutionOptions::schema_ddl(*self.continue_on_error.read(cx))
    }

    fn set_progress(&self, progress: Option<CompareProgress>, cx: &mut Context<Self>) {
        self.progress.update(cx, |slot, cx| {
            *slot = progress;
            cx.notify();
        });
    }

    fn set_status(&self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.status.update(cx, |value, cx| {
            *value = status.into();
            cx.notify();
        });
    }

    fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(db_object_selector_panel(
                t!("Compare.source").to_string(),
                DbSelectorKind::Schema,
                self.source_controls(cx),
                cx,
            ))
            .child(table_selection_panel(
                t!("Compare.source_tables").to_string(),
                self.source_table_list.clone(),
                self.selected_source_tables.clone(),
                cx,
            ))
    }

    fn render_schema_compare_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
                    .child(section_title(t!("Compare.object_types").to_string()))
                    .child(
                        h_flex()
                            .gap_3()
                            .flex_wrap()
                            .child(schema_object_type_option(
                                "schema-object-tables",
                                t!("Compare.object_tables").to_string(),
                                true,
                                true,
                                None,
                                cx,
                            ))
                            .child(schema_object_type_option(
                                "schema-object-indexes",
                                t!("Compare.object_indexes").to_string(),
                                true,
                                false,
                                Some(self.compare_indexes.clone()),
                                cx,
                            ))
                            .child(schema_object_type_option(
                                "schema-object-foreign-keys",
                                t!("Compare.object_foreign_keys").to_string(),
                                true,
                                false,
                                Some(self.compare_foreign_keys.clone()),
                                cx,
                            ))
                            .child(schema_object_type_option(
                                "schema-object-views",
                                t!("Compare.object_views").to_string(),
                                false,
                                true,
                                None,
                                cx,
                            ))
                            .child(schema_object_type_option(
                                "schema-object-routines",
                                t!("Compare.object_routines").to_string(),
                                false,
                                true,
                                None,
                                cx,
                            ))
                            .child(schema_object_type_option(
                                "schema-object-triggers",
                                t!("Compare.object_triggers").to_string(),
                                false,
                                true,
                                None,
                                cx,
                            )),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
                    .child(section_title(t!("Compare.compare_rules").to_string()))
                    .child(
                        h_flex()
                            .gap_3()
                            .flex_wrap()
                            .child(compare_rule_switch(
                                "schema-ignore-auto-increment",
                                self.ignore_auto_increment.clone(),
                                t!("Compare.ignore_auto_increment").to_string(),
                                cx,
                            ))
                            .child(compare_rule_switch(
                                "schema-ignore-charset-collation",
                                self.ignore_charset_collation.clone(),
                                t!("Compare.ignore_charset_collation").to_string(),
                                cx,
                            ))
                            .child(compare_rule_switch(
                                "schema-ignore-table-options",
                                self.ignore_table_options.clone(),
                                t!("Compare.ignore_table_options").to_string(),
                                cx,
                            ))
                            .child(compare_rule_switch(
                                "schema-ignore-comments",
                                self.ignore_comments.clone(),
                                t!("Compare.ignore_comments").to_string(),
                                cx,
                            )),
                    ),
            )
    }

    fn source_selection(&self, cx: &Context<Self>) -> SchemaCompareSelection {
        let database = selected_string(&self.source_database_select, &self.source_database, cx);
        let schema = selected_string(&self.source_schema_select, &self.source_schema, cx);
        let (database, schema) = effective_database_schema(
            database,
            schema,
            policy_for_connection(&self.source_connection_controls(), cx),
        );
        SchemaCompareSelection {
            connection_id: selected_connection_id(
                &self.source_connection_select,
                &self.source_connection_id,
                cx,
            ),
            database,
            schema,
            tables: selected_schema_table_names(
                &self.source_table_list,
                &self.selected_source_tables,
                cx,
            ),
        }
    }

    fn target_selection(&self, cx: &Context<Self>) -> SchemaCompareSelection {
        let database = selected_string(&self.target_database_select, &self.target_database, cx);
        let schema = selected_string(&self.target_schema_select, &self.target_schema, cx);
        let (database, schema) = effective_database_schema(
            database,
            schema,
            policy_for_connection(&self.connection_controls(), cx),
        );
        SchemaCompareSelection {
            connection_id: selected_connection_id(
                &self.target_connection_select,
                &self.target_connection_id,
                cx,
            ),
            database,
            schema,
            tables: selected_schema_table_names(
                &self.target_table_list,
                &self.selected_target_tables,
                cx,
            ),
        }
    }
}

fn selected_schema_table_names<T>(
    list_state: &TableSelectionListState,
    selected_tables: &Entity<HashSet<String>>,
    cx: &Context<T>,
) -> Vec<String> {
    let selected = selected_tables.read(cx);
    if selected.is_empty() {
        return Vec::new();
    }
    let ordered = table_selection_list_tables(list_state, cx)
        .into_iter()
        .filter(|table| selected.contains(table))
        .collect::<Vec<_>>();
    if !ordered.is_empty() {
        return ordered;
    }
    let mut selected = selected.iter().cloned().collect::<Vec<_>>();
    selected.sort();
    selected
}

fn first_table_name(tables: &[String]) -> String {
    tables.first().cloned().unwrap_or_default()
}

fn table_items_or_selection(items: Vec<String>, selected: &[String]) -> Vec<String> {
    if items.is_empty() {
        selected.to_vec()
    } else {
        items
    }
}

fn schema_object_type_option<T: 'static>(
    id: &'static str,
    label: String,
    checked: bool,
    disabled: bool,
    state: Option<Entity<bool>>,
    cx: &Context<T>,
) -> impl IntoElement {
    let is_checked = state.as_ref().map_or(checked, |state| *state.read(cx));
    let state_for_click = state.clone();
    h_flex()
        .gap_1()
        .items_center()
        .child(
            Checkbox::new(id)
                .checked(is_checked)
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    if let Some(state) = &state_for_click {
                        state.update(cx, |value, cx| {
                            *value = !*value;
                            cx.notify();
                        });
                    }
                }),
        )
        .child(
            div()
                .text_sm()
                .text_color(if disabled {
                    cx.theme().muted_foreground
                } else {
                    cx.theme().foreground
                })
                .child(label),
        )
}

fn compare_rule_switch<T: 'static>(
    id: &'static str,
    state: Entity<bool>,
    label: String,
    cx: &Context<T>,
) -> impl IntoElement {
    let checked = *state.read(cx);
    Switch::new(id)
        .small()
        .checked(checked)
        .label(label)
        .on_click(move |checked, _, cx| {
            state.update(cx, |value, cx| {
                *value = *checked;
                cx.notify();
            });
        })
}

fn db_object_policy_for_source(source_node: &DbNode, cx: &mut App) -> DbObjectSelectorPolicy {
    cx.try_global::<GlobalDbState>()
        .map(|db_state| {
            DbObjectSelectorPolicy::from_capabilities(
                &db_state.capabilities(&source_node.database_type),
            )
        })
        .unwrap_or_default()
}

impl Focusable for SchemaCompareWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SchemaCompareWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_running = *self.is_running.read(cx);
        let is_executing = *self.is_executing.read(cx);
        let has_sync_sql = self.has_editor_sql(cx);
        let status = if self.current_step == CompareStep::SqlExecute {
            String::new()
        } else {
            self.status.read(cx).clone()
        };
        let editor_sql = self.sync_sql_editor.read(cx).text().to_string();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .overflow_hidden()
            .child(
                div()
                    .font_semibold()
                    .child(t!("Compare.schema_compare").to_string()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .gap_4()
                    .when(self.current_step == CompareStep::Objects, |this| {
                        this.child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_3()
                                .child(
                                    h_flex().justify_center().child(
                                        Button::new("swap-schema-compare-source-target")
                                            .icon(IconName::Replace)
                                            .tooltip(t!("Compare.swap_source_target").to_string())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.swap_source_target(window, cx);
                                            })),
                                    ),
                                )
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_h_0()
                                        .gap_4()
                                        .child(
                                            div()
                                                .flex_1()
                                                .h_full()
                                                .min_h_0()
                                                .child(self.render_source(cx)),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .h_full()
                                                .min_h_0()
                                                .child(self.render_target(cx)),
                                        ),
                                )
                                .child(ignore_identifier_case_option(
                                    "schema-compare-ignore-identifier-case",
                                    self.ignore_identifier_case.clone(),
                                    cx,
                                ))
                                .child(self.render_schema_compare_options(cx)),
                        )
                    })
                    .when(self.current_step == CompareStep::SqlPreview, |this| {
                        this.child(
                            h_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_4()
                                .child(
                                    div()
                                        .flex_1()
                                        .h_full()
                                        .min_h_0()
                                        .child(self.render_result_meta(cx)),
                                )
                                .child(div().flex_1().h_full().min_h_0().child(sql_editor_panel(
                                    "schema-compare-copy-sql",
                                    &self.sync_sql_editor,
                                    editor_sql,
                                    cx,
                                ))),
                        )
                    })
                    .when(self.current_step == CompareStep::SqlExecute, |this| {
                        this.child(
                            v_flex()
                                .flex_1()
                                .h_full()
                                .min_h_0()
                                .overflow_hidden()
                                .gap_2()
                                .child(sync_sql_execution_continue_on_error_row(
                                    self.continue_on_error.clone(),
                                    is_executing,
                                    cx,
                                ))
                                .child(sync_sql_execution_log_panel(
                                    &self.execution_log,
                                    &self.execution_log_scroll,
                                    is_executing,
                                    cx,
                                )),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(status),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(close_button())
                            .when(is_running, |this| {
                                this.child(
                                    Button::new("cancel-compare")
                                        .danger()
                                        .child(t!("Common.cancel").to_string())
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.cancel_compare(cx);
                                        })),
                                )
                            })
                            .when(self.current_step == CompareStep::Objects, |this| {
                                this.child(
                                    Button::new("compare-next")
                                        .primary()
                                        .disabled(is_running || is_executing)
                                        .child(t!("Common.next").to_string())
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.go_preview_step(window, cx);
                                        })),
                                )
                            })
                            .when(self.current_step == CompareStep::SqlPreview, |this| {
                                this.child(
                                    Button::new("compare-prev")
                                        .child(t!("Common.previous").to_string())
                                        .disabled(is_running || is_executing)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.go_previous_step(cx);
                                        })),
                                )
                                .child(
                                    Button::new("compare-start")
                                        .primary()
                                        .loading(is_running)
                                        .disabled(is_running || is_executing)
                                        .child(t!("Compare.start_compare").to_string())
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.start_compare(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("compare-preview-next")
                                        .disabled(is_running || is_executing || !has_sync_sql)
                                        .child(t!("Common.next").to_string())
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.go_execute_step(cx);
                                        })),
                                )
                            })
                            .when(self.current_step == CompareStep::SqlExecute, |this| {
                                this.child(
                                    Button::new("compare-execute-prev")
                                        .child(t!("Common.previous").to_string())
                                        .disabled(is_running || is_executing)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.go_previous_step(cx);
                                        })),
                                )
                                .child(
                                    Button::new("execute-sync-sql")
                                        .primary()
                                        .child(t!("Compare.execute_sql").to_string())
                                        .loading(is_executing)
                                        .disabled(is_running || is_executing || !has_sync_sql)
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.start_execute_sync_sql(window, cx);
                                        })),
                                )
                            }),
                    ),
            )
    }
}
