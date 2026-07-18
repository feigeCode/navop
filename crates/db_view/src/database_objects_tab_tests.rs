use std::collections::HashSet;

use crate::database_objects_tab::{apply_object_context_menu_target, object_name_highlight_ranges};

#[test]
fn right_click_targets_the_row_and_replaces_single_selection() {
    let mut selected = HashSet::from([1, 3]);
    let mut context_menu_row = Some(1);

    apply_object_context_menu_target(&mut selected, &mut context_menu_row, 5);

    assert_eq!(HashSet::from([5]), selected);
    assert_eq!(Some(5), context_menu_row);
}

#[test]
fn object_rows_reuse_the_database_tree_context_menu_model() {
    let source = include_str!("database_objects_tab.rs");

    assert!(source.contains("MouseButton::Right"));
    assert!(source.contains(".context_menu("));
    assert!(source.contains("build_context_menu_for("));
    assert!(source.contains("DbTreeExtensionMenuRegistry"));
    assert!(source.contains("DatabaseObjectsEvent::TreeEvent"));
}

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
