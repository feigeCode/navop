use db::{GlobalDbState, TableObjectType};
use extension_component::DbSelectorKind;
use gpui::{
    App, AppContext, AsyncApp, ColorExt, Context, Entity, IntoElement, ParentElement, Styled,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, ContentState, IconName, IndexPath, Sizable, StyledExt, h_flex,
    scroll::ScrollableElement,
    select::{SearchableVec, Select},
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use std::collections::HashSet;

use crate::compare::compare_result_feedback::{
    data_compare_failure_issues, failure_details_panel, hide_data_compare_failure_warnings,
};
use crate::compare::data_compare_window::DataCompareWindow;
use crate::compare::data_diff_detail::data_diff_detail_panel;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_empty_picker, sync_statement_picker,
};
use crate::compare::table_picker::{
    TableSelectionListState, clear_table_selection_list, refresh_table_selection_list_app,
    table_selection_list_tables, table_selection_panel,
};
use crate::compare::target_picker::{
    CompareTargetCascadeAction, StringSelect, TargetConnectionControls, TargetStringControls,
    clear_string_select, database_change_cascade_actions, initial_compare_target_cascade_actions,
    load_databases_then, load_schemas_then, schema_change_cascade_actions, selected_string,
};
use crate::compare::window_params::data_compare_same_name_mappings;
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
            None,
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn load_target_databases(&mut self, cx: &mut Context<Self>) {
        clear_string_select(&self.target_schema_select, cx);
        clear_string_select(&self.target_table_select, cx);
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
        clear_string_select(&self.target_table_select, cx);
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
            Some(self.target_table_select.clone()),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let source_tables = self.selected_source_table_names(cx);
        let available_target_tables = table_selection_list_tables(&self.target_table_list, cx);
        let is_single_table = source_tables.len() <= 1;

        v_flex()
            .size_full()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(db_object_selector_panel(
                t!("Compare.target").to_string(),
                DbSelectorKind::Schema,
                self.target_controls(cx),
                cx,
            ))
            .child(if is_single_table {
                self.render_single_target_table().into_any_element()
            } else {
                self.render_target_table_mappings(&source_tables, &available_target_tables, cx)
                    .into_any_element()
            })
            .child(input_row(
                t!("Compare.key_columns").to_string(),
                &self.key_columns,
            ))
    }

    fn render_single_target_table(&self) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("Compare.target_tables").to_string()),
            )
            .child(
                Select::new(&self.target_table_select)
                    .small()
                    .search_placeholder(t!("Compare.select_target_table").to_string())
                    .w_full(),
            )
    }

    fn render_target_table_mappings(
        &self,
        source_tables: &[String],
        available_target_tables: &[String],
        cx: &App,
    ) -> impl IntoElement {
        let case_sensitive_identifiers = !*self.ignore_identifier_case.read(cx);
        let mappings = data_compare_same_name_mappings(
            source_tables,
            available_target_tables,
            case_sensitive_identifiers,
        );
        let matched_count = mappings.iter().filter(|mapping| mapping.matched).count();
        let missing_tables = mappings
            .iter()
            .filter(|mapping| !mapping.matched)
            .map(|mapping| mapping.source_table.clone())
            .collect::<Vec<_>>();

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("Compare.auto_match_target_tables").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!(
                            "Compare.matched_target_tables",
                            matched = matched_count,
                            total = mappings.len()
                        )
                        .to_string(),
                    ),
            )
            .when(!missing_tables.is_empty(), |this| {
                this.child(
                    div().text_sm().text_color(cx.theme().danger).child(
                        t!(
                            "Compare.missing_target_tables",
                            tables = missing_tables.join(", ")
                        )
                        .to_string(),
                    ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .size_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_md()
                            .overflow_y_scrollbar()
                            .children(mappings.into_iter().enumerate().map(|(index, mapping)| {
                                h_flex()
                                    .min_h(px(34.0))
                                    .px_3()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .when(index % 2 == 1, |row| {
                                        row.bg(cx.theme().muted.opacity(0.18))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .child(mapping.source_table),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("→"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .text_color(if mapping.matched {
                                                cx.theme().foreground
                                            } else {
                                                cx.theme().danger
                                            })
                                            .child(mapping.target_table),
                                    )
                            })),
                    ),
            )
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
            .and_then(|result| data_compare_batch_truncation_note(result));
        let missing_target = self
            .result
            .read(cx)
            .as_ref()
            .filter(|result| result.has_missing_target_tables())
            .map(|_| t!("Compare.data_compare_missing_target_note").to_string());
        let failed_tables = self
            .result
            .read(cx)
            .as_ref()
            .filter(|result| result.has_failed_tables())
            .map(|result| {
                (
                    t!(
                        "Compare.data_compare_table_failed_note",
                        count = result.table_failures.len()
                    )
                    .to_string(),
                    data_compare_failure_issues(&result.table_failures),
                )
            });
        let dependency_metadata_warning = self
            .result
            .read(cx)
            .as_ref()
            .filter(|result| result.has_incomplete_dependency_metadata())
            .map(|_| t!("Compare.data_compare_dependency_metadata_failed_note").to_string());
        let progress = self.progress.read(cx).clone();
        let show_empty = self.result.read(cx).is_none() && progress.is_none();
        let plan = self.sync_plan.read(cx).clone();
        let selected_ids = self.selected_statement_ids.read(cx).clone();
        let sync_summary = plan
            .as_ref()
            .map(|plan| selected_sync_sql_summary_for_ids(plan, &selected_ids));

        v_flex()
            .size_full()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(section_title(t!("Compare.result").to_string()))
            .when_some(progress, |this, progress| {
                this.child(compare_progress_view(&progress, cx))
            })
            .when(show_empty, |this| {
                this.child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_h_0()
                        .min_w_0()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_md()
                        .bg(cx.theme().muted.opacity(0.28))
                        .overflow_hidden()
                        .child(
                            ContentState::empty(t!("Compare.data_compare_empty_title").to_string())
                                .icon(IconName::TableData)
                                .detail(t!("Compare.data_compare_empty_detail").to_string())
                                .detail_max_width(px(360.0)),
                        ),
                )
            })
            .when_some(stats, |this, (added, removed, modified)| {
                this.child(stat_cards_row(added, removed, modified, cx))
            })
            .when(self.result.read(cx).is_some(), |this| {
                this.child(data_diff_detail_panel(self.data_diff_list.clone(), cx))
            })
            .when_some(truncation, |this, note| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when_some(missing_target, |this, note| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when_some(failed_tables, |this, (summary, issues)| {
                this.child(failure_details_panel(
                    "data-compare-failures",
                    "toggle-data-compare-failures",
                    summary,
                    issues,
                    self.failure_details_expanded.clone(),
                    cx,
                ))
            })
            .when_some(dependency_metadata_warning, |this, note| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when_some(sync_summary, |this, sync_summary| {
                this.child(div().text_sm().child(sync_summary))
            })
    }

    pub(super) fn render_sync_statement_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut plan = self.sync_plan.read(cx).clone();
        if let (Some(plan), Some(result)) = (plan.as_mut(), self.result.read(cx).as_ref()) {
            hide_data_compare_failure_warnings(plan, &result.table_failures);
        }
        let show_empty = plan.is_none();

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .when(show_empty, |this| {
                this.child(sync_statement_empty_picker(cx))
            })
            .when_some(plan, |this, plan| {
                this.child(sync_statement_picker(
                    plan,
                    self.selected_statement_ids.clone(),
                    self.sync_statement_list.clone(),
                    self.sync_warnings_expanded.clone(),
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
        table_select: Option<StringSelect>,
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
            let result = Tokio::spawn_result(cx, async move {
                db_state
                    .list_tables_direct(&connection_id, &database_name, schema)
                    .await
                    .map(|tables| {
                        tables
                            .into_iter()
                            .filter(|table| table.object_type == TableObjectType::Table)
                            .map(|table| table.name)
                            .collect::<Vec<_>>()
                    })
            })
            .await;
            let _ = cx.update(|cx| match result {
                Ok(tables) => {
                    let count = tables.len();
                    refresh_table_selection_list_app(
                        &list_state,
                        &selected_tables,
                        tables.clone(),
                        preferred.clone(),
                        cx,
                    );
                    if let Some(table_select) = table_select {
                        update_target_table_select_app(
                            &table_select,
                            &preferred_table,
                            tables,
                            preferred,
                            cx,
                        );
                    }
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

    pub(super) fn replace_target_table_select_options(
        &self,
        tables: Vec<String>,
        preferred: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        replace_string_select_options(
            &self.target_table_select,
            &self.target_table,
            tables,
            preferred,
            window,
            cx,
        );
    }

    pub(super) fn sync_single_target_table_to_source(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_tables = self.selected_source_table_names(cx);
        if source_tables.len() != 1 {
            return;
        }
        let target_tables = table_selection_list_tables(&self.target_table_list, cx);
        if target_tables.is_empty() {
            return;
        }
        let case_sensitive_identifiers = !*self.ignore_identifier_case.read(cx);
        let preferred = target_tables
            .iter()
            .find(|table| {
                if case_sensitive_identifiers {
                    *table == &source_tables[0]
                } else {
                    table.eq_ignore_ascii_case(&source_tables[0])
                }
            })
            .cloned()
            .unwrap_or_else(|| target_tables[0].clone());
        replace_string_select_options(
            &self.target_table_select,
            &self.target_table,
            target_tables,
            preferred,
            window,
            cx,
        );
    }
}

fn update_target_table_select_app(
    select: &StringSelect,
    fallback: &Entity<gpui_component::input::InputState>,
    tables: Vec<String>,
    preferred: String,
    cx: &mut App,
) {
    let Some(window_id) = cx.active_window() else {
        return;
    };
    let _ = cx.update_window(window_id, |_, window, cx| {
        replace_string_select_options(select, fallback, tables, preferred, window, cx);
    });
}

fn replace_string_select_options<C: AppContext>(
    select: &StringSelect,
    fallback: &Entity<gpui_component::input::InputState>,
    tables: Vec<String>,
    preferred: String,
    window: &mut Window,
    cx: &mut C,
) {
    let (selected_index, selected_value) = preferred_table_selection(&tables, &preferred);
    fallback.update(cx, |input, cx| {
        input.set_value(selected_value, window, cx);
    });
    select.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(tables), window, cx);
        state.set_selected_index(selected_index, window, cx);
    });
}

fn preferred_table_selection(tables: &[String], preferred: &str) -> (Option<IndexPath>, String) {
    let selected_row = tables
        .iter()
        .position(|table| table == preferred)
        .or_else(|| {
            tables
                .iter()
                .position(|table| table.eq_ignore_ascii_case(preferred))
        })
        .or((!tables.is_empty()).then_some(0));
    (
        selected_row.map(IndexPath::new),
        selected_row
            .and_then(|row| tables.get(row).cloned())
            .unwrap_or_default(),
    )
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
