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
    assert!(
        !release.contains("native.force_close"),
        "release hooks hold the entity/App borrow and must not pump COM messages"
    );
    assert!(release.contains("window.focus(&focus_handle, cx);"));
    let native_take = release
        .find("let Some(native) = this.windows_native.take()")
        .expect("release must take ownership of the native adapter");
    let registration_take = release
        .find("let registration = this.windows_native_registration.take();")
        .expect("release must take ownership of the shutdown registration");
    let event_state_take = release
        .find("this.native_event_state.take();")
        .expect("release must invalidate the native event reducer");
    let detached_marker = release
        .find("mark_windows_native_rdp_detached(")
        .expect("release must transfer the registration to detached cleanup");
    let detached_cleanup = release
        .find("detach_windows_native_cleanup(native, Some(registration), cx, \"view release\")")
        .expect("release must retain pending native cleanup on the owner thread");
    assert!(native_take < registration_take);
    assert!(registration_take < event_state_take);
    assert!(event_state_take < detached_marker);
    assert!(detached_marker < detached_cleanup);

    let cleanup = function_body(
        &view,
        "async fn cleanup_windows_native_initialization(",
        "fn detach_windows_native_cleanup(",
    );
    for token in [
        "WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT",
        "native.force_close(&mut focus_parent)",
        "NativeDestroyProgress::PendingCallbacks",
        "record_detached_windows_native_terminal",
        "WindowsRdpTerminalOutcome::Destroyed",
        "WindowsRdpTerminalOutcome::TimedOutLeaked",
        "Duration::from_millis(16)",
        "Box::leak(Box::new(native))",
    ] {
        assert!(
            cleanup.contains(token),
            "missing detached native cleanup token: {token}"
        );
    }
    assert!(!cleanup.contains("background_spawn"));
    assert!(!cleanup.contains("tokio::spawn"));
    let detached_cleanup = function_body(
        &view,
        "fn detach_windows_native_cleanup(",
        "pub struct RemoteDesktopViewConfig",
    );
    assert!(detached_cleanup.contains("cx.spawn(async move |cx|"));
    assert!(detached_cleanup.contains(".detach();"));
    let detached_terminal = function_body(
        &view,
        "fn record_detached_windows_native_terminal(",
        "fn reset_windows_native_presentation_schedule(",
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
    let force_close = cleanup
        .find("native.force_close(&mut focus_parent)")
        .expect("detached force close");
    let destroyed_branch = cleanup
        .find("NativeDestroyProgress::Destroyed) => {")
        .expect("destroyed cleanup branch");
    let destroyed_terminal = cleanup[destroyed_branch..]
        .find("WindowsRdpTerminalOutcome::Destroyed")
        .map(|offset| destroyed_branch + offset)
        .expect("destroyed terminal completion");
    let destroyed_return = cleanup[destroyed_terminal..]
        .find("return true;")
        .map(|offset| destroyed_terminal + offset)
        .expect("destroyed cleanup termination");
    let already_destroyed_branch = cleanup
        .find("Err(_) if native.is_destroyed() => {")
        .expect("already-destroyed cleanup branch");
    let already_destroyed_return = cleanup[already_destroyed_branch..]
        .find("return true;")
        .map(|offset| already_destroyed_branch + offset)
        .expect("already-destroyed cleanup termination");
    let deadline = cleanup
        .find("if Instant::now() >= deadline")
        .expect("detached cleanup deadline");
    let leak = cleanup
        .find("Box::leak(Box::new(native))")
        .expect("fail-closed adapter leak");
    let timer = cleanup
        .find("timer(Duration::from_millis(16))")
        .expect("detached cleanup retry timer");
    assert!(force_close < destroyed_branch);
    assert!(destroyed_branch < destroyed_terminal);
    assert!(destroyed_terminal < destroyed_return);
    assert!(destroyed_return < already_destroyed_return);
    assert!(already_destroyed_return < deadline);
    assert!(deadline < leak);
    assert!(leak < timer);

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
    let admit = function_body(
        &view,
        "fn admit_windows_native_presentation",
        "fn fail_presentation_initialization",
    );
    assert!(admit.contains("register_windows_native_rdp"));
    assert!(admit.contains("native.generation()"));
    assert!(admit.contains("WindowsRdpRegistrationError"));
    assert!(
        admit
            .matches("fail_unregistered_windows_native_presentation(")
            .count()
            >= 5,
        "every preparation failure and shutdown-admission rejection must still clean up its \
         native host"
    );

    let schedule = function_body(
        &view,
        "fn schedule_windows_native_presentation",
        "fn prepare_windows_native_presentation",
    );
    let admit_call = schedule
        .find("this.admit_windows_native_presentation(prepared, cx)")
        .expect("borrowed registration phase");
    let connect = schedule
        .find("native.connect")
        .expect("unborrowed connect phase");
    assert!(
        admit_call < connect,
        "the adapter must be registered for shutdown drain before Connect starts"
    );
    assert!(schedule.contains("this.attach_windows_native_presentation("));
    assert!(schedule.contains(
        "native,\n                                registration,\n                                scale_factor,"
    ));
    assert!(schedule.contains("fail_windows_native_presentation("));

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
    assert!(failure.contains("self.windows_native_display.reset();"));
    assert!(failure.contains("self.fail_presentation_initialization("));
    assert!(!failure.contains("native.force_close"));
    assert!(schedule.contains("cleanup_windows_native_initialization("));
    assert!(schedule.contains("mark_windows_native_rdp_detached_async"));

    let attach = function_body(
        &view,
        "fn attach_windows_native_presentation",
        "pub(super) fn update_windows_native_bounds",
    );
    assert!(attach.contains("registration: windows_rdp_host::WindowsRdpRegistration"));
    assert!(attach.contains("registration.generation()"));
    assert!(attach.contains("presentation.generation()"));
    assert!(attach.contains("self.windows_native_registration = Some(registration);"));
    for forbidden in [
        "activate_windows_native",
        "focus_windows_native",
        "cx.defer_in",
        "native.update_bounds",
        "native.activate",
    ] {
        assert!(
            !attach.contains(forbidden),
            "Phase 5 attach must remain pure Rust and avoid native work: {forbidden}"
        );
    }

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
        .find("detach_windows_native_cleanup(native, Some(registration), cx, \"view release\")")
        .expect("release must defer owner-thread force close");
    assert!(native_take < registration_take);
    assert!(registration_take < force_close);
    assert!(!release.contains("native.force_close"));
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
        "async fn cleanup_windows_native_initialization(",
        "fn detach_windows_native_cleanup(",
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
    assert!(poll.contains("for effect in effects"));
    assert!(poll.contains("self.apply_windows_native_ui_effect(effect)"));
    assert!(poll.contains("NativeRdpEventState::take_focus_release_pending"));
    assert!(poll.contains("self.tab_active"));
    assert!(poll.contains("self.focus_handle.clone()"));

    assert!(native.contains("pub(super) fn drain_events("));
    assert!(native.contains("state.close_confirmed()"));
}

