use db::{
    GlobalDbState, TableObjectType,
    compare::{
        ColumnSchema, DiffStatus, ForeignKeySchema, IndexSchema, RoutineDiff, RoutineKind,
        RoutineSchema, SchemaCompareResult, SchemaObjectType, TableDiff, TriggerDiff,
        TriggerSchema,
    },
};
use extension_component::DbSelectorKind;
use gpui::{
    App, AsyncApp, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, StyledExt, h_flex, scroll::ScrollableElement, v_flex};
use rust_i18n::t;
use std::collections::HashSet;

use crate::compare::schema_compare_window::SchemaCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_picker,
};
use crate::compare::table_picker::{
    TableSelectionListState, clear_table_selection_list, refresh_table_selection_list_app,
};
use crate::compare::target_picker::{
    CompareTargetCascadeAction, TargetConnectionControls, TargetStringControls,
    clear_string_select, database_change_cascade_actions, initial_compare_target_cascade_actions,
    load_databases_then, load_schemas_then, schema_change_cascade_actions, selected_string,
};
use crate::compare::window_ui::{
    compare_progress_view, register_connection_for_compare, section_title, selected_connection_id,
    stat_cards_row,
};
use crate::db_object_selector::{
    DbObjectSelectorControls, db_object_selector_panel, effective_database_schema,
    policy_for_connection,
};

impl SchemaCompareWindow {
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
                // Schema compare only selects table names on the source side.
                // A target-side table cascade is therefore intentionally a no-op.
                CompareTargetCascadeAction::LoadTables => {}
            }
        }
    }

    pub(super) fn load_target_schemas(&mut self, cx: &mut Context<Self>) {
        load_schemas_then(
            self.connection_controls(),
            self.database_controls(),
            self.schema_controls(),
            self.status.clone(),
            cx,
            |this, cx| this.load_target_after_schema_change(cx),
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
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(self.render_type_mapping_overrides(cx)),
            )
    }

    pub(super) fn render_result_meta(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self
            .result
            .read(cx)
            .as_ref()
            .map(|r| (r.added_count, r.removed_count, r.modified_count));
        let failed_tables = self
            .result
            .read(cx)
            .as_ref()
            .filter(|result| result.has_failed_tables())
            .map(|result| {
                t!(
                    "Compare.schema_compare_table_failed_note",
                    count = result.table_failures.len()
                )
                .to_string()
            });
        let progress = self.progress.read(cx).clone();
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
            .when_some(stats, |this, (added, removed, modified)| {
                this.child(stat_cards_row(added, removed, modified, cx))
            })
            .when_some(self.result.read(cx).as_ref(), |this, result| {
                this.child(schema_diff_panel(result, cx))
            })
            .when_some(failed_tables, |this, note| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when_some(sync_summary, |this, sync_summary| {
                this.child(div().text_sm().child(sync_summary))
            })
    }

    pub(super) fn render_sync_statement_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let plan = self.sync_plan.read(cx).clone();

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
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

    pub(super) fn source_controls(&self, cx: &Context<Self>) -> DbObjectSelectorControls {
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
                        .filter(|table| table.object_type == TableObjectType::Table)
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

pub(crate) fn schema_diff_panel(result: &SchemaCompareResult, cx: &App) -> impl IntoElement {
    let diff_count = result.total_diff_count();
    let has_diffs = diff_count > 0;

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("Compare.schema_diff_details").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            t!("Compare.schema_diff_object_count", count = diff_count).to_string(),
                        ),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("Compare.schema_diff_direction").to_string()),
        )
        .child(
            div()
                .id("schema-compare-diff-scroll")
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .bg(cx.theme().background)
                .overflow_y_scroll()
                .when(!has_diffs, |this| {
                    this.child(
                        div()
                            .p_3()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Compare.schema_diff_no_changes").to_string()),
                    )
                })
                .when(has_diffs, |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .p_2()
                            .gap_2()
                            .children(
                                result
                                    .table_diffs
                                    .iter()
                                    .map(|diff| schema_table_diff_block(diff, cx)),
                            )
                            .when(!result.routine_diffs.is_empty(), |this| {
                                this.child(schema_routine_diff_section(&result.routine_diffs, cx))
                            })
                            .when(!result.trigger_diffs.is_empty(), |this| {
                                this.child(schema_trigger_diff_section(&result.trigger_diffs, cx))
                            }),
                    )
                }),
        )
}

