fn function_region<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split(start)
        .nth(1)
        .and_then(|source| source.split(end).next())
        .unwrap_or_else(|| panic!("missing function region between `{start}` and `{end}`"))
}

fn assert_function_guard_precedes(
    source: &str,
    start: &str,
    end: &str,
    guard: &str,
    side_effect: &str,
    capability: &str,
) {
    assert_guard_precedes(
        function_region(source, start, end),
        guard,
        side_effect,
        capability,
    );
}

#[test]
fn playback_never_registers_or_uses_ssh_broadcast_input() {
    let source = include_str!("../registrations.rs");
    let registration = function_region(
        source,
        "fn register_broadcast_input",
        "fn unregister_broadcast_input",
    );
    assert!(
        registration.contains("live_ssh_feature_supported(terminal.live_connection_kind())"),
        "broadcast registration must authorize the live SSH capability, not source metadata"
    );

    let broadcast = function_region(
        source,
        "fn broadcast_input",
        "fn refresh_public_mcp_session",
    );
    let guard = broadcast
        .find("self.is_live_ssh_terminal(cx)")
        .expect("broadcast sender must have a live SSH guard");
    let delivery = broadcast
        .find("deliveries_from")
        .expect("broadcast delivery lookup should remain");
    assert!(guard < delivery, "the live SSH guard must precede delivery");
}

#[test]
fn typed_terminal_input_preserves_operation_kind_without_rebroadcasting_deliveries() {
    let source = include_str!("../text_input.rs");

    for (start, end, expected_kind) in [
        (
            "pub(super) fn write_to_pty",
            "pub(super) fn write_paste_to_pty",
            "TerminalInputKind::UserInput",
        ),
        (
            "pub(super) fn write_paste_to_pty",
            "pub(super) fn write_control_sequence_to_pty",
            "TerminalInputKind::Paste",
        ),
        (
            "pub(super) fn write_control_sequence_to_pty",
            "fn write_to_pty_with_kind",
            "TerminalInputKind::ControlSequence",
        ),
    ] {
        assert!(
            function_region(source, start, end).contains(expected_kind),
            "{start} must preserve {expected_kind}"
        );
    }

    let dispatch = function_region(
        source,
        "pub(super) fn write_input_to_terminal",
        "pub(super) fn commit_text",
    );
    assert!(dispatch.contains("TerminalInputKind::UserInput => terminal.write(data)"));
    assert!(dispatch.contains("TerminalInputKind::Paste => terminal.write_paste(data)"));
    assert!(
        dispatch.contains(
            "TerminalInputKind::ControlSequence => terminal.write_control_sequence(data)"
        )
    );

    let delivery = function_region(
        source,
        "pub(super) fn write_broadcast_input",
        "pub(super) fn write_input_to_terminal",
    );
    assert!(delivery.contains("self.write_input_to_terminal(kind, &data, cx)"));
    assert!(
        !delivery.contains("self.broadcast_input"),
        "a broadcast delivery must not be broadcast again"
    );
}

#[test]
fn playback_reconnect_is_rejected_before_connection_storage_access() {
    let source = include_str!("../preferences.rs");
    let reconnect = function_region(source, "pub fn reconnect", "pub fn sync_terminal_path");
    let guard = reconnect
        .find("if !self.accepts_live_terminal_input(cx)")
        .expect("reconnect must reject playback");
    let load = reconnect
        .find("resolve_ssh_reconnect_source")
        .expect("SSH reconnect source resolution should remain");
    assert!(
        guard < load,
        "playback must be rejected before reading connection storage"
    );
}

#[test]
fn playback_duplicate_is_rejected_at_view_and_workspace_execution_boundaries() {
    let view_tab = include_str!("../tab_content.rs");
    let workspace_tab = include_str!("../../workspace/tab_content.rs");
    let support = include_str!("../workspace_support.rs");

    assert!(
        function_region(view_tab, "fn duplicate(", "fn try_close")
            .contains("if !self.duplicate_supported(cx)")
    );
    assert!(
        function_region(workspace_tab, "fn duplicate(", "fn try_close")
            .contains("if !self.active_pane().read(cx).duplicate_supported(cx)")
    );
    assert!(support.contains("pub(crate) fn duplicate_supported(&self, cx: &App) -> bool"));
    assert!(support.contains("self.terminal.read(cx).live_connection_kind()"));
}