#[test]
fn windows_native_reconnect_resets_and_reopens_the_native_presentation() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let native = include_str!("windows_native.rs").replace("\r\n", "\n");

    let presentation_reconnect = function_body(
        &native,
        "fn begin_reconnect<S: NativePresentationSink>",
        "fn begin_close<S: NativePresentationSink>",
    );
    for token in [
        "sink.focus_parent()",
        "sink.hide()",
        "self.active = false",
        "self.visible = false",
        "self.effective_visible = false",
        "self.login_complete = false",
        "self.native_child_ready = false",
        "focus_result.and(hide_result)",
    ] {
        assert!(
            presentation_reconnect.contains(token),
            "reconnect presentation reset missing {token}"
        );
    }
    assert!(
        !presentation_reconnect.contains("self.latest_bounds = None"),
        "reconnect must retain the last bounds for the next SetBounds -> Show"
    );
    assert!(
        !presentation_reconnect.contains("self.state = NativePresentationState::Closing"),
        "reconnect must keep the presentation lifecycle open"
    );

    let adapter_reconnect = function_body(
        &native,
        "pub(crate) fn begin_reconnect(",
        "pub(crate) fn refresh_native_readiness",
    );
    assert!(adapter_reconnect.contains("self.presentation.begin_reconnect(&mut sink)?"));
    assert!(adapter_reconnect.contains("focus_parent: Some(focus_parent)"));

    let effects = function_body(
        &view,
        "fn apply_windows_native_ui_effect(",
        "fn mark_windows_native_connected",
    );
    assert!(effects.contains("native.begin_reconnect"));
    assert!(effects.contains("self.native_login_complete = false"));
    assert!(effects.contains("self.windows_native_display.reconnecting(generation)"));
    assert!(effects.contains("self.present_windows_native_after_login()"));
    assert!(effects.contains("WindowsNativeFocusTarget::NativeChild"));

    let poll = function_body(
        &view,
        "fn poll_windows_native_events",
        "fn apply_windows_native_ui_effect",
    );
    assert!(poll.contains("requested_focus = Some(target)"));
    assert!(poll.contains("NativeRdpEventState::take_focus_release_pending"));
    assert!(poll.contains("Some(WindowsNativeFocusTarget::NativeChild)"));
    assert!(poll.contains("self.focus_windows_native()"));
}

