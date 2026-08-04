use gpui::px;
use gpui_component::ThemeGeometry;

use super::resize::resized_connection_tree_width;

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
    assert!(resize.contains("layout.context_sidebar_min"));
    assert!(resize.contains("layout.context_sidebar_max"));
    assert!(resize.contains("resize.hit_area()"));
    assert!(resize.contains("resize.visible_line"));
    assert!(resize.contains(".clamp("));
}

#[test]
fn connection_tree_resize_is_relative_to_the_drag_start_without_accumulation() {
    let initial_width = px(260.0);
    let initial_x = px(260.0);
    let layout = ThemeGeometry::default().layout;

    assert_eq!(
        resized_connection_tree_width(
            initial_width,
            initial_x,
            px(300.0),
            layout.context_sidebar_min,
            layout.context_sidebar_max,
        ),
        px(300.0)
    );
    assert_eq!(
        resized_connection_tree_width(
            initial_width,
            initial_x,
            px(320.0),
            layout.context_sidebar_min,
            layout.context_sidebar_max,
        ),
        px(320.0)
    );
}

#[test]
fn connection_tree_resize_clamps_to_supported_bounds() {
    let layout = ThemeGeometry::default().layout;
    assert_eq!(
        resized_connection_tree_width(
            layout.context_sidebar_default,
            px(260.0),
            px(-500.0),
            layout.context_sidebar_min,
            layout.context_sidebar_max,
        ),
        layout.context_sidebar_min
    );
    assert_eq!(
        resized_connection_tree_width(
            layout.context_sidebar_default,
            px(260.0),
            px(900.0),
            layout.context_sidebar_min,
            layout.context_sidebar_max,
        ),
        layout.context_sidebar_max
    );
}
