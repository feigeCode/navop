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
    assert!(canvas.contains("window.update_dynamic_texture("));
    assert_eq!(
        1,
        canvas.matches("window.paint_dynamic_texture(").count(),
        "the framebuffer must be painted as one dynamic texture"
    );
    assert!(
        canvas.matches("window.paint_image(").count() == 1,
        "only the remote cursor should use the RenderImage paint path"
    );
    assert!(!canvas.contains("for tile in frame.tiles()"));
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
    assert!(source.contains("window.drop_dynamic_texture("));
    assert!(source.contains("window.drop_image("));

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
fn windows_native_close_waits_for_confirmation_and_keeps_a_release_fallback() {
    let render = include_str!("render.rs").replace("\r\n", "\n");
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let native = include_str!("windows_native.rs").replace("\r\n", "\n");

    let close_start = render
        .find("fn try_close(")
        .expect("remote desktop close implementation");
    let close_end = render[close_start..]
        .find("\n}\n\nimpl Render for RemoteDesktopView")
        .map(|offset| close_start + offset)
        .expect("end of remote desktop close implementation");
    let close = &render[close_start..close_end];

    for token in [
        "native.begin_close(&mut focus_parent)",
        "NativeCloseProgress::Ready",
        "NativeCloseProgress::WaitingForEvents",
        "finish_windows_native_close(registration, cx)",
        "force_close_windows_native(registration, cx)",
        "WindowsNativeCloseRetryMode::WaitForConfirmation",
        "WindowsNativeCloseRetryMode::ForceClose",
        "Self::retry_windows_native_close(",
    ] {
        assert!(close.contains(token), "missing native close token: {token}");
    }

    let retry = function_body(
        &render,
        "fn retry_windows_native_close(",
        "impl Focusable for RemoteDesktopView",
    );
    for token in [
        "WINDOWS_NATIVE_CLOSE_TIMEOUT",
        "WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT",
        "hard_deadline",
        "this.poll_windows_native_close(registration, cx)",
        "this.force_close_windows_native(registration, cx)",
        "WindowsNativeClosePoll::Pending",
        "Duration::from_millis(16)",
    ] {
        assert!(
            retry.contains(token),
            "missing native close retry token: {token}"
        );
    }
    assert!(retry.contains("if now >= hard_deadline"));
    assert!(retry.contains("return false;"));

    let release_start = view
        .find("cx.on_release(move |this, cx|")
        .expect("view release hook");
    let release_end = view[release_start..]
        .find("\n        })\n        .detach();")
        .map(|offset| release_start + offset)
        .expect("end of view release hook");
    let release = &view[release_start..release_end];
    assert!(release.contains("native.force_close(&mut focus_parent)"));
    assert!(release.contains("window.focus(&focus_handle, cx);"));
    let native_take = release
        .find("let Some(mut native) = this.windows_native.take()")
        .expect("release must take ownership of the native adapter");
    let event_state_take = release
        .find("this.native_event_state.take();")
        .expect("release must invalidate the native event reducer");
    let force_close = release
        .find("native.force_close(&mut focus_parent)")
        .expect("release must synchronously attempt owner-thread cleanup");
    let detached_cleanup = release
        .find("detach_windows_native_cleanup(native, registration, cx, \"view release\")")
        .expect("release must retain pending native cleanup on the owner thread");
    assert!(native_take < event_state_take);
    assert!(event_state_take < force_close);
    assert!(native_take < detached_cleanup);
    assert!(release.contains("Ok(windows_native::NativeDestroyProgress::Destroyed) => true"));
    assert!(
        release.contains("Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false")
    );
    assert!(release.contains("native.is_destroyed()"));
    assert!(release.contains("if destroyed"));
    assert!(release.contains("mark_windows_native_rdp_detached"));

    let detached_cleanup = function_body(
        &view,
        "fn detach_windows_native_cleanup(",
        "pub struct RemoteDesktopViewConfig",
    );
    for token in [
        "WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT",
        "cx.spawn(async move |cx|",
        "native.force_close(&mut focus_parent)",
        "NativeDestroyProgress::PendingCallbacks",
        "record_detached_windows_native_terminal",
        "WindowsRdpTerminalOutcome::Destroyed",
        "WindowsRdpTerminalOutcome::TimedOutLeaked",
        "Duration::from_millis(16)",
        "Box::leak(Box::new(native))",
        ".detach();",
    ] {
        assert!(
            detached_cleanup.contains(token),
            "missing detached native cleanup token: {token}"
        );
    }
    assert!(!detached_cleanup.contains("background_spawn"));
    assert!(!detached_cleanup.contains("tokio::spawn"));
    let detached_terminal = function_body(
        &view,
        "fn record_detached_windows_native_terminal(",
        "fn detach_windows_native_cleanup(",
    );
    assert!(detached_terminal.contains("record_windows_native_rdp_terminal_async"));
    assert!(
        detached_terminal.contains(".was_rejected()"),
        "detached cleanup must observe terminal dispatcher rejection"
    );
    assert!(
        !detached_terminal.contains("native.force_close"),
        "terminal dispatcher rejection must never trigger direct native cleanup"
    );
    let foreground_spawn = detached_cleanup
        .find("cx.spawn(async move |cx|")
        .expect("foreground cleanup spawn");
    let force_close = detached_cleanup
        .find("native.force_close(&mut focus_parent)")
        .expect("detached force close");
    let destroyed_branch = detached_cleanup
        .find("NativeDestroyProgress::Destroyed) => {")
        .expect("destroyed cleanup branch");
    let destroyed_terminal = detached_cleanup[destroyed_branch..]
        .find("WindowsRdpTerminalOutcome::Destroyed")
        .map(|offset| destroyed_branch + offset)
        .expect("destroyed terminal completion");
    let destroyed_return = detached_cleanup[destroyed_terminal..]
        .find("return;")
        .map(|offset| destroyed_terminal + offset)
        .expect("destroyed cleanup termination");
    let already_destroyed_branch = detached_cleanup
        .find("Err(_) if native.is_destroyed() => {")
        .expect("already-destroyed cleanup branch");
    let already_destroyed_return = detached_cleanup[already_destroyed_branch..]
        .find("return;")
        .map(|offset| already_destroyed_branch + offset)
        .expect("already-destroyed cleanup termination");
    let deadline = detached_cleanup
        .find("if Instant::now() >= deadline")
        .expect("detached cleanup deadline");
    let leak = detached_cleanup
        .find("Box::leak(Box::new(native))")
        .expect("fail-closed adapter leak");
    let timer = detached_cleanup
        .find("timer(Duration::from_millis(16))")
        .expect("detached cleanup retry timer");
    let detach = detached_cleanup
        .rfind(".detach();")
        .expect("detached cleanup task ownership");
    assert!(foreground_spawn < force_close);
    assert!(force_close < destroyed_branch);
    assert!(destroyed_branch < destroyed_terminal);
    assert!(destroyed_terminal < destroyed_return);
    assert!(destroyed_return < already_destroyed_return);
    assert!(already_destroyed_return < deadline);
    assert!(deadline < leak);
    assert!(leak < timer);
    assert!(timer < detach);

    for token in [
        "NativePresentationState::Closing",
        "self.state != NativePresentationState::Open",
        "self.host.request_close()?",
        "self.host.disconnect()",
        "WindowsRdpHostError::CallbackInFlight",
        "NativeDestroyProgress::PendingCallbacks",
        "NativeDestroyProgress::Destroyed",
    ] {
        assert!(
            native.contains(token),
            "missing native shutdown state-machine token: {token}"
        );
    }
}