#[test]
fn presentation_initialization_precedes_and_gates_canvas_runtime_start() {
    let render = include_str!("render.rs").replace("\r\n", "\n");
    let output = include_str!("output.rs").replace("\r\n", "\n");

    let ensure = render
        .find("self.ensure_presentation(window, cx);")
        .expect("presentation initialization");
    let flush = render
        .find("self.flush_pending_start(cx);")
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
fn windows_native_connect_runs_outside_the_app_borrow() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let schedule = function_body(
        &view,
        "fn schedule_windows_native_presentation",
        "fn prepare_windows_native_presentation",
    );

    let spawn = schedule
        .find("cx.spawn")
        .expect("deferred native initialization task");
    let snapshot = schedule
        .find("this.prepare_windows_native_presentation(window, initialization_generation)")
        .expect("borrowed pure-Rust snapshot phase");
    let snapshot_borrow_end = schedule
        .find("let Some(inputs) = inputs else")
        .expect("end of the borrowed snapshot phase");
    let prepare = schedule
        .find("prepare_windows_native_connection(inputs)")
        .expect("unborrowed ActiveX creation and preparation phase");
    let admit = schedule
        .find("this.admit_windows_native_presentation(prepared, cx)")
        .expect("borrowed registration phase");
    let admit_borrow_end = schedule
        .find("let (mut native, registration, connection_options) = match admission")
        .expect("end of the borrowed registration phase");
    let connect = schedule
        .find("native.connect")
        .expect("unborrowed connect phase");
    let attach = schedule
        .find("this.attach_windows_native_presentation(")
        .expect("borrowed attach phase");

    assert!(spawn < snapshot);
    assert!(snapshot < snapshot_borrow_end);
    assert!(
        snapshot_borrow_end < prepare && prepare < admit,
        "the ActiveX host creation and preparation pump Win32 messages that can dispatch \
         pending GPUI foreground tasks; they must run outside the App borrow"
    );
    assert!(admit < admit_borrow_end);
    assert!(
        admit_borrow_end < connect,
        "the ActiveX Connect call pumps COM messages that can dispatch pending GPUI \
         foreground tasks; it must run after the App borrow is released"
    );
    assert!(connect < attach);
    assert!(schedule.contains("windows_native_initialization_generation"));
    assert!(schedule.contains("reset_windows_native_presentation_schedule"));

    // The COM-stage function itself must be borrow-free: it runs outside any
    // GPUI context and only receives a pure-Rust input snapshot.
    let prepare_fn = function_body(
        &view,
        "fn prepare_windows_native_connection",
        "fn preserve_presented_frame_during_session_reset",
    );
    assert!(prepare_fn.contains("WindowsNativeAdapter::create_with_owner"));
    assert!(prepare_fn.contains("native.update_bounds"));
    assert!(prepare_fn.contains("native.apply_credentials"));
    assert!(
        !prepare_fn.contains("cx."),
        "the borrow-free COM stage must not touch any GPUI context"
    );

    // The snapshot phase must stay pure Rust: no COM calls before the borrow
    // is released.
    let snapshot_fn = function_body(
        &view,
        "fn prepare_windows_native_presentation",
        "fn admit_windows_native_presentation",
    );
    assert!(snapshot_fn.contains("parent_window_owner(window)"));
    assert!(snapshot_fn.contains("options: self.options.clone()"));
    assert!(
        !snapshot_fn.contains("windows_native_initialization_generation = None"),
        "Phase 1 must not release the initialization latch while later COM phases are running"
    );
    for token in ["create_with_owner", "update_bounds", "apply_credentials"] {
        assert!(
            !snapshot_fn.contains(token),
            "the borrowed snapshot phase must not call the COM-stage primitive: {token}"
        );
    }
}

