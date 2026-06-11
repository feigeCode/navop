use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::{ActiveTheme, button::Button, h_flex, v_flex};

use crate::compare::data_compare_window::DataCompareWindow;
use crate::compare::sync_statement_picker::{
    selected_sync_sql_summary_for_ids, selected_sync_sql_text_for_ids, sync_statement_picker,
};
use crate::compare::target_picker::{
    TargetConnectionControls, TargetStringControls, load_databases, load_schemas, load_tables,
    string_select_row,
};
use crate::compare::window_ui::{
    connection_select_row, data_summary, detail_row, input_row, section_title, sql_preview,
};

impl DataCompareWindow {
    pub(super) fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(section_title("Source"))
            .child(detail_row(
                "Connection",
                self.source_node.connection_id.clone(),
            ))
            .child(detail_row(
                "Database",
                self.source_node.get_database_name().unwrap_or_default(),
            ))
            .child(detail_row(
                "Schema",
                self.source_node.get_schema_name().unwrap_or_default(),
            ))
            .child(detail_row("Table", self.source_node.name.clone()))
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
            .child(string_select_row("Table", &self.target_table_select))
            .child(input_row("Key columns", &self.key_columns))
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
                    )
                    .child(
                        Button::new("load-target-tables")
                            .child("Load Tables")
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.load_target_tables(cx)),
                            ),
                    ),
            )
    }

    pub(super) fn render_result(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.read(cx).clone();
        let summary = self.result.read(cx).as_ref().map(data_summary);
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
                this.child(sql_preview("data-compare-copy-sql", sql, cx.theme().border))
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