#[test]
fn windows_native_hosts_keep_full_shutdown_registrations_through_every_terminal_path() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let initialize = function_body(
        &view,
        "fn ensure_windows_native_presentation",
        "fn fail_presentation_initialization",
    );
    let create = initialize
        .find("WindowsNativeAdapter::create")
        .expect("native presentation creation");
    let register = initialize[create..]
        .find("register_windows_native_rdp")
        .map(|offset| create + offset)
        .expect("application shutdown registration");
    let post_create = initialize
        .find("let Some(bounds)")
        .expect("post-create native initialization");
    assert!(
        create < register && register < post_create,
        "the created adapter must be admitted before bounds, connect, or attach work"
    );
    assert!(initialize.contains("native.generation()"));
    assert!(initialize.contains("WindowsRdpRegistrationError"));
    assert!(initialize.contains("fail_unregistered_windows_native_presentation"));
    assert!(
        initialize
            .matches("fail_windows_native_presentation(")
            .count()
            >= 5,
        "every admitted post-create failure must retain its full registration"
    );
    assert!(initialize.contains("self.attach_windows_native_presentation(native, registration,"));

    assert!(
        view.contains(
            "windows_native_registration: Option<windows_rdp_host::WindowsRdpRegistration>"
        )
    );
    assert!(view.contains("windows_native_registration: None"));

    let failure = function_body(
        &view,
        "fn fail_windows_native_presentation",
        "fn fail_unregistered_windows_native_presentation",
    );
    assert!(failure.contains("registration: windows_rdp_host::WindowsRdpRegistration"));
    assert!(failure.contains("WindowsRdpTerminalOutcome::Destroyed"));
    assert!(
        failure.contains("detach_windows_native_cleanup(native, Some(registration), cx, stage)")
    );

    let attach = function_body(
        &view,
        "fn attach_windows_native_presentation",
        "pub(super) fn update_windows_native_bounds",
    );
    assert!(attach.contains("registration: windows_rdp_host::WindowsRdpRegistration"));
    assert!(attach.contains("registration.generation()"));
    assert!(attach.contains("presentation.generation()"));
    assert!(attach.contains("self.windows_native_registration = Some(registration);"));

    let release_start = view
        .find("cx.on_release(move |this, cx|")
        .expect("view release hook");
    let release_end = view[release_start..]
        .find("\n        })\n        .detach();")
        .map(|offset| release_start + offset)
        .expect("end of view release hook");
    let release = &view[release_start..release_end];
    let native_take = release
        .find("this.windows_native.take()")
        .expect("native adapter take");
    let registration_take = release
        .find("this.windows_native_registration.take()")
        .expect("shutdown registration take");
    let force_close = release
        .find("native.force_close(&mut focus_parent)")
        .expect("owner-thread force close");
    assert!(native_take < registration_take);
    assert!(registration_take < force_close);
    assert!(release.contains("WindowsRdpTerminalOutcome::Destroyed"));
    assert!(release.contains("Some(registration)"));
    assert!(
        release.contains("WindowsRdpTerminalOutcome::OwnerLost"),
        "a registration whose adapter ownership disappeared must still converge the app drain"
    );
}