fn schema_routine_diff_section(diffs: &[RoutineDiff], cx: &App) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(schema_diff_group_title(
            t!("Compare.object_routines").to_string(),
            cx,
        ))
        .children(diffs.iter().map(|diff| {
            let details = schema_child_details(
                diff.status,
                &diff.changes,
                diff.source.as_ref().map(routine_definition),
                diff.target.as_ref().map(routine_definition),
            );
            schema_child_diff_row(routine_display_name(diff), diff.status, details, cx)
        }))
}

fn schema_trigger_diff_section(diffs: &[TriggerDiff], cx: &App) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(schema_diff_group_title(
            t!("Compare.object_triggers").to_string(),
            cx,
        ))
        .children(diffs.iter().map(|diff| {
            let details = schema_child_details(
                diff.status,
                &diff.changes,
                diff.source.as_ref().map(trigger_definition),
                diff.target.as_ref().map(trigger_definition),
            );
            schema_child_diff_row(trigger_display_name(diff), diff.status, details, cx)
        }))
}

fn schema_table_diff_block(diff: &TableDiff, cx: &App) -> impl IntoElement {
    let object_type = match diff.object_type {
        SchemaObjectType::Table => t!("Compare.object_table").to_string(),
        SchemaObjectType::View => t!("Compare.object_view").to_string(),
    };
    let table_changes = table_level_changes(diff);

    v_flex()
        .w_full()
        .gap_1()
        .p_2()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(schema_diff_status_marker(diff.status, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_semibold()
                        .child(diff.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(object_type),
                ),
        )
        .when(!table_changes.is_empty(), |this| {
            this.child(
                v_flex()
                    .pl_6()
                    .gap_1()
                    .children(table_changes.into_iter().map(|change| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(change)
                    })),
            )
        })
        .when(!diff.column_diffs.is_empty(), |this| {
            this.child(
                v_flex()
                    .pl_4()
                    .gap_1()
                    .child(schema_diff_group_title(
                        t!("Compare.object_columns").to_string(),
                        cx,
                    ))
                    .children(diff.column_diffs.iter().map(|column| {
                        let details = schema_child_details(
                            column.status,
                            &column.changes,
                            column.source.as_ref().map(column_definition),
                            column.target.as_ref().map(column_definition),
                        );
                        schema_child_diff_row(column.name.clone(), column.status, details, cx)
                    })),
            )
        })
        .when(!diff.index_diffs.is_empty(), |this| {
            this.child(
                v_flex()
                    .pl_4()
                    .gap_1()
                    .child(schema_diff_group_title(
                        t!("Compare.object_indexes").to_string(),
                        cx,
                    ))
                    .children(diff.index_diffs.iter().map(|index| {
                        let details = schema_child_details(
                            index.status,
                            &index.changes,
                            index.source.as_ref().map(index_definition),
                            index.target.as_ref().map(index_definition),
                        );
                        schema_child_diff_row(index.name.clone(), index.status, details, cx)
                    })),
            )
        })
        .when(!diff.foreign_key_diffs.is_empty(), |this| {
            this.child(
                v_flex()
                    .pl_4()
                    .gap_1()
                    .child(schema_diff_group_title(
                        t!("Compare.object_foreign_keys").to_string(),
                        cx,
                    ))
                    .children(diff.foreign_key_diffs.iter().map(|foreign_key| {
                        let details = schema_child_details(
                            foreign_key.status,
                            &foreign_key.changes,
                            foreign_key.source.as_ref().map(foreign_key_definition),
                            foreign_key.target.as_ref().map(foreign_key_definition),
                        );
                        schema_child_diff_row(
                            foreign_key.name.clone(),
                            foreign_key.status,
                            details,
                            cx,
                        )
                    })),
            )
        })
}

fn table_level_changes(diff: &TableDiff) -> Vec<String> {
    let child_changes = diff
        .column_diffs
        .iter()
        .flat_map(|child| {
            child
                .changes
                .iter()
                .map(move |change| format!("column `{}`: {change}", child.name))
        })
        .chain(diff.index_diffs.iter().flat_map(|child| {
            child
                .changes
                .iter()
                .map(move |change| format!("index `{}`: {change}", child.name))
        }))
        .chain(diff.foreign_key_diffs.iter().flat_map(|child| {
            child
                .changes
                .iter()
                .map(move |change| format!("foreign key `{}`: {change}", child.name))
        }))
        .collect::<HashSet<_>>();

    diff.changes
        .iter()
        .filter(|change| !child_changes.contains(*change))
        .cloned()
        .collect()
}

fn schema_child_details(
    status: DiffStatus,
    changes: &[String],
    source_definition: Option<String>,
    target_definition: Option<String>,
) -> Vec<String> {
    match status {
        DiffStatus::Added => source_definition.into_iter().collect(),
        DiffStatus::Removed => target_definition.into_iter().collect(),
        DiffStatus::Modified => changes.to_vec(),
    }
}

