#[test]
fn macos_keeps_tab_drag_enabled_while_blank_space_moves_the_window() {
    let source = include_str!("tab_container.rs");
    let macos_disable = ["let allow_tab_drag = ", "!is_macos"].concat();

    assert!(!source.contains(&macos_disable));
    assert!(source.contains("window.start_window_move()"));
    assert!(source.contains(".on_drag("));
}
