#[test]
fn scrollable_tabs_keep_window_controls_at_the_right_edge() {
    let source = include_str!("tab_container.rs");
    let tabs_start = source.find(".id(\"tabs\")").expect("scrollable tabs");
    let controls_start = source[tabs_start..]
        .find("self.render_window_controls(window)")
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