#[test]
fn windows_native_initialization_dispatch_rejection_cannot_strand_or_leak_the_host() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let schedule = function_body(
        &view,
        "fn schedule_windows_native_presentation",
        "fn prepare_windows_native_presentation",
    );

    assert!(
        schedule.contains("reset_windows_native_presentation_schedule"),
        "a rejected Phase 1 update must clear the scheduled latch so a live view can retry"
    );
    assert!(
        schedule.contains("cleanup_rejected_windows_native_preparation"),
        "a rejected Phase 3 update must retain and clean any host created during Phase 2"
    );
    assert!(
        schedule.contains("cleanup_windows_native_initialization"),
        "Connect failure and rejected Phase 5 attach must use borrow-free native cleanup"
    );
    assert!(
        schedule.contains("mark_windows_native_rdp_detached_async"),
        "registered hosts must detach from a vanished view before borrow-free cleanup"
    );

    let cleanup = function_body(
        &view,
        "async fn cleanup_windows_native_initialization",
        "fn detach_windows_native_cleanup",
    );
    assert!(cleanup.contains("native.force_close(&mut focus_parent)"));
    assert!(
        cleanup.contains("record_detached_windows_native_terminal"),
        "borrow-free cleanup must always terminalize a registered host"
    );

    let admit = function_body(
        &view,
        "fn admit_windows_native_presentation",
        "fn fail_presentation_initialization",
    );
    assert!(
        !admit.contains("native.force_close"),
        "Phase 3 holds the App/entity borrow and must never pump native cleanup"
    );
}

#[test]
fn windows_native_attach_rejects_stale_initialization_generations() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let attach = function_body(
        &view,
        "pub(crate) fn attach_windows_native_presentation",
        "pub(super) fn update_windows_native_bounds",
    );

    assert!(
        attach.contains("presentation_initialization")
            && attach.contains("RemoteDesktopPresentationInitialization::Pending"),
        "Phase 5 must not attach after the view has reset, failed, or selected Canvas"
    );
    assert!(
        attach.contains("return Err("),
        "a stale Phase 5 payload must be returned to the borrow-free cleanup path"
    );
}

