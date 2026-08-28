use db::sql_editor::execution::{SqlExecutionResultSource, sql_fingerprint};
use db::sql_editor::statement_ranges::SqlTextRange;

use crate::sql_result_tab::SqlResultTab;

#[test]
fn result_tab_preserves_execution_source_identity() {
    let source = SqlExecutionResultSource {
        request_id: 3,
        document_revision: 7,
        source_range: Some(SqlTextRange {
            start_byte: 10,
            end_byte: 18,
        }),
        sql_fingerprint: sql_fingerprint("select 2"),
        statement_index: Some(1),
    };
    let tab = SqlResultTab {
        sql: "select 2".to_string(),
        result: db::SqlResult::Exec(db::ExecResult {
            sql: "select 2".to_string(),
            rows_affected: 0,
            elapsed_ms: 0,
            message: None,
        }),
        execution_time: "0ms".to_string(),
        rows_count: "0 rows".to_string(),
        data_grid: None,
        content: None,
        source,
    };

    assert_eq!(source, tab.source);
}

#[test]
fn statement_summary_has_a_clipped_scroll_boundary_and_visible_scrollbar() {
    let source = include_str!("sql_result_tab.rs");

    assert!(source.contains(".id(\"statement-list-viewport\")"));
    assert!(source.contains(".min_h_0()"));
    assert!(source.contains(".overflow_hidden()"));
    assert!(source.contains("Scrollbar::vertical(&self.scroll_handle)"));
}

#[test]
fn result_tabs_keep_content_shrinkable_inside_the_panel() {
    let source = include_str!("sql_result_tab.rs");

    assert!(source.contains(".id(\"sql-result-content\")"));
    assert!(source.contains(".min_w_0()"));
    assert!(source.contains(".min_h_0()"));
}

#[test]
fn result_content_wrapper_provides_a_full_height_vertical_flex_context() {
    let source = include_str!("sql_result_tab.rs");
    let wrapper = source
        .split(".id(\"sql-result-content\")")
        .nth(1)
        .expect("result content wrapper should exist");

    assert!(wrapper.contains(".flex_col()"));
    assert!(wrapper.contains(".size_full()"));
}

#[test]
fn statement_scrollbar_has_an_explicit_overlay_boundary() {
    let source = include_str!("sql_result_tab.rs");
    let scrollbar = source
        .split(".id(\"statement-list-container\")")
        .nth(1)
        .and_then(|source| source.split(".into_any_element()").next())
        .expect("statement scrollbar boundary should exist");

    assert!(scrollbar.contains("Scrollbar::vertical(&self.scroll_handle)"));
    assert!(scrollbar.contains(".absolute()"));
    assert!(scrollbar.contains(".top_0()"));
    assert!(scrollbar.contains(".right_0()"));
    assert!(scrollbar.contains(".bottom_0()"));
}
