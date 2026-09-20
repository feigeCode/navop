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

#[test]
fn object_rows_wire_keyboard_and_mouse_range_selection() {
    let source = include_str!("database_objects_tab.rs");

    // Ctrl/Cmd+A 走 key context action，保证列表聚焦时即可全选可见行
    assert!(source.contains(".key_context(DB_SEARCH_CONTEXT)"));
    assert!(source.contains(".on_action(cx.listener(Self::on_action_select_all_objects))"));
    assert!(source.contains("select_all_rows(&mut self.selected_indices"));

    // Shift 点击扩到区间，拖选走 mouse_move/mouse_up
    assert!(source.contains("event.modifiers.shift"));
    assert!(source.contains("apply_row_range(&mut self.selected_indices"));
    assert!(source.contains("apply_drag_selection("));
    assert!(source.contains("exceeds_drag_threshold("));
    assert!(source.contains(".on_mouse_move(cx.listener("));
    // 行内释放走 on_row_mouse_up，面板根节点兜住“拖到行外松手”
    assert!(
        source.contains(".on_mouse_up(MouseButton::Left, cx.listener(Self::on_panel_mouse_up))")
    );
    assert!(source.contains("fn on_panel_mouse_up("));
    assert!(source.contains("fn end_row_drag("));
}

#[test]
fn select_all_asks_host_to_load_rows_that_have_no_open_tab() {
    let source = include_str!("database_objects_tab.rs");

    assert!(source.contains("DatabaseObjectsEvent::SelectAllInScope"));
    assert!(source.contains("fn selection_scope(&self)"));
    assert!(source.contains("fn select_rows_matching_nodes("));
}