#[test]
fn playback_pty_and_ime_writes_are_rejected_before_side_effects() {
    let source = include_str!("../text_input.rs");

    for (start, end, side_effect, capability) in [
        (
            "pub(super) fn write_to_pty",
            "pub(super) fn write_broadcast_input",
            "self.write_input_to_terminal",
            "PTY writes",
        ),
        (
            "pub(super) fn write_input_to_terminal",
            "pub(super) fn commit_text",
            "grid().display_offset()",
            "terminal input",
        ),
        (
            "pub(super) fn commit_text",
            "pub(super) fn set_marked_text",
            "apply_inline_input_to_history_prompt",
            "IME commits",
        ),
        (
            "pub(super) fn set_marked_text",
            "pub(super) fn clear_marked_text",
            "self.ime_state =",
            "IME composition",
        ),
    ] {
        assert_function_guard_precedes(
            source,
            start,
            end,
            "if !self.accepts_live_terminal_input(cx)",
            side_effect,
            capability,
        );
    }
}

#[test]
fn playback_key_handler_retains_copy_before_rejecting_input() {
    let source = include_str!("../text_input.rs");
    let key_event = source
        .split("pub(super) fn handle_key_event")
        .nth(1)
        .expect("key event handler should exist");
    let copy = key_event
        .find("action_id::TERMINAL_COPY")
        .expect("copy shortcut should remain available");
    let guard = key_event
        .find("if !self.accepts_live_terminal_input(cx)")
        .expect("key input must reject playback");
    let paste = key_event
        .find("action_id::TERMINAL_PASTE")
        .expect("paste shortcut should remain for live terminals");
    let input_side_effect = key_event
        .find("BlinkCursor::pause")
        .expect("live input should still pause cursor blink");
    assert!(
        copy < guard && guard < paste && guard < input_side_effect,
        "copy must remain available before playback is rejected, while paste and input state \
         changes must remain behind the live capability gate"
    );
}

#[test]
fn playback_paste_is_rejected_before_clipboard_or_prompt_side_effects() {
    let source = include_str!("../clipboard.rs");

    assert_function_guard_precedes(
        source,
        "pub(super) fn paste(",
        "pub(super) fn increase_font",
        "if !self.accepts_live_terminal_input(cx)",
        "read_from_clipboard",
        "clipboard paste",
    );

    assert_function_guard_precedes(
        source,
        "pub(super) fn paste_text(",
        "pub(super) fn paste_text_unchecked",
        "if !self.accepts_live_terminal_input(cx)",
        "normalize_paste_line_endings",
        "checked text paste",
    );

    let paste_unchecked = source
        .split("pub(super) fn paste_text_unchecked")
        .nth(1)
        .expect("unchecked paste implementation should exist");
    assert_guard_precedes(
        paste_unchecked,
        "if !self.accepts_live_terminal_input(cx)",
        "normalize_paste_line_endings",
        "unchecked text paste",
    );
}

#[test]
fn playback_never_starts_or_completes_sftp_clipboard_uploads() {
    let source = include_str!("../clipboard_image.rs");

    for (start, end, side_effect, capability) in [
        (
            "pub(super) fn paste_clipboard_image_to_remote_cli",
            "pub(super) fn spawn_clipboard_image_upload",
            ".ssh_config()",
            "clipboard image upload",
        ),
        (
            "pub(super) fn spawn_clipboard_image_upload",
            "pub(super) fn handle_clipboard_image_upload_result",
            "Tokio::spawn",
            "SFTP upload task",
        ),
        (
            "pub(super) fn paste_remote_image_path",
            "pub(super) fn paste_code_block",
            "apply_paste_to_history_prompt",
            "uploaded remote path paste",
        ),
    ] {
        assert_function_guard_precedes(
            source,
            start,
            end,
            "if !self.is_live_ssh_terminal(cx)",
            side_effect,
            capability,
        );
    }
}

#[test]
fn playback_history_assistance_never_starts_sftp_completion() {
    let source = include_str!("../history_query.rs");
    assert_function_guard_precedes(
        source,
        "pub(super) fn history_prompt_enabled",
        "pub(super) fn refresh_history_prompt_matches",
        "let Some(connection_kind) = terminal.live_connection_kind()",
        "terminal.mode()",
        "history prompt availability",
    );
    assert_function_guard_precedes(
        source,
        "pub(super) fn refresh_cd_completion_matches",
        "\n}",
        "if !self.is_live_ssh_terminal(cx)",
        "cd_completion_cache.get",
        "SFTP directory completion",
    );

    let query = function_region(
        source,
        "pub(super) fn current_cd_completion_query",
        "pub(super) fn refresh_cd_completion_matches",
    );
    assert!(
        query.contains("live_ssh_feature_supported(terminal.live_connection_kind())"),
        "cd completion queries must authorize live SSH instead of recording source metadata"
    );
}