#[test]
fn windows_native_shutdown_controller_drains_on_the_foreground_and_leaks_before_timeout_completion()
{
    let controller = [
        include_str!("../windows_native_shutdown.rs"),
        include_str!("../windows_native_shutdown/platform.rs"),
        include_str!("../windows_native_shutdown/platform/drain.rs"),
    ]
    .join("\n")
    .replace("\r\n", "\n");
    let view = include_str!("../view.rs").replace("\r\n", "\n");

    for token in [
        "WindowsRdpShutdownRegistry",
        "BTreeMap<WindowsRdpRegistration, WindowsNativeRdpOwner>",
        "WeakEntity<RemoteDesktopView>",
        "registry.begin_drain()",
        "registry.pending_registrations()",
        ".fail_closed_report()",
        "owner.update(cx",
        "force_close_windows_native_for_shutdown",
        "quarantine_windows_native_for_shutdown",
        "cx.spawn(async move |cx|",
        "Duration::from_millis(16)",
    ] {
        assert!(
            controller.contains(token),
            "missing shutdown controller token: {token}"
        );
    }
    let missing_owner_branch = controller
        .find("fn record_missing_owner(")
        .expect("missing-owner branch");
    let owner_lost = controller[missing_owner_branch..]
        .find("WindowsRdpTerminalOutcome::OwnerLost")
        .expect("missing-owner terminal completion after the deadline");
    assert!(
        controller[missing_owner_branch..missing_owner_branch + owner_lost]
            .contains("if deadline_elapsed"),
        "owner loss must only become terminal after the bounded drain deadline"
    );
    assert!(
        controller[missing_owner_branch..]
            .contains("Windows native RDP shutdown registration has no owner")
    );
    let detached_owner_branch = controller
        .find("fn record_stalled_detached_owner(")
        .expect("detached-owner branch");
    let detached_owner_lost = controller[detached_owner_branch..]
        .find("WindowsRdpTerminalOutcome::OwnerLost")
        .expect("detached owner must converge after the bounded drain deadline");
    assert!(
        controller[detached_owner_branch..detached_owner_branch + detached_owner_lost]
            .contains("deadline_elapsed"),
        "detached cleanup must retain its normal owner-thread deadline before app drain fallback"
    );
    assert!(
        controller[detached_owner_branch..].contains("detached cleanup did not report a terminal")
    );
    let released_owner_branch = controller
        .find("fn record_released_view_owner(")
        .expect("released-view owner branch");
    let released_owner_end = controller[released_owner_branch..]
        .find("\n}\n\nfn ")
        .map(|offset| released_owner_branch + offset)
        .expect("end of released-view owner branch");
    let released_owner = &controller[released_owner_branch..released_owner_end];
    assert!(
        released_owner.contains("deadline_elapsed"),
        "a released weak view must retain the bounded deadline before owner-loss completion"
    );
    assert!(released_owner.contains("record_windows_native_rdp_view_owner_lost_async"));
    let drain = function_body(
        &controller,
        "async fn drain(",
        "pub fn shutdown_windows_native_rdp",
    );
    assert!(
        drain.contains("return fail_closed_report;"),
        "controller loss must return the last conservative drain report"
    );
    assert!(
        !drain.contains("WindowsNativeRdpShutdownReport::default()"),
        "controller loss must not be misreported as an empty successful drain"
    );
    let shutdown_entrypoint = function_body(
        &controller,
        "pub fn shutdown_windows_native_rdp(cx: &mut App)",
        "\n}\n",
    );
    assert!(
        shutdown_entrypoint.contains("WindowsNativeRdpShutdownReport::unavailable_controller()"),
        "a missing shutdown controller must produce an explicitly incomplete report"
    );
    assert!(!controller.contains("WindowsNativeAdapter"));
    assert!(!controller.contains("WindowsRdpHost"));
    assert!(!controller.contains("background_spawn"));
    assert!(!controller.contains("tokio::spawn"));

    let detached_cleanup = function_body(
        &view,
        "fn detach_windows_native_cleanup(",
        "pub struct RemoteDesktopViewConfig",
    );
    assert!(
        detached_cleanup.contains("registration: Option<windows_rdp_host::WindowsRdpRegistration>")
    );
    let leak = detached_cleanup
        .find("Box::leak(Box::new(native))")
        .expect("fail-closed adapter leak");
    let timed_out = detached_cleanup
        .find("WindowsRdpTerminalOutcome::TimedOutLeaked")
        .expect("timeout completion");
    assert!(
        leak < timed_out,
        "the complete adapter must be leaked before recording timeout completion"
    );

    let quarantine = function_body(
        &view,
        "fn quarantine_windows_native_for_shutdown",
        "fn poll_windows_native_close",
    );
    assert!(quarantine.contains("self.windows_native_registration == Some(registration)"));
    assert!(
        quarantine.contains("return false;"),
        "registration/adapter mismatches must be observable by the drain controller"
    );
    let quarantine_take = quarantine
        .find(".windows_native\n            .take()")
        .expect("quarantine adapter take");
    let quarantine_leak = quarantine
        .find("Box::leak(Box::new(native))")
        .expect("quarantine adapter leak");
    let quarantine_terminal = quarantine
        .find("WindowsRdpTerminalOutcome::TimedOutLeaked")
        .expect("quarantine timeout completion");
    assert!(quarantine_take < quarantine_leak);
    assert!(quarantine_leak < quarantine_terminal);
    assert!(
        quarantine.contains("\n        true\n"),
        "successful quarantine must be acknowledged to the drain controller"
    );
}

