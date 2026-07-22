#[test]
fn connection_tree_width_is_mouse_resizable_with_bounds() {
    let sidebar = include_str!("mod.rs");
    let tree = include_str!("tree.rs");
    let resize = include_str!("resize.rs");

    assert!(sidebar.contains("tree_width: Pixels"));
    assert!(tree.contains(".w(self.tree_width)"));
    assert!(tree.contains("render_tree_resize_handle"));
    assert!(resize.contains(".cursor_col_resize()"));
    assert!(resize.contains(".on_drag(ResizePanel"));
    assert!(resize.contains(".on_drag_move("));
    assert!(resize.contains("CONNECTION_TREE_MIN_WIDTH"));
    assert!(resize.contains("CONNECTION_TREE_MAX_WIDTH"));
    assert!(resize.contains(".clamp("));
}
