use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::v_flex;

use crate::compare::schema_compare_window::SchemaCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_picker,
};
use crate::compare::target_picker::{
    TargetConnectionControls, TargetStringControls, load_databases, load_schemas, string_select_row,
};
use crate::compare::window_ui::{
    compare_progress_view, connection_select_row, section_title, stat_cards_row,
};

impl SchemaCompareWindow {
    pub(super) fn load_target_databases(&mut self, cx: &mut Context<Self>) {
        load_databases(
            self.target_connection_select.clone(),
            self.target_connection_id.clone(),
            self.target_database_select.clone(),
            self.target_database.clone(),
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

    pub(super) fn render_target(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(section_title("目标"))
            .child(connection_select_row(
                "连接",
                &self.target_connection_select,
            ))
            .child(string_select_row("数据库", &self.target_database_select))
            .child(string_select_row("Schema", &self.target_schema_select))
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
            .gap_2()
            .child(section_title("结果"))
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
                    selected_ids,
                ))
            })
    }

    fn connection_controls(&self) -> TargetConnectionControls {
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
}