#[test]
fn windows_native_shutdown_uses_locked_gpui_context_contracts() {
    let facade = include_str!("../windows_native_shutdown.rs").replace("\r\n", "\n");
    let platform = include_str!("../windows_native_shutdown/platform.rs").replace("\r\n", "\n");
    let drain = include_str!("../windows_native_shutdown/platform/drain.rs").replace("\r\n", "\n");

    assert!(
        facade.starts_with(
            "#[cfg(not(all(feature = \"windows-native-rdp\", target_os = \"windows\")))]\n\
             use gpui::{App, Task};"
        ),
        "the non-Windows facade imports must not become unused in the native Windows build"
    );
    assert!(
        drain.contains("use gpui::{App, BorrowAppContext, Task};"),
        "the synchronous App drain entrypoint must import BorrowAppContext"
    );
    for signature in [
        "fn poll_view_owner(\n",
        "fn poll_registration(\n",
        "async fn drain(\n",
    ] {
        let body = &drain[drain
            .find(signature)
            .unwrap_or_else(|| panic!("missing drain function: {signature}"))..];
        assert!(
            body.contains("cx: &mut gpui::AsyncApp"),
            "{signature} must retain mutable AsyncApp access for WeakEntity::update"
        );
    }
    let poll_registration =
        function_body(&drain, "fn poll_registration(\n", "fn completed_report(");
    for call in [
        "record_stalled_detached_owner(registration, deadline_elapsed, cx)",
        "poll_view_owner(owner, registration, deadline_elapsed, cx)",
    ] {
        assert!(
            poll_registration.contains(call),
            "poll_registration must preserve terminal dispatcher delivery from {call}"
        );
        assert!(
            !poll_registration.contains(&format!("{call};")),
            "poll_registration must not discard terminal dispatcher rejection from {call}"
        );
    }
    let try_update = function_body(
        &platform,
        "fn try_update_windows_native_rdp_shutdown<R>(",
        "pub(crate) fn record_windows_native_rdp_terminal_async(",
    );
    assert!(
        try_update.contains("try_read_global::<GlobalWindowsNativeRdpShutdown"),
        "AsyncApp terminal dispatch must reject updates after GPUI begins quitting"
    );
    assert!(
        try_update.contains("Some(cx.update_global(update))"),
        "accepted AsyncApp terminal dispatch must update the shutdown global"
    );
    for (signature, end) in [
        (
            "pub(crate) fn record_windows_native_rdp_terminal_async(",
            "pub(super) fn record_windows_native_rdp_view_owner_lost_async(",
        ),
        (
            "pub(super) fn record_windows_native_rdp_view_owner_lost_async(",
            "pub use drain::{",
        ),
    ] {
        let body = function_body(&platform, signature, end);
        assert!(
            body.contains("let result = try_update_windows_native_rdp_shutdown"),
            "{signature} must retain AsyncApp dispatcher rejection"
        );
        assert!(
            body.contains("WindowsNativeRdpTerminalDispatch::from_option(result)"),
            "{signature} must return observable dispatcher delivery"
        );
    }
    let rejection = drain
        .find(".was_rejected()")
        .expect("bounded drain dispatcher rejection branch");
    let fail_closed_return = drain[rejection..]
        .find("return fail_closed_report;")
        .map(|offset| rejection + offset)
        .expect("dispatcher rejection must return the last conservative report");
    let rejection_branch = &drain[rejection..fail_closed_return];
    assert!(
        !rejection_branch.contains("force_close_windows_native"),
        "dispatcher rejection must not fall back to wrong-thread native cleanup"
    );

    let platform_quit = function_body(
        &drain,
        "pub fn fail_closed_windows_native_rdp_for_platform_quit(",
        "pub fn shutdown_windows_native_rdp(cx: &mut App)",
    );
    assert!(
        platform_quit.contains("let start = begin_drain(cx);"),
        "platform-driven quit must synchronously close Native RDP admission"
    );
    assert!(
        platform_quit.contains("start.completed_report.unwrap_or(start.fail_closed_report)"),
        "platform-driven quit must return a conservative report for every pending registration"
    );
    for forbidden in [
        "cx.spawn",
        "drain(cx, start.fail_closed_report)",
        "poll_registration",
        "force_close_windows_native",
        "WindowsRdpTerminalOutcome::Destroyed",
    ] {
        assert!(
            !platform_quit.contains(forbidden),
            "late platform quit fallback must not perform owner-thread native cleanup: {forbidden}"
        );
    }
}

