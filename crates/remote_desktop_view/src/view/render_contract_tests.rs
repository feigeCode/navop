#[test]
fn rendered_frame_uses_a_parent_bounded_canvas_without_intrinsic_image_layout() {
    let source = include_str!("render.rs").replace("\r\n", "\n");

    let canvas_start = source
        .find("fn remote_desktop_frame_canvas")
        .expect("remote desktop frame canvas");
    let canvas_end = source[canvas_start..]
        .find("impl Focusable for RemoteDesktopView")
        .map(|offset| canvas_start + offset)
        .expect("remote desktop view implementation");
    let canvas = &source[canvas_start..canvas_end];

    assert!(canvas.contains("canvas("));
    assert!(
        !canvas.contains("window.handle_input("),
        "remote desktops must not register the local platform IME"
    );
    assert!(canvas.contains("window.paint_image("));
    assert!(
        canvas.matches("window.paint_image(").count() >= 2,
        "framebuffer tiles and remote cursor must be painted in the same bounded canvas"
    );
    assert!(canvas.contains("for tile in frame.tiles()"));
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

    assert_parent_bounded_remote_desktop_content(&source);
}

#[test]
fn remote_cursor_never_calls_gpui_paint_only_cursor_apis_from_output_callbacks() {
    let output = include_str!("output.rs");
    let cursor = include_str!("cursor.rs");
    let native_cursor = include_str!("../native_cursor.rs");
    let view = include_str!("../view.rs");

    assert!(!output.contains("set_cursor_style"));
    assert!(!cursor.contains("set_cursor_style"));
    assert!(cursor.contains("self.has_paintable_bitmap()"));
    assert!(cursor.contains("if !self.manage_native_cursor"));
    assert!(view.contains(
        "let manage_native_cursor = config.options.protocol == RemoteDesktopProtocol::Rdp;"
    ));
    assert!(view.contains("RemoteCursorState::new(manage_native_cursor)"));
    assert!(!native_cursor.contains("ShowCursor"));
    assert!(native_cursor.contains("SetCursor"));
}

#[test]
fn local_pointer_move_makes_the_canvas_cursor_paintable_before_hiding_native_cursor() {
    let input = include_str!("input.rs");
    let move_start = input
        .find("pub(super) fn send_pointer_move")
        .expect("pointer move handler");
    let move_end = input[move_start..]
        .find("pub(super) fn send_mouse_button")
        .map(|offset| move_start + offset)
        .expect("mouse button handler");
    let pointer_move = &input[move_start..move_end];

    let pointer_hover = pointer_move
        .find("self.cursor.set_pointer_hovered(true);")
        .expect("pointer movement must keep hover state synchronized");
    let cursor_position = pointer_move
        .find("let position_changed = self.cursor.set_position(x, y);")
        .expect("local pointer movement must predict the canvas cursor position");
    let native_rehide = pointer_move
        .find("self.cursor.rehide_native_cursor_after_pointer_move();")
        .expect("native cursor must be hidden once after local prediction");
    let immediate_repaint = pointer_move
        .find("cx.notify();")
        .expect("canvas cursor movement must request an immediate repaint");
    let remote_input = pointer_move
        .find("self.send_input(RemoteDesktopInput::MouseMove { x, y });")
        .expect("pointer movement must still be sent to the remote session");

    assert!(pointer_hover < cursor_position);
    assert!(cursor_position < native_rehide);
    assert!(native_rehide < immediate_repaint);
    assert!(immediate_repaint < remote_input);
    assert!(pointer_move.contains("if position_changed {"));
    assert!(!pointer_move.contains("refresh_native_cursor"));
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
    assert!(content.contains(".child(remote_desktop_frame_canvas(canvas_paint))"));
    assert!(
        !content.contains(".when_some(rendered_frame"),
        "frame replacement must not switch between Img and status layout trees"
    );

    let status = &content[content
        .find(".when(show_empty_status")
        .expect("empty-frame status")..];
    assert!(status.contains(".min_w_0()"));
    assert!(status.contains(".max_w_full()"));
    assert!(status.contains(".flex_shrink_0()"));
    assert!(status.contains(".overflow_hidden()"));
    assert!(status.contains(".whitespace_nowrap()"));
    assert!(status.contains(".text_center()"));
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
fn key_events_only_refresh_capslock_without_overwriting_physical_modifiers() {
    let source = include_str!("input.rs").replace("\r\n", "\n");
    let key_down = function_body(
        &source,
        "pub(super) fn handle_key_down",
        "pub(super) fn handle_key_up",
    );
    let key_up = function_body(
        &source,
        "pub(super) fn handle_key_up",
        "pub(super) fn send_tab",
    );

    for handler in [key_down, key_up] {
        assert!(handler.contains("self.sync_rdp_capslock_state(window.capslock());"));
        assert!(
            !handler.contains("event.keystroke.modifiers"),
            "GPUI folds Shift into printable punctuation and clears this modifier snapshot"
        );
    }
}

fn function_body<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("function start");
    let end = source[start..]
        .find(end)
        .map(|offset| start + offset)
        .expect("next function");
    &source[start..end]
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
    assert!(
        output.contains("self.reset_session_state(None, SessionResetReason::Reconnecting, cx)")
    );
    assert!(output.contains("self.notify_reconnecting(reconnect, window, cx)"));
    assert!(!output.contains("RemoteDesktopOutput::Reconnecting(message)"));
    assert!(notifications.contains(".id1::<RemoteDesktopReconnectNotification>("));
    assert!(notifications.contains(".autohide(true)"));
    assert!(!render.contains("show_status_overlay"));
    assert!(!render.contains("remote-desktop-status-overlay"));
}

#[test]
fn session_takeover_notifies_the_user_and_requests_tab_close_without_reconnecting() {
    let output = include_str!("output.rs");
    let notifications = include_str!("notifications.rs");

    assert!(output.contains("RemoteDesktopFailure::SessionTakenOver"));
    assert!(output.contains("self.notify_session_taken_over(window, cx)"));
    assert!(output.contains("cx.emit(TabContentEvent::CloseRequested)"));
    assert!(notifications.contains("Notification::warning(message)"));
    assert!(notifications.contains("RemoteDesktopSessionNotification"));
    assert!(notifications.contains("localized_session_taken_over"));
}

#[test]
fn remote_output_wakes_render_without_fixed_interval_polling() {
    let view = include_str!("../view.rs");
    let output = include_str!("output.rs");

    assert!(
        !view.contains("Duration::from_millis(33)"),
        "remote output must not wait for the former 33ms render polling interval"
    );
    assert!(
        !view.contains("_output_poll_task"),
        "the fixed-interval output polling task must stay removed"
    );
    assert!(
        output.contains("runtime.output_rx.subscribe()"),
        "the view must subscribe to mailbox output-ready events"
    );
    assert!(
        output.contains("output_ready.wait().await"),
        "the view must wait for mailbox output-ready events"
    );
    assert!(
        output.contains("this.update(cx, |_, cx| cx.notify())"),
        "mailbox output-ready events must request a GPUI render"
    );
}
