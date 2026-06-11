use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::{ActiveTheme, button::Button, h_flex, v_flex};

use crate::compare::schema_compare_window::SchemaCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, selected_sync_sql_text_for_ids, sync_statement_picker,
};
use crate::compare::target_picker::{
    TargetConnectionControls, TargetStringControls, load_databases, load_schemas, string_select_row,
};
use crate::compare::window_ui::{
    connection_select_row, schema_summary, section_title, sql_preview,
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

    pub(super) fn render_target(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(section_title("Target"))
            .child(connection_select_row(
                "Connection",
                &self.target_connection_select,
            ))
            .child(string_select_row("Database", &self.target_database_select))
            .child(string_select_row("Schema", &self.target_schema_select))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("load-target-databases")
                            .child("Load DBs")
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.load_target_databases(cx)),
                            ),
                    )
                    .child(
                        Button::new("load-target-schemas")
                            .child("Load Schemas")
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.load_target_schemas(cx)),
                            ),
                    ),
            )
    }

    pub(super) fn render_result(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.read(cx).clone();
        let summary = self.result.read(cx).as_ref().map(schema_summary);
        let plan = self.sync_plan.read(cx);
        let selected_ids = self.selected_statement_ids.read(cx).clone();
        let sql = plan.as_ref().map_or_else(String::new, |plan| {
            selected_sync_sql_text_for_ids(plan, &selected_ids)
        });
        let sync_summary = plan
            .as_ref()
            .map(|plan| selected_sync_sql_summary_for_ids(plan, &selected_ids));

        v_flex()
            .gap_2()
            .child(section_title("Result"))
            .child(div().text_sm().child(status))
            .when_some(summary, |this, summary| {
                this.child(div().text_sm().child(summary))
            })
            .when_some(sync_summary, |this, sync_summary| {
                this.child(div().text_sm().child(sync_summary))
            })
            .when_some(plan.clone(), |this, plan| {
                this.child(sync_statement_picker(
                    plan,
                    self.selected_statement_ids.clone(),
                    selected_ids,
                ))
            })
            .when(!sql.is_empty(), |this| {
                this.child(sql_preview(
                    "schema-compare-copy-sql",
                    sql,
                    cx.theme().border,
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