#[test]
fn windows_native_events_are_drained_on_the_gpui_owner_thread() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let native = include_str!("windows_native.rs").replace("\r\n", "\n");

    let poll_task = function_body(
        &view,
        "let output_poll_task = cx.spawn",
        "cx.on_release(move |this, cx|",
    );
    assert!(poll_task.contains("this.poll_windows_native_events()"));
    assert!(poll_task.contains("native_event_window_handle.update"));
    assert!(poll_task.contains("window.focus(&focus_handle, cx);"));

    let poll = function_body(
        &view,
        "fn poll_windows_native_events",
        "fn poll_windows_native_close",
    );
    assert!(poll.contains("native.drain_events(event_state)"));
    assert!(poll.contains("event_state.take_focus_release_pending()"));
    assert!(poll.contains("self.tab_active"));
    assert!(poll.contains("self.focus_handle.clone()"));

    assert!(native.contains("pub(super) fn drain_events("));
    assert!(native.contains("state.close_confirmed()"));
}

#[test]
fn presentation_initialization_precedes_and_gates_canvas_runtime_start() {
    let render = include_str!("render.rs").replace("\r\n", "\n");
    let output = include_str!("output.rs").replace("\r\n", "\n");

    let ensure = render
        .find("self.ensure_presentation(window, cx);")
        .expect("presentation initialization");
    let flush = render
        .find("self.flush_pending_start();")
        .expect("pending Canvas start");
    assert!(
        ensure < flush,
        "native selection and creation must finish before Canvas can start"
    );

    let start = function_body(
        &output,
        "pub(super) fn start_runtime",
        "pub(super) fn drain_output",
    );
    assert!(start.contains("if !self.presentation_initialization.allows_canvas_runtime()"));
}