#[test]
fn playback_view_initialization_does_not_attach_live_workspace_or_ssh_resources() {
    let source = include_str!("../initialization.rs");
    let initialization = function_region(source, "pub(super) fn new_with_terminal", "\n}");

    assert!(
        initialization
            .contains("let live_connection_kind = terminal.read(cx).live_connection_kind()")
    );
    assert!(initialization.contains(
        "let is_local_terminal = live_connection_kind == Some(TerminalConnectionKind::Local)"
    ));
    let ssh_gate = initialization
        .find("if live_ssh_feature_supported(live_connection_kind)")
        .expect("SSH resources must be authorized by the live SSH capability");
    let ssh_config = initialization
        .find(".ssh_config()")
        .expect("live SSH initialization should retain SSH configuration");
    let ssh_manager = initialization
        .find(".ssh_session_manager()")
        .expect("live SSH initialization should retain its session manager");
    assert!(ssh_gate < ssh_config && ssh_gate < ssh_manager);
    assert!(initialization.contains("terminal_history_scope(live_connection_kind, connection_id)"));
    assert!(initialization.contains("if live_connection_kind.is_some()"));
}

#[test]
fn playback_has_dedicated_read_only_view_and_workspace_constructors() {
    let view = include_str!("../constructors.rs");
    let workspace = include_str!("../../workspace/view.rs");

    let view_constructor =
        function_region(view, "pub fn new_recording_playback", "pub fn new_serial");
    assert!(view_constructor.contains("Terminal::new_recording_playback(playback, cx)"));
    assert!(view_constructor.contains("duplicate_source: None"));
    assert!(view_constructor.contains("recording_playback_name: Some(display_name)"));

    let workspace_constructor = function_region(
        workspace,
        "pub fn new_recording_playback",
        "pub(super) fn from_pane",
    );
    assert!(workspace_constructor.contains("TerminalView::new_recording_playback"));
    assert!(workspace_constructor.contains("with_workspace_pane()"));
}

#[test]
fn playback_duplicate_and_reconnect_require_an_owned_live_source() {
    let support = include_str!("../workspace_support.rs");
    let reconnect = include_str!("../preferences.rs");

    assert!(support.contains("pub(crate) fn duplicate_source_snapshot"));
    assert!(support.contains("self.duplicate_source.clone()?"));
    assert!(reconnect.contains("let Some(duplicate_source) = self.duplicate_source.as_ref()"));
}

#[test]
fn playback_command_bar_events_are_rejected_before_parsing_or_paste() {
    let source = include_str!("../command_bar_events.rs");
    for (start, end, side_effect, capability) in [
        (
            "TerminalCommandBarEvent::Submit(command) =>",
            "TerminalCommandBarEvent::InputToPty(command) =>",
            "command_batch_lines",
            "command submission",
        ),
        (
            "TerminalCommandBarEvent::InputToPty(command) =>",
            "TerminalCommandBarEvent::FocusTerminal",
            "self.paste_text",
            "command bar PTY input",
        ),
    ] {
        assert_function_guard_precedes(
            source,
            start,
            end,
            "if !self.accepts_live_terminal_input(cx)",
            side_effect,
            capability,
        );
    }
}

#[test]
fn playback_action_input_paths_are_rejected_before_local_side_effects() {
    let tool_dock = include_str!("../tool_dock.rs");
    for (start, end, side_effect, capability) in [
        (
            "pub(super) fn send_tab",
            "pub(super) fn send_shift_tab",
            "try_accept_explicit_history_prompt",
            "send-tab action",
        ),
        (
            "pub(super) fn send_shift_tab",
            "pub(super) fn render_sidebar_resize_handle",
            "dismiss_history_prompt",
            "send-shift-tab action",
        ),
    ] {
        assert_function_guard_precedes(
            tool_dock,
            start,
            end,
            "if !self.accepts_live_terminal_input(cx)",
            side_effect,
            capability,
        );
    }

    let selection = include_str!("../selection_search.rs");
    assert_function_guard_precedes(
        selection,
        "pub(super) fn clear_screen",
        "pub(super) fn reset_render_cache",
        "if !self.accepts_live_terminal_input(cx)",
        "self.clear_history_prompt",
        "clear-screen action",
    );

    let vi_input = include_str!("../vi_input.rs");
    assert_function_guard_precedes(
        vi_input,
        "pub(super) fn handle_vi_key_event",
        "pub(super) fn vi_start_selection",
        "if !self.accepts_live_terminal_input(cx)",
        "term.vi_motion",
        "VI key input",
    );
    assert_function_guard_precedes(
        vi_input,
        "pub(super) fn toggle_vi_mode",
        "\n}",
        "if !self.accepts_live_terminal_input(cx)",
        "terminal.toggle_vi_mode",
        "toggle-VI action",
    );
}

