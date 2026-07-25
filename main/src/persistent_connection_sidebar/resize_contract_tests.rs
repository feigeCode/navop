use gpui::px;

use super::resize::{
    CONNECTION_TREE_MAX_WIDTH, CONNECTION_TREE_MIN_WIDTH, resized_connection_tree_width,
};

#[test]
fn connection_tree_width_is_mouse_resizable_with_bounds() {
    let sidebar = include_str!("mod.rs");
    let tree = include_str!("tree.rs");
    let resize = include_str!("resize.rs");

    assert!(sidebar.contains("tree_width: Pixels"));
    assert!(tree.contains(".w(self.tree_width)"));
    assert!(tree.contains("render_tree_resize_handle"));
    assert!(resize.contains(".cursor_col_resize()"));
    assert!(resize.contains(".on_drag("));
    assert!(resize.contains("ConnectionTreeResize {"));
    assert!(resize.contains(".on_drag_move("));
    assert!(resize.contains("initial_width: self.tree_width"));
    assert!(resize.contains("initial_x"));
    assert!(!resize.contains("event.bounds.center().x"));
    assert!(resize.contains("CONNECTION_TREE_MIN_WIDTH"));
    assert!(resize.contains("CONNECTION_TREE_MAX_WIDTH"));
    assert!(resize.contains(".clamp("));
}

#[test]
fn connection_tree_resize_is_relative_to_the_drag_start_without_accumulation() {
    let initial_width = px(260.0);
    let initial_x = px(260.0);

    assert_eq!(
        resized_connection_tree_width(initial_width, initial_x, px(300.0)),
        px(300.0)
    );
    assert_eq!(
        resized_connection_tree_width(initial_width, initial_x, px(320.0)),
        px(320.0)
    );
}

#[test]
fn connection_tree_resize_clamps_to_supported_bounds() {
    assert_eq!(
        resized_connection_tree_width(px(260.0), px(260.0), px(-500.0)),
        CONNECTION_TREE_MIN_WIDTH
    );
    assert_eq!(
        resized_connection_tree_width(px(260.0), px(260.0), px(900.0)),
        CONNECTION_TREE_MAX_WIDTH
    );
}