#[test]
fn windows_native_initialization_orders_create_bounds_connect_and_attach() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let initialize = function_body(
        &view,
        "fn ensure_windows_native_presentation",
        "fn fail_presentation_initialization",
    );

    let proxy_check = initialize
        .find("if proxy_configured")
        .expect("proxy capability check");
    let create = initialize
        .find("WindowsNativeAdapter::create")
        .expect("native presentation creation");
    assert!(
        proxy_check < create,
        "unsupported SOCKS/HTTP proxy settings must fail before creating a native host"
    );
    assert!(initialize.contains("WindowsNativePresentationCreateError::ProxyUnsupported => None"));
    let proxy_failure = function_body(
        initialize,
        "WindowsNativePresentationCreateError::ProxyUnsupported,\n            )) =>",
        "WindowsNativePresentationCreateError::Adapter(error),\n            )) =>",
    );
    assert!(proxy_failure.contains("self.fail_presentation_initialization("));
    assert!(
        proxy_failure
            .contains("RemoteDesktopPresentation::NativeWindows,\n                    true,")
    );
    assert!(
        !proxy_failure.contains("RemoteDesktopPresentationInitialization::Canvas"),
        "Auto must fail closed rather than bypassing an unsupported proxy via Canvas fallback"
    );

    let post_create_start = initialize
        .find("let Some(bounds)")
        .expect("post-create native initialization");
    let post_create = &initialize[post_create_start..];
    let mut previous = 0;
    for token in [
        "native.update_bounds",
        "parse_destination",
        "WindowsRdpConnectionOptions::new",
        "native.connect",
        "attach_windows_native_presentation",
        "RemoteDesktopPresentationInitialization::Native",
    ] {
        let position = post_create[previous..]
            .find(token)
            .map(|offset| previous + offset)
            .unwrap_or_else(|| panic!("missing ordered native initialization token: {token}"));
        previous = position + token.len();
    }

    assert!(
        !post_create.contains("RemoteDesktopPresentationInitialization::Canvas"),
        "once a native host exists, later setup/connect failures must not open a Canvas session"
    );
    assert!(
        post_create
            .matches("fail_windows_native_presentation")
            .count()
            >= 5,
        "all post-create initialization failures must close the native host and fail closed"
    );
    for (offset, _) in post_create.match_indices("self.fail_windows_native_presentation(") {
        let invocation = &post_create[offset..];
        let end = invocation
            .find(");")
            .expect("native initialization failure invocation");
        assert!(
            invocation[..end].contains("cx"),
            "native initialization failure cleanup must retain ownership on the GPUI thread"
        );
    }
}