fn column_definition(column: &ColumnSchema) -> String {
    let mut details = vec![
        format!("type: {}", column.data_type),
        if column.nullable {
            "nullable".to_string()
        } else {
            "not null".to_string()
        },
    ];
    push_optional_detail(&mut details, "default", column.default_value.as_deref());
    push_optional_detail(&mut details, "charset", column.charset.as_deref());
    push_optional_detail(&mut details, "collation", column.collation.as_deref());
    push_optional_detail(&mut details, "comment", column.comment.as_deref());
    details.join(" · ")
}

fn index_definition(index: &IndexSchema) -> String {
    format!(
        "columns: [{}] · {}",
        index.columns.join(", "),
        if index.unique { "unique" } else { "non-unique" }
    )
}

fn foreign_key_definition(foreign_key: &ForeignKeySchema) -> String {
    let mut details = vec![format!(
        "columns: [{}] → {}({})",
        foreign_key.columns.join(", "),
        foreign_key.ref_table,
        foreign_key.ref_columns.join(", ")
    )];
    push_optional_detail(&mut details, "on delete", foreign_key.on_delete.as_deref());
    push_optional_detail(&mut details, "on update", foreign_key.on_update.as_deref());
    details.join(" · ")
}

fn routine_display_name(diff: &RoutineDiff) -> String {
    let routine = diff.source.as_ref().or(diff.target.as_ref());
    let kind = match diff.kind {
        RoutineKind::Function => "function",
        RoutineKind::Procedure => "procedure",
    };
    let name = routine
        .and_then(|routine| routine.schema.as_deref())
        .filter(|schema| !schema.trim().is_empty())
        .map(|schema| format!("{}.{}", schema.trim(), diff.name))
        .unwrap_or_else(|| diff.name.clone());
    let arguments = routine
        .and_then(|routine| routine.identity_arguments.as_deref())
        .filter(|arguments| !arguments.trim().is_empty())
        .map(str::trim)
        .map(str::to_string)
        .or_else(|| {
            routine
                .filter(|routine| !routine.parameters.is_empty())
                .map(|routine| routine.parameters.join(", "))
        })
        .unwrap_or_default();

    format!("{kind} {name}({arguments})")
}

fn trigger_display_name(diff: &TriggerDiff) -> String {
    let trigger = diff.source.as_ref().or(diff.target.as_ref());
    trigger
        .map(|trigger| {
            let table = trigger
                .schema
                .as_deref()
                .filter(|schema| !schema.trim().is_empty())
                .map(|schema| format!("{}.{}", schema.trim(), trigger.table_name))
                .unwrap_or_else(|| trigger.table_name.clone());
            format!("trigger {table}.{}", diff.name)
        })
        .unwrap_or_else(|| format!("trigger {}", diff.name))
}

fn routine_definition(routine: &RoutineSchema) -> String {
    let mut details = Vec::new();
    push_optional_detail(&mut details, "returns", routine.return_type.as_deref());
    if !routine.parameters.is_empty() {
        details.push(format!("parameters: [{}]", routine.parameters.join(", ")));
    }
    if let Some(definition) = compact_definition(routine.definition.as_deref()) {
        details.push(format!("definition: {definition}"));
    }
    push_optional_detail(&mut details, "comment", routine.comment.as_deref());

    details.join(" · ")
}

fn trigger_definition(trigger: &TriggerSchema) -> String {
    let mut details = vec![
        format!("timing: {}", trigger.timing),
        format!("event: {}", trigger.event),
    ];
    if let Some(definition) = compact_definition(trigger.definition.as_deref()) {
        details.push(format!("definition: {definition}"));
    }

    details.join(" · ")
}

fn compact_definition(definition: Option<&str>) -> Option<String> {
    const MAX_CHARS: usize = 240;

    let definition = definition?.split_whitespace().collect::<Vec<_>>().join(" ");
    if definition.is_empty() {
        return None;
    }

    let mut chars = definition.chars();
    let compact = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        Some(format!("{compact}…"))
    } else {
        Some(compact)
    }
}

fn push_optional_detail(details: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        details.push(format!("{label}: {}", value.trim()));
    }
}

fn schema_diff_group_title(title: String, cx: &App) -> impl IntoElement {
    div()
        .pt_1()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

fn schema_child_diff_row(
    name: String,
    status: DiffStatus,
    changes: Vec<String>,
    cx: &App,
) -> impl IntoElement {
    let details = if changes.is_empty() {
        String::new()
    } else {
        changes.join(" · ")
    };

    v_flex()
        .pl_2()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(schema_diff_status_marker(status, cx))
                .child(div().text_sm().child(name)),
        )
        .when(!details.is_empty(), |this| {
            this.child(
                div()
                    .pl_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(details),
            )
        })
}