#[test]
fn playback_clear_selection_never_changes_recorded_vi_mode() {
    let source = include_str!("../selection_search.rs");
    let clear_selection = function_region(
        source,
        "pub(super) fn clear_selection",
        "pub(super) fn search_forward",
    );
    let capability = clear_selection
        .find("let accepts_live_input = self.accepts_live_terminal_input(cx);")
        .expect("clear selection must inspect the live input capability");
    let live_only_branch = clear_selection
        .find("else if accepts_live_input")
        .expect("leaving VI mode must be restricted to live terminals");
    let toggle = clear_selection
        .find("term_lock.toggle_vi_mode()")
        .expect("live terminals should retain clear-selection VI behavior");
    assert!(capability < live_only_branch && live_only_branch < toggle);
}

#[test]
fn playback_mouse_reporting_is_rejected_before_report_side_effects() {
    let source = include_str!("../scroll.rs");

    assert_function_guard_precedes(
        source,
        "pub(super) fn try_report_sgr_mouse_button",
        "pub(super) fn write_sgr_mouse_button_report",
        "if !self.accepts_live_terminal_input(cx)",
        "self.terminal.read(cx).mode()",
        "SGR mouse reporting",
    );
    assert_function_guard_precedes(
        source,
        "pub(super) fn write_sgr_mouse_button_report",
        "\n}",
        "if !self.accepts_live_terminal_input(cx)",
        "mouse_button_code(button)",
        "SGR mouse report construction",
    );
}

#[test]
fn playback_mouse_paste_handlers_reject_before_reporting_or_ui_side_effects() {
    let source = include_str!("../mouse_down.rs");

    assert_function_guard_precedes(
        source,
        "pub(super) fn handle_middle_mouse_down",
        "pub(super) fn handle_right_mouse_down",
        "if !self.accepts_live_terminal_input(cx)",
        "self.try_report_sgr_mouse_button",
        "middle-click reporting and paste",
    );
    assert_function_guard_precedes(
        source,
        "pub(super) fn handle_right_mouse_down",
        "\n}",
        "if !self.accepts_live_terminal_input(cx)",
        "cx.stop_propagation()",
        "right-click paste",
    );
}

#[test]
fn playback_stale_sgr_press_never_clears_the_local_selection() {
    let source = include_str!("../mouse_selection.rs");
    let mouse_up = function_region(
        source,
        "pub(super) fn handle_mouse_up",
        "pub(super) fn handle_window_mouse_up",
    );
    let pending_press = mouse_up
        .split("if let Some(pending)")
        .nth(1)
        .expect("mouse-up should retain pending SGR press handling");

    assert_guard_precedes(
        pending_press,
        "if self.accepts_live_terminal_input(cx)",
        "terminal.clear_selection()",
        "pending SGR mouse press",
    );
}

#[test]
fn recorded_local_metadata_never_authorizes_local_path_addons() {
    for (source, expected_live_checks, capability) in [
        (
            include_str!("../terminal_render.rs"),
            1,
            "terminal frame addons",
        ),
        (include_str!("../mouse_down.rs"), 1, "mouse-down addons"),
        (
            include_str!("../mouse_selection.rs"),
            2,
            "mouse move/up addons",
        ),
    ] {
        assert!(
            !source.contains(".connection_kind() == TerminalConnectionKind::Local"),
            "{capability} must not authorize local paths from recording source metadata"
        );
        assert!(
            source.matches("live_connection_kind()").count() >= expected_live_checks,
            "{capability} must authorize local paths from the live connection kind"
        );
    }
}

fn assert_guard_precedes(source: &str, guard: &str, side_effect: &str, capability: &str) {
    let guard_position = source
        .find(guard)
        .unwrap_or_else(|| panic!("{capability} must have a live capability guard"));
    let side_effect_position = source
        .find(side_effect)
        .unwrap_or_else(|| panic!("{capability} side effect `{side_effect}` should remain"));
    assert!(
        guard_position < side_effect_position,
        "{capability} must reject playback before `{side_effect}`"
    );
}