#[test]
fn windows_native_initialization_orders_create_bounds_connect_and_attach() {
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let snapshot = function_body(
        &view,
        "fn prepare_windows_native_presentation",
        "fn admit_windows_native_presentation",
    );

    let selection = snapshot
        .find("select_remote_desktop_presentation(")
        .expect("presentation selection");
    let proxy_check = snapshot
        .find("if self.options.proxy.is_some()")
        .expect("proxy capability check");
    let owner = snapshot
        .find("parent_window_owner(window)")
        .expect("native owner extraction");
    assert!(
        selection < proxy_check && proxy_check < owner,
        "selection and the unsupported SOCKS/HTTP proxy check must fail before touching the \
         native host"
    );
    let proxy_failure = function_body(
        snapshot,
        "if self.options.proxy.is_some() {",
        "let Some(bounds) = self.content_bounds",
    );
    assert!(proxy_failure.contains("self.fail_presentation_initialization("));
    assert!(
        proxy_failure.contains("RemoteDesktopPresentation::NativeWindows,\n                true,")
    );
    assert!(
        !proxy_failure.contains("RemoteDesktopPresentationInitialization::Canvas"),
        "Auto must fail closed rather than bypassing an unsupported proxy via Canvas fallback"
    );
    assert!(proxy_failure.contains("native_proxy_unsupported"));

    // Phase 2: the borrow-free COM sequence.
    let prepare = function_body(
        &view,
        "fn prepare_windows_native_connection",
        "fn preserve_presented_frame_during_session_reset",
    );
    let mut previous = 0;
    for token in [
        "WindowsNativeAdapter::create_with_owner",
        "native.update_bounds",
        "parse_destination",
        "windows_native_policy::connection_policy",
        "WindowsRdpConnectionOptions::new",
        ".with_policy(policy)",
        "windows_native_policy::apply_gateway_credentials",
        "native.apply_credentials",
    ] {
        let position = prepare[previous..]
            .find(token)
            .map(|offset| previous + offset)
            .unwrap_or_else(|| panic!("missing ordered native initialization token: {token}"));
        previous = position + token.len();
    }
    assert!(
        prepare.matches("WindowsNativePrepareFailure").count() >= 5,
        "every COM preparation failure must route its native host to cleanup"
    );

    // Phase 3: admission routes preparation failures before the host is
    // registered, and registered failures fail closed (no Canvas session).
    let admit = function_body(
        &view,
        "fn admit_windows_native_presentation",
        "fn fail_presentation_initialization",
    );
    let post_create = admit
        .find("Err(WindowsNativePrepareFailure::Bounds")
        .expect("post-create preparation failure branches");
    let shared_folders = admit
        .find("Err(WindowsNativePrepareFailure::SharedFoldersUnsupported")
        .expect("shared-folder capability branch");
    let connection_options = admit
        .find("Err(WindowsNativePrepareFailure::ConnectionOptions")
        .expect("post-create connection-options branch");
    assert!(
        !admit[post_create..shared_folders]
            .contains("RemoteDesktopPresentationInitialization::Canvas")
            && !admit[connection_options..]
                .contains("RemoteDesktopPresentationInitialization::Canvas"),
        "once a native host exists, later preparation/connect failures must not open a Canvas \
         session, except the explicit shared-folder capability fallback"
    );
    let shared_folder_branch = &admit[shared_folders..connection_options];
    for token in [
        "RemoteDesktopBackendPreference::Auto",
        "RemoteDesktopPresentationInitialization::Canvas",
        "WindowsNativeRdpUnavailableReason::SharedFoldersUnsupported",
        "self.fail_windows_native_presentation(",
        "registration: None",
        "reason: \"shared-folders\"",
    ] {
        assert!(
            shared_folder_branch.contains(token),
            "missing shared-folder fail-closed/fallback token: {token}"
        );
    }
    assert!(
        admit[post_create..]
            .matches("fail_unregistered_windows_native_presentation")
            .count()
            >= 5,
        "all post-create preparation failures must close the native host and fail closed"
    );
    let register = admit
        .find("register_windows_native_rdp")
        .expect("application shutdown registration");
    assert!(
        post_create < register,
        "every preparation failure must be routed before the host is registered"
    );
    for (offset, _) in admit.match_indices("self.fail_unregistered_windows_native_presentation(") {
        let invocation = &admit[offset..];
        let end = invocation
            .find(");")
            .expect("native initialization failure invocation");
        assert!(
            !invocation[..end].contains("cx"),
            "Phase 3 failure routing must return ownership before COM cleanup pumps messages"
        );
    }
    // Phase 4 and Phase 5 live in the schedule: connect outside the borrow,
    // then attach with the registration.
    let schedule = function_body(
        &view,
        "fn schedule_windows_native_presentation",
        "fn prepare_windows_native_presentation",
    );
    assert!(schedule.contains("cleanup_windows_native_initialization("));
    let mut previous = 0;
    for token in [
        "cx.spawn",
        "this.prepare_windows_native_presentation(window, initialization_generation)",
        "prepare_windows_native_connection(inputs)",
        "this.admit_windows_native_presentation(prepared, cx)",
        "native.connect",
        "this.attach_windows_native_presentation(",
    ] {
        let position = schedule[previous..]
            .find(token)
            .map(|offset| previous + offset)
            .unwrap_or_else(|| panic!("missing ordered deferred native token: {token}"));
        previous = position + token.len();
    }
    let attach = function_body(
        &view,
        "fn attach_windows_native_presentation",
        "pub(super) fn update_windows_native_bounds",
    );
    assert!(attach.contains("RemoteDesktopPresentationInitialization::Native"));
}

