#[test]
fn macos_keeps_tab_drag_enabled_while_blank_space_moves_the_window() {
    let source = include_str!("tab_container.rs");
    let macos_disable = ["let allow_tab_drag = ", "!is_macos"].concat();
    let tabs_start = source.find(".id(\"tabs\")").expect("tabs container");
    let tabs_scroll = source[tabs_start..]
        .find(".overflow_x_scroll()")
        .map(|offset| tabs_start + offset)
        .expect("scrollable tabs container");
    let tabs_boundary = source[tabs_start..]
        .find(".id(\"tab-scroll-boundary\")")
        .map(|offset| tabs_start + offset)
        .expect("tabs boundary");
    let tabs_block = &source[tabs_start..tabs_boundary];

    assert!(!source.contains(&macos_disable));
    assert!(source.contains(".id(\"tab-bar-window-drag-left\")"));
    assert!(source.contains(".id(\"tab-bar-window-drag-right\")"));
    assert!(source.contains("window.start_window_move()"));
    assert!(!tabs_block.contains("window_control_area(WindowControlArea::Drag)"));
    assert!(!tabs_block.contains(".child(right_window_drag_region)"));
    assert!(tabs_scroll < tabs_boundary);
    assert!(source[tabs_boundary..].contains(".child(right_window_drag_region)"));
    assert!(source.contains(".on_drag("));
    assert!(source.contains("DragTab::new("));
}