fn schema_diff_status_marker(status: DiffStatus, cx: &App) -> impl IntoElement {
    let (symbol, label, color) = match status {
        DiffStatus::Added => ("+", t!("Compare.added").to_string(), cx.theme().success),
        DiffStatus::Removed => ("−", t!("Compare.removed").to_string(), cx.theme().danger),
        DiffStatus::Modified => ("~", t!("Compare.modified").to_string(), cx.theme().warning),
    };

    h_flex()
        .gap_1()
        .child(
            div()
                .w(px(14.0))
                .text_sm()
                .font_semibold()
                .text_color(color)
                .child(symbol),
        )
        .child(div().text_xs().text_color(color).child(label))
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

#[cfg(test)]
mod tests {
    use super::{
        column_definition, compact_definition, routine_display_name, schema_child_details,
        table_level_changes, trigger_display_name,
    };
    use db::compare::{
        ColumnDiff, ColumnSchema, DiffStatus, RoutineDiff, RoutineKind, RoutineSchema,
        SchemaObjectType, TableDiff, TriggerDiff, TriggerSchema,
    };

    #[test]
    fn added_column_details_include_the_source_definition() {
        let column = ColumnSchema {
            name: "name".to_string(),
            data_type: "varchar(64)".to_string(),
            nullable: false,
            default_value: Some("'guest'".to_string()),
            comment: Some("display name".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_bin".to_string()),
        };

        assert_eq!(
            schema_child_details(
                DiffStatus::Added,
                &["column added".to_string()],
                Some(column_definition(&column)),
                None,
            ),
            vec![
                "type: varchar(64) · not null · default: 'guest' · charset: utf8mb4 · collation: utf8mb4_bin · comment: display name"
                    .to_string()
            ]
        );
    }

    #[test]
    fn table_level_changes_only_remove_exact_child_change_messages() {
        let diff = TableDiff {
            name: "users".to_string(),
            status: DiffStatus::Modified,
            object_type: SchemaObjectType::Table,
            changes: vec![
                "column `name`: type: text → varchar(64)".to_string(),
                "column `legacy text remains table-level`".to_string(),
                "engine: MyISAM → InnoDB".to_string(),
            ],
            source: None,
            target: None,
            column_diffs: vec![ColumnDiff {
                name: "name".to_string(),
                status: DiffStatus::Modified,
                changes: vec!["type: text → varchar(64)".to_string()],
                source: None,
                target: None,
            }],
            index_diffs: Vec::new(),
            foreign_key_diffs: Vec::new(),
            comment_changed: false,
            table_options_changed: true,
        };

        assert_eq!(
            table_level_changes(&diff),
            vec![
                "column `legacy text remains table-level`".to_string(),
                "engine: MyISAM → InnoDB".to_string(),
            ]
        );
    }

    #[test]
    fn programmable_object_display_names_include_identity_context() {
        let routine = RoutineSchema {
            kind: RoutineKind::Function,
            name: "calculate_total".to_string(),
            schema: Some("public".to_string()),
            identity_arguments: Some("integer, numeric".to_string()),
            ..Default::default()
        };
        let routine_diff = RoutineDiff {
            name: routine.name.clone(),
            kind: routine.kind,
            status: DiffStatus::Added,
            changes: Vec::new(),
            source: Some(routine),
            target: None,
        };
        let trigger = TriggerSchema {
            name: "audit_orders".to_string(),
            schema: Some("public".to_string()),
            table_name: "orders".to_string(),
            event: "INSERT".to_string(),
            timing: "AFTER".to_string(),
            definition: None,
        };
        let trigger_diff = TriggerDiff {
            name: trigger.name.clone(),
            status: DiffStatus::Added,
            changes: Vec::new(),
            source: Some(trigger),
            target: None,
        };

        assert_eq!(
            routine_display_name(&routine_diff),
            "function public.calculate_total(integer, numeric)"
        );
        assert_eq!(
            trigger_display_name(&trigger_diff),
            "trigger public.orders.audit_orders"
        );
    }

    #[test]
    fn programmable_definitions_are_compacted_and_bounded() {
        assert_eq!(
            compact_definition(Some("BEGIN\n  RETURN 1;\nEND")),
            Some("BEGIN RETURN 1; END".to_string())
        );

        let compact = compact_definition(Some(&"x".repeat(300))).unwrap();
        assert_eq!(compact.chars().count(), 241);
        assert!(compact.ends_with('…'));
    }
}
