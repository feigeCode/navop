fn popup_open_source() -> &'static str {
    let source = include_str!("popup_window.rs");
    let open_start = source
        .find("pub fn open_popup_window")
        .expect("popup window opener");
    let content_start = source[open_start..]
        .find("\nstruct PopupWindowContent")
        .map(|offset| open_start + offset)
        .expect("popup window content");
    &source[open_start..content_start]
}

#[test]
fn fullscreen_hidden_titlebar_is_revealed_from_a_top_edge_hover_zone() {
    let source = include_str!("popup_window.rs");
    let render_start = source
        .find("impl Render for PopupWindowContent")
        .expect("popup content renderer");
    let render = &source[render_start..];

    assert!(render.contains("let auto_hide_titlebar"));
    assert!(render.contains(".id(\"fullscreen-titlebar-reveal-zone\")"));
    assert!(render.contains(".absolute()"));
    assert!(render.contains(".top_0()"));
    assert!(render.contains(".on_hover(cx.listener"));
    assert!(render.contains("this.titlebar_revealed = true"));
    assert!(render.contains("this.titlebar_revealed = false"));
    assert!(render.contains("TitleBar::new()"));
    assert!(render.contains(".id(\"fullscreen-titlebar\")"));
    assert!(
        render.contains(".when(auto_hide_titlebar && !self.titlebar_revealed"),
        "the embedded content must reserve the top hover strip while hidden"
    );
    assert!(render.contains("this.pt(px(4.0))"));
}

#[test]
fn escape_exits_auto_hidden_popup_fullscreen() {
    let source = include_str!("popup_window.rs");

    assert!(source.contains("KeyBinding::new("));
    assert!(source.contains("\"escape\","));
    assert!(source.contains("ExitPopupFullscreen,"));
    assert!(source.contains(".when(auto_hide_titlebar"));
    assert!(source.contains(".key_context(FULLSCREEN_POPUP_CONTEXT)"));
    assert!(source.contains(".on_action(cx.listener"));
    assert!(source.contains("window.toggle_fullscreen()"));
    assert!(source.contains("cx.stop_propagation()"));
}

#[test]
fn popup_fullscreen_hint_uses_an_auto_hiding_notification() {
    let source = include_str!("popup_window.rs");
    let open = popup_open_source();

    assert!(source.contains("fullscreen_hint: Option<SharedString>"));
    assert!(open.contains("if let Some(fullscreen_hint)"));
    assert!(open.contains("cx.update_window(window.into()"));
    assert!(open.contains("Notification::info(fullscreen_hint)"));
    assert!(open.contains(".autohide(true)"));
    assert!(open.contains("window.push_notification"));
}

#[test]
fn every_popup_registers_with_the_shared_window_close_router() {
    let open = popup_open_source();

    assert!(open.contains("crate::window_close::register_window"));
    assert!(open.contains("window.window_handle()"));
}

#[test]
fn popup_uses_the_active_window_display() {
    let open = popup_open_source();

    assert!(open.contains("cx.active_window()"));
    assert!(open.contains("window.display(cx)"));
    assert!(open.contains("cx.primary_display()"));
    assert!(open.contains("display_id"));
    assert!(open.contains("display.visible_bounds()"));
    assert!(open.contains("Bounds::centered_at(display_bounds.center()"));
}
