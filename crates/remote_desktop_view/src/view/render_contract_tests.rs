#[test]
fn rendered_frame_uses_a_parent_bounded_canvas_without_intrinsic_image_layout() {
    let source = include_str!("render.rs");

    let canvas_start = source
        .find("fn remote_desktop_frame_canvas")
        .expect("remote desktop frame canvas");
    let canvas_end = source[canvas_start..]
        .find("impl Focusable for RemoteDesktopView")
        .map(|offset| canvas_start + offset)
        .expect("remote desktop view implementation");
    let canvas = &source[canvas_start..canvas_end];

    assert!(canvas.contains("canvas("));
    assert!(canvas.contains("window.handle_input("));
    assert!(canvas.contains("window.paint_image("));
    assert!(
        canvas.matches("window.paint_image(").count() >= 2,
        "framebuffer and remote cursor must be painted in the same bounded canvas"
    );
    let paint_phase = canvas
        .find("move |bounds, frame, window, cx|")
        .expect("remote desktop canvas paint phase");
    assert!(
        !canvas[..paint_phase].contains("window.handle_input("),
        "Window::handle_input may only be called during GPUI paint"
    );
    assert!(canvas[paint_phase..].contains("window.handle_input("));
    assert!(canvas.contains(".absolute()"));
    assert!(canvas.contains(".inset_0()"));
    assert!(canvas.contains(".size_full()"));
    assert!(canvas.contains(".min_w_0()"));
    assert!(canvas.contains(".min_h_0()"));
    assert!(canvas.contains(".overflow_hidden()"));
    assert!(
        !canvas.contains("img("),
        "GPUI Img injects remote image dimensions during request_layout"
    );
    let frame_paint = canvas
        .find("paint_remote_frame(")
        .expect("framebuffer paint helper");
    let cursor_paint = canvas
        .find("paint_remote_cursor(")
        .expect("remote cursor paint helper");
    assert!(
        frame_paint < cursor_paint,
        "the remote cursor must be painted over the framebuffer"
    );

    assert_parent_bounded_remote_desktop_content(source);
}

#[test]
fn remote_cursor_never_calls_gpui_paint_only_cursor_apis_from_output_callbacks() {
    let output = include_str!("output.rs");
    let cursor = include_str!("cursor.rs");
    let native_cursor = include_str!("../native_cursor.rs");

    assert!(!output.contains("set_cursor_style"));
    assert!(!cursor.contains("set_cursor_style"));
    assert!(!native_cursor.contains("ShowCursor"));
    assert!(native_cursor.contains("SetCursor"));
}

fn assert_parent_bounded_remote_desktop_content(source: &str) {
    let content_start = source
        .find("let content = div()")
        .expect("remote desktop content");
    let root_start = source[content_start..]
        .find("\n        div()\n            .size_full()\n            .min_w_0()")
        .map(|offset| content_start + offset)
        .expect("remote desktop root");
    let content = &source[content_start..root_start];
    let root = &source[root_start..];

    for constraint in [
        ".size_full()",
        ".min_w_0()",
        ".min_h_0()",
        ".relative()",
        ".overflow_hidden()",
    ] {
        assert!(content.contains(constraint));
    }
    assert!(content.contains(".child(remote_desktop_frame_canvas(canvas_paint, focus_handle))"));
    assert!(
        !content.contains(".when_some(rendered_frame"),
        "frame replacement must not switch between Img and status layout trees"
    );

    let status = &content[content
        .find(".when(show_empty_status")
        .expect("empty-frame status")..];
    assert!(status.contains(".min_w_0()"));
    assert!(status.contains(".max_w_full()"));
    assert!(status.contains(".overflow_hidden()"));
    for constraint in [
        ".size_full()",
        ".min_w_0()",
        ".min_h_0()",
        ".overflow_hidden()",
    ] {
        assert!(root.contains(constraint));
    }
}

#[test]
fn reconnect_keeps_the_presented_frame_visible() {
    let source = include_str!("render.rs");

    assert!(
        source.contains("let rendered_frame = self.rendered_frames.current().cloned();"),
        "the presentation frame must not be gated by the transient connected flag"
    );
    assert!(
        !source.contains(".then(|| self.rendered_frames.current().cloned())"),
        "a reconnect must not blank the last frame"
    );
}

#[test]
fn reconnect_status_uses_a_transient_notification_outside_tab_content() {
    let output = include_str!("output.rs");
    let notifications = include_str!("notifications.rs");
    let render = include_str!("render.rs");

    assert!(notifications.contains("window.defer(cx"));
    assert!(notifications.contains("Notification::info(message)"));
    assert!(notifications.contains("localized_reconnect_notification("));
    assert!(!notifications.contains("localized_reconnect_status("));
    assert!(output.contains("self.reset_session_state(None, SessionResetReason::Reconnecting)"));
    assert!(output.contains("self.notify_reconnecting(reconnect, window, cx)"));
    assert!(!output.contains("RemoteDesktopOutput::Reconnecting(message)"));
    assert!(notifications.contains(".id1::<RemoteDesktopReconnectNotification>("));
    assert!(notifications.contains(".autohide(true)"));
    assert!(!render.contains("show_status_overlay"));
    assert!(!render.contains("remote-desktop-status-overlay"));
}
