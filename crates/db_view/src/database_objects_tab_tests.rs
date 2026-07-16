use crate::database_objects_tab::object_name_highlight_ranges;

#[test]
fn object_name_cells_avoid_label_line_height_clipping() {
    let source = include_str!("database_objects_tab.rs");

    assert!(source.contains("render_object_name_text(cell_value"));
    assert!(!source.contains("Label::new(cell_value)"));
}

#[test]
fn object_name_highlights_preserve_identifier_boundaries() {
    let text = "infra_api_access_log";

    assert_eq!(vec![6..9], object_name_highlight_ranges(text, "API"));
    assert_eq!(
        Vec::<std::ops::Range<usize>>::new(),
        object_name_highlight_ranges(text, "")
    );
}