#[test]
fn explicit_canvas_retry_requires_confirmed_native_cleanup_and_defers_runtime_start() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let render = include_str!("render.rs").replace("\r\n", "\n");

    assert!(view.contains("[SendTab, SendShiftTab, RemoteCopy, RemotePaste, UseCanvas]"));

    let retry = function_body(
        &view,
        "fn use_canvas",
        "#[cfg(all(feature = \"windows-native-rdp\", target_os = \"windows\"))]\n    fn fail_windows_native_presentation",
    );
    let retry_gate = retry
        .find("allows_explicit_canvas_retry()")
        .expect("retry state gate");
    let native_guard = retry
        .find("if self.windows_native.is_some()")
        .expect("native child guard");
    let close_canvas = retry
        .find("close_runtime_once(&mut self.input_tx);")
        .expect("existing Canvas runtime close");
    let select_canvas = retry
        .find("RemoteDesktopPresentationInitialization::Canvas")
        .expect("explicit Canvas selection");
    assert!(retry_gate < native_guard);
    assert!(native_guard < close_canvas);
    assert!(close_canvas < select_canvas);
    assert!(retry.contains("self.output_rx = None;"));
    assert!(retry.contains("fallback_reason: None"));
    assert!(retry.contains("cx.notify();"));
    assert!(
        !retry.contains("start_runtime"),
        "the action must let the next render start exactly one Canvas runtime"
    );

    let failure = function_body(
        &view,
        "fn fail_windows_native_presentation",
        "pub(crate) fn attach_windows_native_presentation",
    );
    assert!(failure.contains("cx: &mut Context<Self>"));
    assert!(failure.contains("let destroyed = match native.force_close(&mut focus_parent)"));
    assert!(failure.contains("match native.force_close(&mut focus_parent)"));
    assert!(failure.contains("Ok(windows_native::NativeDestroyProgress::Destroyed) => true"));
    assert!(
        failure.contains("Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false")
    );
    assert!(failure.contains("Err(close_error) =>"));
    assert!(failure.contains("native.is_destroyed()"));
    assert!(failure.contains("let canvas_retry_available = destroyed;"));
    assert!(failure.contains("let needs_detached_cleanup = !destroyed;"));
    assert!(failure.contains("if needs_detached_cleanup"));
    assert!(failure.contains("mark_windows_native_rdp_detached(registration, cx)"));
    assert!(
        failure.contains("detach_windows_native_cleanup(native, Some(registration), cx, stage);")
    );
    assert!(failure.contains(
        "RemoteDesktopPresentation::NativeWindows,\n            canvas_retry_available,"
    ));

    assert!(render.contains(".on_action(cx.listener(Self::use_canvas))"));
    assert!(render.contains("this.use_canvas(&UseCanvas, window, cx);"));
}

