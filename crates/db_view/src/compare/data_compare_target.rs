use db::GlobalDbState;
use extension_component::DbSelectorKind;
use gpui::{
    AsyncApp, Context, Entity, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder,
};
use gpui_component::{ActiveTheme, v_flex};
use rust_i18n::t;
use std::collections::HashSet;

use crate::compare::data_compare_window::DataCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_picker,
};
use crate::compare::table_picker::{
    TableSelectionListState, clear_table_selection_list, refresh_table_selection_list_app,
    table_selection_panel,
};
use crate::compare::target_picker::{
    CompareTargetCascadeAction, TargetConnectionControls, TargetStringControls,
    clear_string_select, database_change_cascade_actions, initial_compare_target_cascade_actions,
    load_databases_then, load_schemas_then, schema_change_cascade_actions, selected_string,
};
use crate::compare::window_ui::{
    compare_progress_view, input_row, register_connection_for_compare, section_title,
    selected_connection_id, stat_cards_row,
};
use crate::db_object_selector::{
    DbObjectSelectorControls, db_object_selector_panel, effective_database_schema,
    policy_for_connection,
};

impl DataCompareWindow {
    pub(super) fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(super) fn load_source_databases(&mut self, cx: &mut Context<Self>) {
        clear_string_select(&self.source_schema_select, cx);
        clear_table_selection_list(&self.source_table_list, &self.selected_source_tables, cx);
        load_databases_then(
            self.source_connection_controls(),
            self.source_database_controls(),
            self.status.clone(),
            cx,
            |this, cx| this.load_source_after_database_change(cx),
        );
    }

    pub(super) fn load_source_initial_cascade(&mut self, cx: &mut Context<Self>) {
        let policy = policy_for_connection(&self.source_connection_controls(), cx);
        let has_selected_database =
            !selected_string(&self.source_database_select, &self.source_database, cx)
                .trim()
                .is_empty();
        self.run_source_cascade_actions(
            initial_compare_target_cascade_actions(policy, has_selected_database),
            cx,
        );
    }

    pub(super) fn load_source_after_database_change(&mut self, cx: &mut Context<Self>) {
        let policy = policy_for_connection(&self.source_connection_controls(), cx);
        self.run_source_cascade_actions(database_change_cascade_actions(policy), cx);
    }

    pub(super) fn load_source_after_schema_change(&mut self, cx: &mut Context<Self>) {
        let has_selected_schema = self
            .source_schema_select
            .read(cx)
            .selected_value()
            .is_some();
        self.run_source_cascade_actions(schema_change_cascade_actions(has_selected_schema), cx);
    }

    fn run_source_cascade_actions(
        &mut self,
        actions: Vec<CompareTargetCascadeAction>,
        cx: &mut Context<Self>,
    ) {
        for action in actions {
            match action {
                CompareTargetCascadeAction::LoadDatabases => self.load_source_databases(cx),
                CompareTargetCascadeAction::LoadSchemas => self.load_source_schemas(cx),
                CompareTargetCascadeAction::LoadTables => self.load_source_tables(cx),
            }
        }
    }

    pub(super) fn load_source_schemas(&mut self, cx: &mut Context<Self>) {
        clear_table_selection_list(&self.source_table_list, &self.selected_source_tables, cx);
        load_schemas_then(
            self.source_connection_controls(),
            self.source_database_controls(),
            self.source_schema_controls(),
            self.status.clone(),
            cx,
            |this, cx| this.load_source_after_schema_change(cx),
        );
    }