#[test]
fn windows_native_fixed_mode_keeps_hwnd_bounds_but_skips_protocol_display_updates() {
    let output = include_str!("output.rs").replace("\r\n", "\n");
    let integration = include_str!("windows_native_display_integration.rs").replace("\r\n", "\n");

    let update_bounds = function_body(
        &output,
        "pub(super) fn update_content_bounds",
        "pub(super) fn flush_pending_start",
    );
    let native_bounds = update_bounds
        .find("self.update_windows_native_bounds(bounds, display_scale_factor)")
        .expect("native HWND bounds update");
    let observe = update_bounds
        .find("self.observe_windows_native_viewport(bounds, display_scale_factor)")
        .expect("native protocol display observation");
    assert!(
        native_bounds < observe,
        "native child HWND bounds must update before protocol-level display handling"
    );

    let observe = function_body(
        &integration,
        "pub(super) fn observe_windows_native_viewport",
        "pub(super) fn flush_windows_native_display_settings",
    );
    let fixed_guard = observe
        .find("windows_native_policy::uses_dynamic_display_updates")
        .expect("fixed desktop mode guard");
    let physical_conversion = observe
        .find("logical_bounds_to_physical")
        .expect("dynamic viewport conversion");
    assert!(
        fixed_guard < physical_conversion,
        "fixed desktop mode must stop protocol display updates before observing viewport changes"
    );
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
    assert!(retry.contains("self.failure_detail = None;"));
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
    assert!(failure.contains("self.windows_native_display.reset();"));
    assert!(failure.contains("self.fail_presentation_initialization("));
    assert!(failure.contains("RemoteDesktopPresentation::NativeWindows"));
    assert!(!failure.contains("native.force_close"));
    let cleanup = function_body(
        &view,
        "async fn cleanup_windows_native_initialization(",
        "fn detach_windows_native_cleanup(",
    );
    assert!(cleanup.contains("native.force_close(&mut focus_parent)"));
    assert!(cleanup.contains("NativeDestroyProgress::Destroyed"));
    assert!(cleanup.contains("NativeDestroyProgress::PendingCallbacks"));
    assert!(cleanup.contains("WindowsRdpTerminalOutcome::TimedOutLeaked"));
    assert!(cleanup.contains("Box::leak(Box::new(native))"));
    let completion = function_body(
        &view,
        "fn complete_windows_native_initialization_cleanup(",
        "pub(crate) fn attach_windows_native_presentation",
    );
    assert!(completion.contains("canvas_retry_available: destroyed"));

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
    assert!(source.contains("view.update_content_bounds(bounds, window.scale_factor(), view_cx)"));
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

    // Phase 5 only installs Rust ownership/state. LoginComplete/Reconnected
    // perform native synchronization after the entity borrow has ended.
    for forbidden in [
        "self.native_login_complete",
        "self.activate_windows_native",
        "cx.defer_in",
        "self.focus_windows_native",
    ] {
        assert!(
            !attach.contains(forbidden),
            "attach must not enter the native presentation path: {forbidden}"
        );
    }
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
        .find("show_empty_status && (!uses_windows_native || show_failure_detail)")
        .expect("empty-frame status")..];
    assert!(status.contains(".min_w_0()"));
    assert!(status.contains(".max_w_full()"));
    assert!(status.contains(".flex_shrink_0()"));
    assert!(status.contains(".overflow_hidden()"));
    assert!(status.contains(".whitespace_normal()"));
    assert!(status.contains(".overflow_scrollbar()"));
    assert!(status.contains("remote-desktop-copy-diagnostic"));
    assert!(status.contains("ClipboardItem::new_string"));
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
    assert!(content.contains("view.update_content_bounds(bounds, window.scale_factor(), view_cx)"));
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

