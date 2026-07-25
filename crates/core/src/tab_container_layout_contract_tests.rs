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

    let active_view_start = source
        .find("fn render_active_tab_view")
        .expect("active tab view boundary");
    let active_view_end = source[active_view_start..]
        .find("fn render_content_with_sidebars")
        .map(|offset| active_view_start + offset)
        .expect("sidebar content renderer");
    let active_view = &source[active_view_start..active_view_end];

    assert!(active_view.contains(".size_full()"));
    assert!(active_view.contains(".min_w_0()"));
    assert!(active_view.contains(".min_h_0()"));
    assert!(active_view.contains(".overflow_hidden()"));

    let tab_content_start = source
        .find("pub fn render_tab_content")
        .expect("tab content renderer");
    let tab_content_end = source[tab_content_start..]
        .find("fn tab_switcher_entries")
        .map(|offset| tab_content_start + offset)
        .expect("tab switcher entries");
    let tab_content = &source[tab_content_start..tab_content_end];

    assert_eq!(
        tab_content.matches("Self::render_active_tab_view(").count(),
        2,
        "both sidebar and non-sidebar paths must isolate the active view"
    );
    assert!(!tab_content.contains("el.child(view)"));
}

#[test]
fn sidebar_center_clips_active_view_intrinsic_size_at_every_flex_boundary() {
    let source = include_str!("tab_container.rs");
    let renderer_start = source
        .find("fn render_content_with_sidebars")
        .expect("sidebar content renderer");
    let renderer_end = source[renderer_start..]
        .find("pub fn render_tab_content")
        .map(|offset| renderer_start + offset)
        .expect("tab content renderer");
    let renderer = &source[renderer_start..renderer_end];

    let center_start = renderer
        .find("let center_content = div()")
        .expect("sidebar center content");
    let center_end = renderer[center_start..]
        .find("let center = if bottom.is_empty()")
        .map(|offset| center_start + offset)
        .expect("sidebar center layout");
    let center = &renderer[center_start..center_end];

    assert!(center.contains(".size_full()"));
    assert!(center.contains(".min_w_0()"));
    assert!(center.contains(".min_h_0()"));
    assert!(center.contains(".overflow_hidden()"));
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
fn windows_native_controls_occlude_the_tab_drag_region() {
    let source = include_str!("tab_container.rs");
    let button_start = source
        .find("fn render_control_button")
        .expect("window control button renderer");
    let button_end = source[button_start..]
        .find("/// 渲染窗口置顶按钮")
        .map(|offset| button_start + offset)
        .expect("always-on-top renderer");
    let button = &source[button_start..button_end];
    let windows_branch_start = button
        .find(".when(is_windows")
        .expect("Windows control branch");
    let windows_branch = &button[windows_branch_start..];

    assert!(
        windows_branch.contains("this.occlude().window_control_area(control_area)"),
        "Windows caption buttons must block the broader tab Drag hitbox before declaring \
         Min/Max/Close areas"
    );
}

#[test]
fn tab_items_keep_visible_blocks_and_a_distinct_active_outline() {
    let source = include_str!("tab_container.rs");
    let tab_bar_start = source
        .find("pub fn render_tab_bar")
        .expect("tab bar renderer");
    let tab_bar = &source[tab_bar_start..];

    assert!(tab_bar.contains("let inactive_tab_border_color = border_color.opacity(0.65);"));
    assert!(tab_bar.contains("let active_tab_border_color = theme.primary.opacity(0.85);"));
    assert!(tab_bar.matches(".border_1()").count() >= 2);
    assert!(
        tab_bar
            .matches(".border_color(inactive_tab_border_color)")
            .count()
            >= 2
    );
    assert!(
        tab_bar
            .matches("el.bg(active_tab_color)\n                                    .border_color(active_tab_border_color)")
            .count()
            >= 2
    );
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
