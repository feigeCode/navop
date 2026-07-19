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
