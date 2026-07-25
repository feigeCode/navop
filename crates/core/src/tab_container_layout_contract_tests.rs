#[test]
fn scrollable_tabs_keep_window_controls_at_the_right_edge() {
    let source = include_str!("tab_container.rs");
    let tabs_start = source.find(".id(\"tabs\")").expect("scrollable tabs");
    let controls_start = source[tabs_start..]
        .find("self.render_window_controls(window, cx)")
        .map(|offset| tabs_start + offset)
        .expect("window controls");
    let tabs = &source[tabs_start..controls_start];

    assert!(tabs.contains(".size_full()"));
    assert!(tabs.contains(".overflow_x_scroll()"));
    assert!(tabs.contains(".map(|tabs|"));
    assert!(tabs.contains(".id(\"tab-scroll-boundary\")"));
    assert!(tabs.contains(".flex_1()"));
    assert!(tabs.contains(".min_w_0()"));
    assert!(tabs.contains(".overflow_hidden()"));
}

#[test]
fn active_tab_intrinsic_size_cannot_shrink_the_window_chrome() {
    let source = include_str!("tab_container.rs");
    let render_start = source
        .find("impl Render for TabContainer")
        .expect("tab container renderer");
    let render = &source[render_start..];
    let root_end = render.find(".child(").expect("tab container root child");
    let root = &render[..root_end];

    assert!(root.contains(".size_full()"));
    assert!(root.contains(".min_w_0()"));
    assert!(root.contains(".min_h_0()"));
    assert!(root.contains(".overflow_hidden()"));

    let content_start = source.find(".id(\"tab-content\")").expect("tab content");
    let content_end = source[content_start..]
        .find(".when(!has_sidebar_layout")
        .map(|offset| content_start + offset)
        .expect("tab content body");
    let content = &source[content_start..content_end];

    assert!(content.contains(".flex_1()"));
    assert!(content.contains(".w_full()"));
    assert!(content.contains(".min_w_0()"));
    assert!(content.contains(".min_h_0()"));
    assert!(content.contains(".overflow_hidden()"));
}

#[test]
fn window_controls_follow_the_active_theme_for_contrast() {
    let source = include_str!("tab_container.rs");
    let controls_start = source
        .find("fn render_window_controls")
        .expect("window controls renderer");
    let controls_end = source[controls_start..]
        .find("/// 渲染窗口置顶按钮")
        .map(|offset| controls_start + offset)
        .expect("always-on-top renderer");
    let controls = &source[controls_start..controls_end];

    assert!(controls.contains("let foreground = cx.theme().foreground;"));
    assert!(controls.contains("cx.theme().secondary_hover"));
    assert!(controls.contains("cx.theme().secondary_active"));
    assert!(controls.contains("cx.theme().danger"));
    assert!(!controls.contains(".text_color(gpui::white())"));

    let always_on_top = &source[controls_end..];
    assert!(always_on_top.contains("let icon_color: gpui::Hsla"));
    assert!(always_on_top.contains("cx.theme().foreground"));
    assert!(always_on_top.contains("cx.theme().secondary_hover"));
    assert!(always_on_top.contains("cx.theme().secondary_active"));
    assert!(!always_on_top.contains("gpui::rgb(0xffffff)"));
    assert!(!always_on_top.contains("gpui::rgb(0x2a2a2a)"));
}

#[test]
fn sidebar_resize_uses_tab_container_bounds_instead_of_window_bounds() {
    let source = include_str!("tab_container.rs");
    let renderer_start = source
        .find("fn render_content_with_sidebars")
        .expect("sidebar content renderer");
    let renderer_end = source[renderer_start..]
        .find("pub fn render_tab_content")
        .map(|offset| renderer_start + offset)
        .expect("tab content renderer");
    let renderer = &source[renderer_start..renderer_end];

    assert!(renderer.contains(".id(\"tab-sidebar-root\")"));
    assert!(renderer.contains(".on_prepaint({"));
    assert!(renderer.contains("container.sidebar_bounds = bounds;"));

    let handler_start = source
        .find("impl Element for SidebarResizeEventHandler")
        .expect("sidebar resize event handler");
    let handler = &source[handler_start..];
    assert!(!handler.contains("let bounds = window.bounds();"));
}
