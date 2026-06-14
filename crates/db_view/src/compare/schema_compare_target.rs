use extension_component::DbSelectorKind;
use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::v_flex;
use rust_i18n::t;

use crate::compare::schema_compare_window::SchemaCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_picker,
};
use crate::compare::target_picker::{
    TargetConnectionControls, TargetStringControls, clear_string_select, load_databases,
    load_schemas,
};
use crate::compare::window_ui::{compare_progress_view, section_title, stat_cards_row};
use crate::db_object_selector::{
    DbObjectSelectorControls, db_object_selector_panel, policy_for_connection,
};

impl SchemaCompareWindow {
    pub(super) fn load_source_databases(&mut self, cx: &mut Context<Self>) {
        clear_string_select(&self.source_schema_select, cx);
        load_databases(
            self.source_connection_controls(),
            self.source_database_controls(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn load_source_schemas(&mut self, cx: &mut Context<Self>) {
        load_schemas(
            self.source_connection_controls(),
            self.source_database_controls(),
            self.source_schema_controls(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn load_target_databases(&mut self, cx: &mut Context<Self>) {
        clear_string_select(&self.target_schema_select, cx);
        load_databases(
            self.connection_controls(),
            self.database_controls(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn load_target_schemas(&mut self, cx: &mut Context<Self>) {
        load_schemas(
            self.connection_controls(),
            self.database_controls(),
            self.schema_controls(),
            self.status.clone(),
            cx,
        );
    }

    pub(super) fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        db_object_selector_panel(
            t!("Compare.target").to_string(),
            DbSelectorKind::Schema,
            self.target_controls(cx),
            cx,
        )
    }

    pub(super) fn render_result_meta(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self
            .result
            .read(cx)
            .as_ref()
            .map(|r| (r.added_count, r.removed_count, r.modified_count));
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
}