#[test]
fn rdp_presentation_status_stays_outside_the_native_child_bounds() {
    let source = include_str!("render.rs").replace("\r\n", "\n");

    for token in [
        "show_presentation_status",
        ".fallback_reason()",
        "allows_explicit_canvas_retry()",
        "remote-desktop-presentation-status",
        "remote-desktop-use-canvas",
        "RemoteDesktop.presentation_backend",
        "RemoteDesktop.fallback_reason",
        "RemoteDesktop.use_canvas",
    ] {
        assert!(
            source.contains(token),
            "missing presentation UI token: {token}"
        );
    }
    for locale_key in [
        "fallback_feature_disabled",
        "fallback_unsupported_platform",
        "fallback_probe_reported_unavailable",
        "fallback_class_not_registered",
        "fallback_required_interface_missing",
    ] {
        assert!(
            source.contains(locale_key),
            "fallback UI must use stable taxonomy key: {locale_key}"
        );
    }

    let root = &source[source
        .find("\n        div()\n            .size_full()\n            .min_w_0()")
        .expect("remote desktop root")..];
    assert!(root.contains(".flex()\n            .flex_col()"));
    assert!(source.contains(".on_prepaint(move |bounds, window, cx|"));
    assert!(source.contains("view.update_content_bounds(bounds, window.scale_factor())"));
    assert!(!root.contains(".on_children_prepainted("));
    let status = root
        .find(".when(show_presentation_status")
        .expect("presentation status condition");
    let content = root.rfind(".child(content)").expect("content child");
    assert!(
        status < content,
        "the status row must be laid out before the native-child content bounds"
    );
    assert!(!source.contains("show_status_overlay"));
    assert!(!source.contains("remote-desktop-status-overlay"));
}

#[test]
fn windows_native_tab_lifecycle_defers_focus_only_while_active() {
    let render = include_str!("render.rs").replace("\r\n", "\n");
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let activate = function_body(&render, "fn on_activate", "fn on_deactivate");
    let deactivate = function_body(&render, "fn on_deactivate", "fn try_close");
    let attach = function_body(
        &view,
        "fn attach_windows_native_presentation",
        "pub(super) fn update_windows_native_bounds",
    );

    let active_assignment = activate
        .find("self.tab_active = true;")
        .expect("active lifecycle assignment");
    let native_activation = activate
        .find("self.activate_windows_native(false)")
        .expect("native activation");
    let stale_focus_drain = activate
        .find("self.poll_windows_native_events()")
        .expect("stale native focus drain");
    assert!(stale_focus_drain < active_assignment);
    assert!(active_assignment < native_activation);
    assert!(activate.contains("cx.defer_in(window"));
    assert!(activate.contains("if this.tab_active"));
    assert!(activate.contains("this.focus_windows_native();"));

    let inactive_assignment = deactivate
        .find("self.tab_active = false;")
        .expect("inactive lifecycle assignment");
    let native_deactivation = deactivate
        .find("self.deactivate_windows_native")
        .expect("native deactivation");
    let released_focus_drain = deactivate
        .find("self.poll_windows_native_events()")
        .expect("released native focus drain");
    assert!(inactive_assignment < native_deactivation);
    assert!(native_deactivation < released_focus_drain);

    assert!(attach.contains("if self.tab_active && self.activate_windows_native(false)"));
    assert!(attach.contains("cx.defer_in(window"));
    assert!(attach.contains("if this.tab_active"));
    assert!(attach.contains("this.focus_windows_native();"));
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
        ".w_full()",
        ".flex_grow(1.0)",
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
        ".flex()",
        ".flex_col()",
        ".overflow_hidden()",
    ] {
        assert!(root.contains(constraint));
    }
    assert!(content.contains(".on_prepaint(move |bounds, window, cx|"));
    assert!(content.contains("view.update_content_bounds(bounds, window.scale_factor())"));
    assert!(!root.contains(".on_children_prepainted("));
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