    pub(super) fn load_source_tables(&mut self, cx: &mut Context<Self>) {
        self.load_table_list(
            self.source_connection_controls(),
            self.source_database_controls(),
            self.source_schema_controls(),
            self.source_table.clone(),
            self.source_table_list.clone(),
            self.selected_source_tables.clone(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn load_target_databases(&mut self, cx: &mut Context<Self>) {
        clear_string_select(&self.target_schema_select, cx);
        clear_table_selection_list(&self.target_table_list, &self.selected_target_tables, cx);
        load_databases_then(
            self.connection_controls(),
            self.database_controls(),
            self.status.clone(),
            cx,
            |this, cx| this.load_target_after_database_change(cx),
        );
    }

    pub(super) fn load_target_initial_cascade(&mut self, cx: &mut Context<Self>) {
        let policy = policy_for_connection(&self.connection_controls(), cx);
        let has_selected_database =
            !selected_string(&self.target_database_select, &self.target_database, cx)
                .trim()
                .is_empty();
        self.run_target_cascade_actions(
            initial_compare_target_cascade_actions(policy, has_selected_database),
            cx,
        );
    }

    pub(super) fn load_target_after_database_change(&mut self, cx: &mut Context<Self>) {
        let policy = policy_for_connection(&self.connection_controls(), cx);
        self.run_target_cascade_actions(database_change_cascade_actions(policy), cx);
    }

    pub(super) fn load_target_after_schema_change(&mut self, cx: &mut Context<Self>) {
        let has_selected_schema = self
            .target_schema_select
            .read(cx)
            .selected_value()
            .is_some();
        self.run_target_cascade_actions(schema_change_cascade_actions(has_selected_schema), cx);
    }

    fn run_target_cascade_actions(
        &mut self,
        actions: Vec<CompareTargetCascadeAction>,
        cx: &mut Context<Self>,
    ) {
        for action in actions {
            match action {
                CompareTargetCascadeAction::LoadDatabases => self.load_target_databases(cx),
                CompareTargetCascadeAction::LoadSchemas => self.load_target_schemas(cx),
                CompareTargetCascadeAction::LoadTables => self.load_target_tables(cx),
            }
        }
    }

    pub(super) fn load_target_schemas(&mut self, cx: &mut Context<Self>) {
        clear_table_selection_list(&self.target_table_list, &self.selected_target_tables, cx);
        load_schemas_then(
            self.connection_controls(),
            self.database_controls(),
            self.schema_controls(),
            self.status.clone(),
            cx,
            |this, cx| this.load_target_after_schema_change(cx),
        );
    }

    pub(super) fn load_target_tables(&mut self, cx: &mut Context<Self>) {
        self.load_table_list(
            self.connection_controls(),
            self.database_controls(),
            self.schema_controls(),
            self.target_table.clone(),
            self.target_table_list.clone(),
            self.selected_target_tables.clone(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(db_object_selector_panel(
                t!("Compare.target").to_string(),
                DbSelectorKind::Schema,
                self.target_controls(cx),
                cx,
            ))
            .child(table_selection_panel(
                t!("Compare.target_tables").to_string(),
                self.target_table_list.clone(),
                self.selected_target_tables.clone(),
                cx,
            ))
            .child(input_row(
                t!("Compare.key_columns").to_string(),
                &self.key_columns,
            ))
    }

    pub(super) fn render_result_meta(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self
            .result
            .read(cx)
            .as_ref()
            .map(|result| data_compare_batch_stats(result));
        let truncation = self
            .result
            .read(cx)
            .as_ref()
            .and_then(data_compare_batch_truncation_note);
        let progress = self.progress.read(cx).clone();
        let plan = self.sync_plan.read(cx).clone();
        let selected_ids = self.selected_statement_ids.read(cx).clone();
        let sync_summary = plan
            .as_ref()
            .map(|plan| selected_sync_sql_summary_for_ids(plan, &selected_ids));

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(section_title(t!("Compare.result").to_string()))
            .when_some(progress, |this, progress| {
                this.child(compare_progress_view(&progress, cx))
            })
            .when_some(stats, |this, (added, removed, modified)| {
                this.child(stat_cards_row(added, removed, modified, cx))
            })
            .when_some(truncation, |this, note| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when_some(sync_summary, |this, sync_summary| {
                this.child(div().text_sm().child(sync_summary))
            })
            .when_some(plan, |this, plan| {
                this.child(sync_statement_picker(
                    plan,
                    self.selected_statement_ids.clone(),
                    self.sync_statement_list.clone(),
                    cx,
                ))
            })
    }

    pub(super) fn source_connection_controls(&self) -> TargetConnectionControls {
        TargetConnectionControls {
            select: self.source_connection_select.clone(),
            fallback: self.source_connection_id.clone(),
        }
    }

    fn source_database_controls(&self) -> TargetStringControls {
        TargetStringControls {
            select: self.source_database_select.clone(),
            fallback: self.source_database.clone(),
        }
    }

    fn source_schema_controls(&self) -> TargetStringControls {
        TargetStringControls {
            select: self.source_schema_select.clone(),
            fallback: self.source_schema.clone(),
        }
    }

    fn source_controls(&self, cx: &Context<Self>) -> DbObjectSelectorControls {
        let connection = self.source_connection_controls();
        let policy = policy_for_connection(&connection, cx);
        DbObjectSelectorControls {
            connection,
            database: Some(self.source_database_controls()),
            schema: Some(self.source_schema_controls()),
            table: None,
            column: None,
            policy,
        }
    }

    pub(super) fn connection_controls(&self) -> TargetConnectionControls {
        TargetConnectionControls {
            select: self.target_connection_select.clone(),
            fallback: self.target_connection_id.clone(),
        }
    }

    fn database_controls(&self) -> TargetStringControls {
        TargetStringControls {
            select: self.target_database_select.clone(),
            fallback: self.target_database.clone(),
        }
    }

    fn schema_controls(&self) -> TargetStringControls {
        TargetStringControls {
            select: self.target_schema_select.clone(),
            fallback: self.target_schema.clone(),
        }
    }

    fn target_controls(&self, cx: &Context<Self>) -> DbObjectSelectorControls {
        let connection = self.connection_controls();
        let policy = policy_for_connection(&connection, cx);
        DbObjectSelectorControls {
            connection,
            database: Some(self.database_controls()),
            schema: Some(self.schema_controls()),
            table: None,
            column: None,
            policy,
        }
    }

    fn load_table_list(
        &self,
        connection: TargetConnectionControls,
        database: TargetStringControls,
        schema: TargetStringControls,
        preferred_table: Entity<gpui_component::input::InputState>,
        list_state: TableSelectionListState,
        selected_tables: Entity<HashSet<String>>,
        status: Entity<String>,
        cx: &mut Context<Self>,
    ) {
        let connection_id = selected_connection_id(&connection.select, &connection.fallback, cx);
        let database_name = selected_string(&database.select, &database.fallback, cx);
        let schema_name = selected_string(&schema.select, &schema.fallback, cx);
        let (database_name, schema_name) = effective_database_schema(
            database_name,
            schema_name,
            policy_for_connection(&connection, cx),
        );
        let preferred = preferred_table.read(cx).text().to_string();
        clear_table_selection_list(&list_state, &selected_tables, cx);

        if connection_id.trim().is_empty() || database_name.trim().is_empty() {
            set_status(
                &status,
                t!("DbObjectSelector.select_connection_database").to_string(),
                cx,
            );
            return;
        }

        register_connection_for_compare(&connection_id, cx);
        set_status(
            &status,
            t!("DbObjectSelector.loading_tables").to_string(),
            cx,
        );
        let db_state = cx.global::<GlobalDbState>().clone();
        let schema = (!schema_name.trim().is_empty()).then_some(schema_name);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = db_state
                .list_tables(cx, connection_id, database_name, schema)
                .await
                .map(|tables| {
                    tables
                        .into_iter()
                        .map(|table| table.name)
                        .collect::<Vec<_>>()
                });
            let _ = cx.update(|cx| match result {
                Ok(tables) => {
                    let count = tables.len();
                    refresh_table_selection_list_app(
                        &list_state,
                        &selected_tables,
                        tables,
                        preferred,
                        cx,
                    );
                    set_status_app(
                        &status,
                        t!("DbObjectSelector.loaded_count", count = count).to_string(),
                        cx,
                    );
                }
                Err(error) => set_status_app(
                    &status,
                    t!("DbObjectSelector.load_failed", error = error.to_string()).to_string(),
                    cx,
                ),
            });
        })
        .detach();
    }
}

fn set_status<T>(status: &Entity<String>, message: String, cx: &mut Context<T>) {
    status.update(cx, |status, cx| {
        *status = message;
        cx.notify();
    });
}

fn set_status_app(status: &Entity<String>, message: String, cx: &mut gpui::App) {
    status.update(cx, |status, cx| {
        *status = message;
        cx.notify();
    });
}

fn data_compare_batch_stats(
    result: &crate::compare::DataCompareBatchResult,
) -> (usize, usize, usize) {
    result
        .table_results
        .iter()
        .fold((0, 0, 0), |(added, removed, modified), table| {
            (
                added + table.added.len(),
                removed + table.removed.len(),
                modified + table.modified.len(),
            )
        })
}

fn data_compare_batch_truncation_note(
    result: &crate::compare::DataCompareBatchResult,
) -> Option<String> {
    let source_truncated = result
        .table_results
        .iter()
        .any(|table| table.source_truncated);
    let target_truncated = result
        .table_results
        .iter()
        .any(|table| table.target_truncated);
    crate::compare::window_ui::data_truncation_note(source_truncated, target_truncated)
}
