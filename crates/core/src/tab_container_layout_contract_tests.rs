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
