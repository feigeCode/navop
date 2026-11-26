#[test]
fn hidden_navigation_toggle_keeps_its_layout_slot() {
    let source = include_str!("../../../crates/core/src/tab_container.rs");
    assert!(source.contains("reserve_navigation_sidebar_toggle: false"));
    let builder = source
        .split("pub fn with_navigation_sidebar_toggle(")
        .nth(1)
        .unwrap()
        .split("pub fn with_home_button(")
        .next()
        .unwrap();
    assert!(builder.contains("self.reserve_navigation_sidebar_toggle = true"));
    // 开关位同时服务连接资源栏、主页应用菜单与 AI 工作台会话列表，
    // 任一状态存在都要保留这个槽位，避免开关在两处跳来跳去。
    assert!(source.contains("self.reserve_navigation_sidebar_toggle"));
    assert!(source.contains("|| navigation_sidebar_expanded.is_some()"));
    assert!(source.contains("|| home_menu_expanded.is_some()"));
    assert!(source.contains("|| workbench_nav.is_some()"));
    let boundary = source
        .split(".id(\"navigation-sidebar-toggle-boundary\")")
        .nth(1)
        .unwrap()
        .split(".when_some(on_home")
        .next()
        .unwrap();
    assert!(boundary.contains(".flex_shrink_0()"));
    // 三种状态都没有时按钮只占位不可见：宽度由 flex_shrink_0 保住，布局不跳。
    assert!(boundary.contains("navigation_sidebar_expanded.is_none()"));
    assert!(boundary.contains("home_menu_expanded.is_none()"));
    assert!(boundary.contains("workbench_nav.is_none()"));
    assert!(boundary.contains("this.invisible()"));
    assert!(boundary.contains("Button::new(\"navigation-sidebar-toggle\")"));
}