#[test]
fn windows_native_presentation_readiness_is_independent_of_canvas_frames() {
    let native = include_str!("windows_native.rs").replace("\r\n", "\n");
    let view = include_str!("../view.rs").replace("\r\n", "\n");
    let maintenance = function_body(
        &view,
        "let output_poll_task = cx.spawn",
        "cx.on_release(move |this, cx|",
    );
    let overlay = include_str!("windows_native_overlay/lifecycle.rs").replace("\r\n", "\n");
    let capability = include_str!("presentation_capability.rs").replace("\r\n", "\n");

    for token in [
        "fn mark_login_complete",
        "fn set_native_child_ready",
        "fn set_effective_visible",
        "fn can_present",
        "fn presentation_ready",
        "fn activation_pending",
        "effective_visible",
        "native_child_ready",
        "login_complete",
        "fn is_effectively_visible",
    ] {
        assert!(native.contains(token), "windows_native.rs missing {token}");
    }
    // The readiness gate must sit inside activate() so a deferred activate
    // emits no sink commands.
    let activate = function_body(
        &native,
        "fn activate<S: NativePresentationSink>",
        "fn focus<S: NativePresentationSink>",
    );
    assert!(activate.contains("if !self.can_present()"));
    assert!(activate.contains("return Ok(());"));

    for token in [
        "fn present_windows_native_after_login",
        "fn synchronize_windows_native_presentation",
        "fn refresh_windows_native_readiness",
        "fn log_windows_native_readiness",
        "native_login_complete",
        "presentation ready",
    ] {
        assert!(view.contains(token), "view.rs missing {token}");
    }
    // LoginComplete/Reconnected must force a re-synchronization of bounds.
    let login_complete = function_body(
        &view,
        "fn present_windows_native_after_login",
        "fn synchronize_windows_native_presentation",
    );
    assert!(login_complete.contains("synchronize_windows_native_presentation()"));
    // The view must propagate the login phase into the adapter; keeping only
    // the view-local bool would leave the activate() readiness gate closed.
    assert!(login_complete.contains("native.mark_login_complete()"));
    // The maintenance task re-reads readiness every tick.
    assert!(maintenance.contains("refresh_windows_native_readiness()"));
    let readiness = function_body(
        &view,
        "fn refresh_windows_native_readiness",
        "fn log_windows_native_readiness",
    );
    assert!(readiness.contains("native.activation_pending()"));
    // The overlay exposes actual HWND visibility, never canvas frame readiness.
    assert!(overlay.contains("fn is_actually_visible"));
    assert!(overlay.contains("IsWindowVisible"));
    // The Windows-only probe mapping must stay exhaustive as the host error
    // enum grows; this source check runs on every platform.
    assert!(capability.contains("WindowsRdpHostError::PresentationIncomplete =>"));
    assert!(!view.contains("rendered_frames.promote()\n            && self.rendered_frames"));
}
