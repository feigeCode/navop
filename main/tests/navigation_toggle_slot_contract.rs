#[test]
fn hidden_navigation_toggle_keeps_its_layout_slot() {
    let source = include_str!("../../crates/core/src/tab_container.rs");
    assert!(source.contains("reserve_navigation_sidebar_toggle: false"));
    let builder = source
        .split("pub fn with_navigation_sidebar_toggle(")
        .nth(1)
        .unwrap()
        .split("pub fn with_home_button(")
        .next()
        .unwrap();
    assert!(builder.contains("self.reserve_navigation_sidebar_toggle = true"));
    assert!(source.contains(
        "self.reserve_navigation_sidebar_toggle || navigation_sidebar_expanded.is_some()"
    ));
    let boundary = source
        .split(".id(\"navigation-sidebar-toggle-boundary\")")
        .nth(1)
        .unwrap()
        .split(".when_some(on_home")
        .next()
        .unwrap();
    assert!(boundary.contains(".flex_shrink_0()"));
    assert!(
        boundary.contains(".when(navigation_sidebar_expanded.is_none(), |this| this.invisible())")
    );
    assert!(boundary.contains("Button::new(\"navigation-sidebar-toggle\")"));
}
