use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::{ActiveTheme, v_flex};

use crate::compare::data_compare_window::DataCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, sync_statement_picker,
};
use crate::compare::target_picker::{
    TargetConnectionControls, TargetStringControls, load_databases, load_schemas, load_tables,
    string_select_row,
};
use crate::compare::window_ui::{
    compare_progress_view, connection_select_row, data_truncation_note, detail_row, input_row,
    section_title, stat_cards_row,
};

impl DataCompareWindow {
    pub(super) fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(section_title("源"))
            .child(detail_row("连接", self.source_node.connection_id.clone()))
            .child(detail_row(
                "数据库",
                self.source_node.get_database_name().unwrap_or_default(),
            ))
            .child(detail_row(
                "Schema",
                self.source_node.get_schema_name().unwrap_or_default(),
            ))
            .child(detail_row("表", self.source_node.name.clone()))
    }

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

    pub(super) fn load_target_tables(&mut self, cx: &mut Context<Self>) {
        load_tables(
            self.connection_controls(),
            self.database_controls(),
            self.schema_controls(),
            self.table_controls(),
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
            .child(string_select_row("表", &self.target_table_select))
            .child(input_row("主键列", &self.key_columns))
    }

    pub(super) fn render_result_meta(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self
            .result
            .read(cx)
            .as_ref()
            .map(|r| (r.added.len(), r.removed.len(), r.modified.len()));
        let truncation = self.result.read(cx).as_ref().and_then(data_truncation_note);
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

    fn table_controls(&self) -> TargetStringControls {
        TargetStringControls {
            select: self.target_table_select.clone(),
            fallback: self.target_table.clone(),
        }
    }
}
