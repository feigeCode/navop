use super::*;

#[test]
fn pending_host_key_confirmation_suppresses_connection_overlay() {
    for connection_state in [
        ConnectionState::Disconnected { error: None },
        ConnectionState::Connecting,
    ] {
        assert!(!should_show_connection_overlay(&connection_state, true));
    }
}

#[test]
fn connection_overlay_returns_after_host_key_confirmation_finishes() {
    for connection_state in [
        ConnectionState::Disconnected { error: None },
        ConnectionState::Connecting,
    ] {
        assert!(should_show_connection_overlay(&connection_state, false));
    }
    assert!(!should_show_connection_overlay(
        &ConnectionState::Connected,
        false
    ));
}

#[test]
fn disconnected_terminal_uses_a_non_blocking_status_banner() {
    assert_eq!(
        Some(ConnectionStatusPresentation::Banner),
        connection_status_presentation(
            &ConnectionState::Disconnected { error: None },
            false,
            false,
            false,
        )
    );
    assert_eq!(
        Some(ConnectionStatusPresentation::Banner),
        connection_status_presentation(&ConnectionState::Connecting, false, false, false)
    );
}

#[test]
fn terminal_credentials_and_ssh_mfa_keep_the_blocking_connection_dialog() {
    assert_eq!(
        Some(ConnectionStatusPresentation::Dialog),
        connection_status_presentation(&ConnectionState::Connecting, false, true, false)
    );
    assert_eq!(
        Some(ConnectionStatusPresentation::Dialog),
        connection_status_presentation(&ConnectionState::Connecting, false, false, true)
    );
    assert_eq!(
        Some(ConnectionStatusPresentation::Dialog),
        connection_status_presentation(&ConnectionState::Connecting, false, true, true)
    );
}

#[test]
fn host_key_confirmation_and_connected_state_hide_connection_status() {
    assert_eq!(
        None,
        connection_status_presentation(
            &ConnectionState::Disconnected { error: None },
            true,
            true,
            true,
        )
    );
    assert_eq!(
        None,
        connection_status_presentation(&ConnectionState::Connected, false, true, true)
    );
}

#[test]
fn connection_status_rendering_does_not_restore_the_full_screen_backdrop() {
    let source = include_str!("../connection_overlay.rs");

    assert!(source.contains("render_connection_banner"));
    assert!(source.contains("render_connection_dialog"));
    assert!(
        !source.contains(".bg(Hsla {"),
        "ordinary reconnect feedback must not cover the terminal with a dark backdrop"
    );
}

#[test]
fn connection_error_banner_shows_scrollable_multiline_details() {
    let source = include_str!("../connection_overlay.rs");
    let error_block = source
        .split(".when_some(error_msg")
        .nth(1)
        .expect("connection overlay should render an error block");

    assert!(
        error_block.contains(".whitespace_normal()"),
        "terminal connection error details should wrap instead of staying on one line"
    );
    assert!(
        error_block.contains(".overflow_scrollbar()"),
        "long terminal connection error details, including unbroken tokens, should remain inspectable"
    );
    assert!(
        error_block.contains(".max_h(px("),
        "the scrollable terminal connection error area should have a bounded height"
    );
    assert!(
        !error_block.contains(".truncate()"),
        "terminal connection error details must not be visually truncated"
    );
}

#[test]
fn credential_dialog_supports_ssh_and_telnet_runtime_prompts() {
    let source = include_str!("../connection_overlay.rs");

    assert!(source.contains("SshSession.credentials_required"));
    assert!(source.contains("TelnetSession.credentials_required"));
    assert!(source.contains("submit-terminal-credentials"));
    assert!(source.contains("submit_credentials"));
}

#[test]
fn reconnect_success_reports_that_a_new_remote_shell_was_opened() {
    let terminal_events = include_str!("../terminal_events.rs");
    let locales = include_str!("../../../locales/terminal_view.yml");

    assert!(terminal_events.contains("SshSession.reconnected_new_shell"));
    assert!(locales.contains("reconnected_new_shell:"));
    assert!(locales.contains("已重新连接并打开新的远端 Shell"));
}

#[test]
fn reconnect_follow_up_is_only_armed_after_an_ssh_reconnect_starts() {
    assert_eq!(
        (false, false),
        reconnect_follow_up_state(false, TerminalConnectionKind::Ssh)
    );
    assert_eq!(
        (true, false),
        reconnect_follow_up_state(true, TerminalConnectionKind::Serial)
    );
    assert_eq!(
        (true, true),
        reconnect_follow_up_state(true, TerminalConnectionKind::Ssh)
    );
}

#[test]
fn initial_connecting_status_does_not_claim_that_it_is_reconnecting() {
    let overlay = include_str!("../connection_overlay.rs");
    let locales = include_str!("../../../locales/terminal_view.yml");

    assert!(overlay.contains("SshSession.connecting_preserves_terminal"));
    assert!(overlay.contains("SshSession.reconnecting_preserves_terminal"));
    assert!(locales.contains("connecting_preserves_terminal:"));
}

#[test]
fn terminal_tools_sidebar_defaults_to_a_roomier_width() {
    assert_eq!(px(400.0), TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH);
}

#[test]
fn command_history_changed_refreshes_history_command_panel() {
    assert!(should_refresh_history_commands_for_terminal_event(
        &TerminalModelEvent::CommandHistoryChanged
    ));
    assert!(!should_refresh_history_commands_for_terminal_event(
        &TerminalModelEvent::Wakeup
    ));
}
#[test]
fn terminal_close_confirmation_is_only_for_local_terminals() {
    for kind in [TerminalConnectionKind::Ssh, TerminalConnectionKind::Serial] {
        assert!(!should_confirm_local_terminal_close(
            Some(kind),
            true,
            TermMode::ALT_SCREEN,
            None,
        ));
    }
    assert!(!should_confirm_local_terminal_close(
        None,
        true,
        TermMode::ALT_SCREEN,
        None,
    ));
}

#[test]
fn take_whole_scroll_lines_preserves_fractional_remainder() {
    let mut accumulated = 0.4;
    assert_eq!(take_whole_scroll_lines(&mut accumulated), 0);
    assert!((accumulated - 0.4).abs() < f32::EPSILON);

    accumulated += 0.8;
    assert_eq!(take_whole_scroll_lines(&mut accumulated), 1);
    assert!((accumulated - 0.2).abs() < 0.0001);
}

#[test]
fn take_whole_scroll_lines_handles_negative_accumulation() {
    let mut accumulated = -0.45;
    assert_eq!(take_whole_scroll_lines(&mut accumulated), 0);
    assert!((accumulated + 0.45).abs() < f32::EPSILON);

    accumulated -= 0.8;
    assert_eq!(take_whole_scroll_lines(&mut accumulated), -1);
    assert!((accumulated + 0.25).abs() < 0.0001);
}

#[test]
fn terminal_keybindings_bind_ctrl_zero_to_reset_font() {
    let source = include_str!("../keybindings.rs");

    assert!(source.contains(r#"terminal_platform_shortcut("cmd-0", "ctrl-0")"#));
    assert!(source.contains("ResetFont"));
}

#[test]
fn terminal_keybindings_bind_clear_screen_shortcut() {
    let source = include_str!("../keybindings.rs");

    assert!(source.contains("TERMINAL_CLEAR_SCREEN_SHORTCUT"));
    assert!(source.contains("ClearScreen"));
}

#[test]
fn terminal_context_menu_exposes_clear_screen() {
    let source = include_str!("../terminal_render.rs");

    assert!(source.contains("ContextMenu.clear_screen_with_shortcut"));
    assert!(source.contains("this.clear_screen(&ClearScreen, window, cx)"));
}

#[test]
fn terminal_context_menu_pastes_selected_text_through_safe_paste_path() {
    let source = include_str!("../terminal_render.rs");
    let context_menu = source
        .split("pub(super) fn build_context_menu")
        .nth(1)
        .expect("terminal context menu should exist");
    let paste_selection = context_menu
        .split("ContextMenu.paste_selection")
        .nth(1)
        .expect("paste-selection item should exist");
    let regular_paste = paste_selection
        .find("ContextMenu.paste_with_shortcut")
        .expect("paste-selection should be placed before clipboard paste");
    let paste_selection = &paste_selection[..regular_paste];

    assert!(paste_selection.contains(".disabled(!can_paste_selection)"));
    assert!(paste_selection.contains("this.paste_text(&selection_text, window, cx)"));
}

#[test]
fn terminal_context_menu_disables_live_actions_during_playback() {
    let source = include_str!("../terminal_render.rs");
    let context_menu = source
        .split("pub(super) fn build_context_menu")
        .nth(1)
        .expect("terminal context menu should exist");
    let paste = context_menu
        .find("ContextMenu.paste_with_shortcut")
        .expect("paste item should remain available for live terminals");
    let clear_screen = context_menu
        .find("ContextMenu.clear_screen_with_shortcut")
        .expect("clear-screen item should remain available for live terminals");
    let paste_gate = context_menu[paste..clear_screen]
        .find(".disabled(!accepts_live_input)")
        .expect("paste must be disabled without a live input capability");
    let select_all = context_menu
        .find("ContextMenu.select_all_with_shortcut")
        .expect("select-all should remain available during playback");
    let clear_screen_gate = context_menu[clear_screen..select_all]
        .find(".disabled(!accepts_live_input)")
        .expect("clear screen must be disabled without a live input capability");

    assert!(paste_gate > 0);
    assert!(clear_screen_gate > 0);
}

#[test]
fn terminal_right_click_paste_is_only_enabled_for_live_input() {
    let source = include_str!("../render_surface.rs");
    let viewport_state = source
        .split("fn terminal_viewport_state")
        .nth(1)
        .and_then(|source| source.split("fn render_terminal_core").next())
        .expect("terminal viewport state should exist");

    assert!(
        viewport_state.contains("right_click_paste: self.right_click_paste && accepts_live_input")
    );
}

#[test]
fn playback_alt_screen_history_can_show_a_local_scrollbar() {
    let source = include_str!("../render_surface.rs");
    let viewport_state = source
        .split("fn terminal_viewport_state")
        .nth(1)
        .and_then(|source| source.split("fn render_terminal_core").next())
        .expect("terminal viewport state should exist");
    let scrollbar_policy = viewport_state
        .split("show_scrollbar:")
        .nth(1)
        .and_then(|source| source.split(",\n").next())
        .expect("terminal scrollbar policy should exist");

    assert!(
        scrollbar_policy.contains("!accepts_live_input"),
        "playback must ignore recorded alt-screen mode when exposing local scrollback"
    );
    assert!(scrollbar_policy.contains("!terminal_mode.contains(TermMode::ALT_SCREEN)"));
    assert!(scrollbar_policy.contains("history_size > 0"));
}

#[test]
fn terminal_tools_are_not_exposed_as_external_sidebar_contributions() {
    let source = include_str!("../tab_content.rs");
    let sidebar_contributions = source
        .split("fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution>")
        .nth(1)
        .expect("terminal sidebar_contributions override should exist");

    assert!(sidebar_contributions.contains("Vec::new()"));
    assert!(!source.contains("terminal.toolbar"));
    assert!(!source.contains("terminal.ai-chat"));
    assert!(!source.contains("TerminalSidebarRenderMode"));
    assert!(!source.contains("sidebar_render_mode"));
    assert!(!source.contains("with_external_sidebar"));
}

#[test]
fn terminal_render_owns_internal_tool_dock_regions() {
    let render_source = include_str!("../render_layout.rs");

    assert!(render_source.contains("terminal-tool-dock-root"));
    assert!(render_source.contains("terminal-tool-dock-left"));
    assert!(render_source.contains("terminal-tool-dock-center"));
    assert!(render_source.contains("terminal-tool-dock-right"));
    assert!(render_source.contains("terminal-tool-dock-bottom"));
    assert!(render_source.contains("terminal-tool-dock-toolbar"));
    assert!(render_source.contains(".child(self.sidebar_toolbar.clone())"));
    assert!(render_source.contains("right_tool_region_width(&layout, sidebar_size)"));
}

#[test]
fn terminal_internal_tool_dock_uses_fixed_host_bounds() {
    let render_source = include_str!("../render_layout.rs");

    assert!(render_source.contains(".w(state.width)"));
    assert!(render_source.contains(".min_w(state.width)"));
    assert!(render_source.contains(".max_w(state.width)"));
    assert!(render_source.matches(".w(sidebar_size)").count() >= 2);
    assert!(render_source.matches(".min_w(sidebar_size)").count() >= 2);
    assert!(render_source.matches(".max_w(sidebar_size)").count() >= 2);
    assert!(render_source.contains(".h(sidebar_size)"));
    assert!(render_source.contains(".min_h(sidebar_size)"));
    assert!(render_source.contains(".max_h(sidebar_size)"));
    assert!(render_source.contains(".min_w(TOOLBAR_WIDTH)"));
    assert!(render_source.contains(".max_w(TOOLBAR_WIDTH)"));
}

#[test]
fn terminal_internal_dock_keeps_bottom_inside_center_column() {
    let render_source = include_str!("../render_layout.rs");

    let root = render_source
        .find("terminal-tool-dock-root")
        .expect("root dock marker should exist");
    let center = render_source
        .find("terminal-tool-dock-center")
        .expect("center dock marker should exist");
    let bottom = render_source
        .find("terminal-tool-dock-bottom")
        .expect("bottom dock marker should exist");
    let right = render_source
        .find("terminal-tool-dock-right")
        .expect("right dock marker should exist");

    assert!(root < center);
    assert!(center < bottom);
    assert!(
        bottom < right,
        "bottom dock should be rendered inside the center column before the right toolbar dock"
    );
}

#[test]
fn terminal_command_bar_is_between_viewport_and_optional_bottom_tool() {
    let render_source = include_str!("../render_layout.rs");
    let viewport = render_source
        .find("render_terminal_viewport")
        .expect("terminal viewport should be rendered");
    let command_bar = render_source
        .find(".child(self.command_bar.clone())")
        .expect("bottom command bar should be rendered");
    let bottom_tool = render_source
        .find("when_some(state.bottom_panel")
        .expect("optional bottom tool should be rendered");

    assert!(viewport < command_bar);
    assert!(command_bar < bottom_tool);
}

#[test]
fn terminal_command_bar_is_only_rendered_for_live_terminal_input() {
    let render_source = include_str!("../render_layout.rs");
    let center_region = render_source
        .split("fn render_center_region")
        .nth(1)
        .and_then(|source| source.split("fn render_bottom_region").next())
        .expect("center region implementation should exist");

    let capability = center_region
        .find("let show_command_bar = self.accepts_live_terminal_input(cx);")
        .expect("command bar visibility must use the live terminal capability");
    let conditional = center_region
        .find(".when(show_command_bar")
        .expect("command bar must be conditionally rendered");
    let command_bar = center_region
        .find(".child(self.command_bar.clone())")
        .expect("live terminals should retain the command bar");

    assert!(capability < conditional);
    assert!(conditional < command_bar);
}

#[test]
fn terminal_recording_controls_live_in_the_command_bar() {
    let command_bar_source = include_str!("../command_bar/recording_render.rs");
    let command_bar_layout_source = include_str!("../command_bar/render.rs");
    let footer_source = include_str!("../recording_playback_render.rs");

    assert!(command_bar_layout_source.contains("render_recording_button"));
    assert!(command_bar_layout_source.contains("render_recording_controls"));
    assert!(command_bar_source.contains("terminal-command-recording\""));
    assert!(command_bar_source.contains("terminal-command-recording-start"));
    assert!(command_bar_source.contains("terminal-command-recording-pause"));
    assert!(command_bar_source.contains("terminal-command-recording-resume"));
    assert!(command_bar_source.contains("terminal-command-recording-stop"));
    assert!(!footer_source.contains("self.render_recording_footer(cx)"));
}

#[test]
fn terminal_command_bar_sits_with_primary_content_and_playback_keeps_its_footer() {
    let render_source = include_str!("../render_layout.rs");
    let center_region = render_source
        .split("fn render_center_region")
        .nth(1)
        .and_then(|source| source.split("fn render_bottom_region").next())
        .expect("center region implementation should exist");
    let primary_content = center_region
        .find(".child(primary_content)")
        .expect("primary terminal content should be rendered");
    let zmodem_progress = center_region
        .find(".when_some(zmodem_progress")
        .expect("ZMODEM progress should be rendered below primary content");
    let command_bar = center_region
        .find(".child(self.command_bar.clone())")
        .expect("live recording controls should be hosted by the command bar");
    let playback_footer = center_region
        .find(".child(self.render_terminal_session_footer(cx))")
        .expect("recording playback should retain its dedicated footer");
    let bottom_tool = center_region
        .find(".when_some(state.bottom_panel")
        .expect("optional bottom tool should be rendered");

    assert!(command_bar < primary_content);
    assert!(primary_content < zmodem_progress);
    assert!(zmodem_progress < playback_footer);
    assert!(playback_footer < bottom_tool);
}

#[test]
fn zmodem_upload_progress_has_stable_structure() {
    let progress_source = include_str!("../zmodem_progress.rs");

    assert!(progress_source.contains(r#""terminal-zmodem-upload""#));
    assert!(progress_source.contains(r#""terminal-zmodem-upload-name""#));
    assert!(progress_source.contains(r#"Progress::new("terminal-zmodem-upload-progress")"#));
    assert!(progress_source.contains(r#"t!("TerminalZmodem.uploading")"#));
}

#[test]
fn terminal_ui_does_not_expose_the_internal_operation_audit() {
    let view_source = include_str!("../../view.rs");
    let initialization_source = include_str!("../initialization.rs");
    let render_source = include_str!("../render_layout.rs");
    let command_bar_source = include_str!("../command_bar/render.rs");
    let command_bar_events_source = include_str!("../command_bar_events.rs");

    assert!(!view_source.contains("mod operation_history;"));
    assert!(!view_source.contains("OperationHistoryPanelState"));
    assert!(!initialization_source.contains("operation_history_request()"));
    assert!(!command_bar_source.contains("terminal-command-operation-history-toggle"));
    assert!(!command_bar_events_source.contains("ToggleOperationHistory"));
    assert!(!render_source.contains("render_operation_history_drawer"));
    assert!(!render_source.contains("terminal-operation-history-host"));
}

#[test]
fn command_bar_reflows_the_canvas_and_preserves_bounds_driven_pty_resize() {
    let render_layout_source = include_str!("../render_layout.rs");
    let render_surface_source = include_str!("../render_surface.rs");
    let terminal_layout_source = include_str!("../terminal_layout.rs");
    let command_bar_source = include_str!("../command_bar/render.rs");

    let viewport = render_surface_source
        .split("pub(super) fn render_terminal_viewport")
        .nth(1)
        .and_then(|source| source.split("fn terminal_viewport_state").next())
        .expect("terminal viewport implementation should exist");
    assert!(viewport.contains(".flex_1()"));
    assert!(viewport.contains(".min_h_0()"));

    let surface = render_surface_source
        .split("fn render_terminal_surface")
        .nth(1)
        .and_then(|source| source.split("fn render_addon_tooltip").next())
        .expect("terminal surface implementation should exist");
    assert!(surface.contains("window.content_mask().bounds"));
    assert!(surface.contains("this.resize_if_needed(viewport_bounds, cx);"));

    assert!(terminal_layout_source.contains("terminal_grid_size("));
    assert!(terminal_layout_source.contains("terminal.resize("));
    assert!(
        !render_layout_source.contains("COMMAND_BAR_COLLAPSED_HEIGHT"),
        "the center layout must not manually subtract command bar height"
    );
    assert!(
        !command_bar_source.contains("resize_if_needed"),
        "the command bar must let the canvas bounds drive the existing resize path"
    );
}

#[test]
fn clipped_terminal_surface_uses_the_visible_width_for_grid_columns() {
    let surface_bounds = Bounds::new(Point::new(px(12.0), px(12.0)), size(px(1000.0), px(420.0)));
    let content_mask_bounds =
        Bounds::new(Point::new(px(12.0), px(12.0)), size(px(800.0), px(380.0)));

    let viewport_bounds = terminal_viewport_bounds(surface_bounds, content_mask_bounds);

    assert_eq!(
        viewport_bounds,
        Bounds::new(Point::new(px(12.0), px(12.0)), size(px(800.0), px(380.0)))
    );
    assert_eq!(
        terminal_grid_size(viewport_bounds.size, px(10.0), px(20.0)),
        (80, 19)
    );
    assert_eq!(
        terminal_grid_size(surface_bounds.size, px(10.0), px(20.0)),
        (100, 21),
        "the unclipped surface would tell the PTY to wrap twenty columns too late"
    );
}

#[test]
fn terminal_viewport_bounds_preserves_the_grid_origin_when_clipped() {
    let surface_bounds = Bounds::new(Point::new(px(100.0), px(80.0)), size(px(500.0), px(300.0)));
    let offset_mask = Bounds::new(Point::new(px(140.0), px(120.0)), size(px(300.0), px(180.0)));
    let disjoint_mask = Bounds::new(Point::new(px(700.0), px(500.0)), size(px(100.0), px(100.0)));

    assert_eq!(
        terminal_viewport_bounds(surface_bounds, offset_mask),
        Bounds::new(Point::new(px(100.0), px(80.0)), size(px(340.0), px(220.0))),
        "clipping must not shift the terminal grid origin used by mouse and IME coordinates"
    );

    let empty = terminal_viewport_bounds(surface_bounds, disjoint_mask);
    assert_eq!(empty.origin, surface_bounds.origin);
    assert_eq!(empty.size, size(px(0.0), px(0.0)));
    assert_eq!(terminal_grid_size(empty.size, px(10.0), px(20.0)), (1, 1));
}

#[test]
fn command_bar_recording_controls_are_pane_private_and_use_safe_terminal_apis() {
    let view_source = include_str!("../../view.rs");
    let recording_source = include_str!("../recording_footer.rs");
    let command_bar_source = include_str!("../command_bar/recording_render.rs");
    let event_source = include_str!("../command_bar_events.rs");
    let render_source = include_str!("../render_layout.rs");

    for pane_field in [
        "recording_path_prompt_pending: bool",
        "recording_control_error: Option<String>",
        "recording_ticker: Option<Task<()>>",
    ] {
        assert!(
            view_source.contains(pane_field),
            "recording UI state must remain on each TerminalView: `{pane_field}`"
        );
    }
    assert!(!recording_source.contains("Global<"));
    assert!(recording_source.contains(".start_output_recording(output_path)"));
    assert!(recording_source.contains(".pause_recording()"));
    assert!(recording_source.contains(".resume_recording()"));
    assert!(recording_source.contains(".stop_recording()"));
    assert!(command_bar_source.contains("TerminalCommandBarEvent::StartRecording"));
    assert!(command_bar_source.contains("TerminalCommandBarEvent::PauseRecording"));
    assert!(command_bar_source.contains("TerminalCommandBarEvent::ResumeRecording"));
    assert!(command_bar_source.contains("TerminalCommandBarEvent::StopRecording"));
    assert!(event_source.contains("self.request_recording_start(cx)"));
    assert!(event_source.contains("self.request_recording_pause(cx)"));
    assert!(event_source.contains("self.request_recording_resume(cx)"));
    assert!(event_source.contains("self.request_recording_stop(cx)"));

    for action_handler in [
        ".on_action(cx.listener(Self::start_recording_action))",
        ".on_action(cx.listener(Self::pause_recording_action))",
        ".on_action(cx.listener(Self::resume_recording_action))",
        ".on_action(cx.listener(Self::stop_recording_action))",
    ] {
        assert!(render_source.contains(action_handler));
    }
}

#[test]
fn command_bar_discloses_output_only_capture_and_visible_failures() {
    let command_bar_source = include_str!("../command_bar/recording_render.rs");

    assert!(command_bar_source.contains("TerminalRecording.output_only"));
    assert!(command_bar_source.contains("TerminalRecording.input_included"));
    assert!(command_bar_source.contains("recording_snapshot_failure(&snapshot)"));
    assert!(command_bar_source.contains("recording_control_error"));
    assert!(command_bar_source.contains("cx.theme().danger"));
}

#[test]
fn recording_footer_formats_elapsed_time_and_safe_unique_paths() {
    assert_eq!(
        format_recording_elapsed(std::time::Duration::ZERO),
        "00:00:00"
    );
    assert_eq!(
        format_recording_elapsed(std::time::Duration::from_secs(3_661)),
        "01:01:01"
    );

    let directory = std::path::Path::new("recordings");
    let timestamp = "2026-07-27T15:30:12Z"
        .parse()
        .expect("fixed UTC timestamp should parse");
    let recording_id = uuid::Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff")
        .expect("fixed recording UUID should parse");
    let output_path = recording_output_path(directory, timestamp, recording_id);

    assert_eq!(output_path.parent(), Some(directory));
    assert_eq!(
        output_path.extension().and_then(|value| value.to_str()),
        Some("cast")
    );
    assert_eq!(
        output_path.file_name().and_then(|value| value.to_str()),
        Some("navop-terminal-20260727-153012-00112233-4455-6677-8899-aabbccddeeff.cast")
    );

    let path_helper = include_str!("../recording_footer.rs")
        .split("pub(super) fn recording_output_path")
        .nth(1)
        .expect("recording path helper should exist");
    for sensitive_source in [
        "hostname",
        "username",
        "connection_name",
        "connection_string",
        "credential",
        "cwd",
        "remote_path",
    ] {
        assert!(
            !path_helper.contains(sensitive_source),
            "recording filename must not incorporate `{sensitive_source}`"
        );
    }
}

#[test]
fn recording_playback_footer_is_a_fixed_dispatched_flex_child() {
    let render_source = include_str!("../recording_playback_render.rs");
    let layout_source = include_str!("../render_layout.rs");

    for fixed_height_contract in [
        ".h(RECORDING_PLAYBACK_FOOTER_HEIGHT)",
        ".min_h(RECORDING_PLAYBACK_FOOTER_HEIGHT)",
        ".max_h(RECORDING_PLAYBACK_FOOTER_HEIGHT)",
        ".flex_shrink_0()",
    ] {
        assert!(
            render_source.contains(fixed_height_contract),
            "playback footer must retain `{fixed_height_contract}`"
        );
    }
    assert!(!render_source.contains(".absolute()"));
    assert!(render_source.contains(".is_recording_playback()"));
    assert!(render_source.contains("self.render_recording_playback_footer(cx)"));
    assert!(!render_source.contains("self.render_recording_footer(cx)"));
    assert!(layout_source.contains(".child(self.render_terminal_session_footer(cx))"));
    assert!(!layout_source.contains(".child(self.render_recording_footer(cx))"));
}

#[test]
fn recording_playback_slider_seeks_only_after_user_release() {
    let controls_source = include_str!("../recording_playback_controls.rs");
    let handler = controls_source
        .split("fn handle_recording_playback_slider_event")
        .nth(1)
        .and_then(|source| source.split("fn request_recording_playback_seek").next())
        .expect("playback slider handler should exist");

    let change = handler
        .find("SliderEvent::Change(SliderValue::Single")
        .expect("slider change branch should exist");
    let release = handler
        .find("SliderEvent::Release(SliderValue::Single")
        .expect("slider release branch should exist");
    let seek = handler
        .find("request_recording_playback_seek")
        .expect("release should request one bounded seek");

    assert!(change < release);
    assert!(release < seek);
    assert!(
        !handler[..release].contains("request_recording_playback_seek"),
        "continuous slider changes must not rebuild the playback grid"
    );
}

#[test]
fn recording_playback_clock_is_owned_and_advanced_in_bounded_steps() {
    let view_source = include_str!("../../view.rs");
    let controls_source = include_str!("../recording_playback_controls.rs");
    let initialization_source = include_str!("../initialization.rs");
    let render_layout_source = include_str!("../render_layout.rs");

    for pane_field in [
        "recording_playback_slider: Entity<SliderState>",
        "recording_playback_slider_dragging: bool",
        "recording_playback_control_error: Option<String>",
        "recording_playback_ticker: Option<Task<()>>",
    ] {
        assert!(
            view_source.contains(pane_field),
            "playback UI state must remain on each TerminalView: `{pane_field}`"
        );
    }
    assert!(initialization_source.contains("SliderState::new()"));
    assert!(initialization_source.contains("Self::handle_recording_playback_slider_event"));
    assert!(
        initialization_source
            .contains("subscriptions.push(recording_playback_slider_subscription)")
    );
    assert!(render_layout_source.contains("self.sync_recording_playback_ticker(cx);"));
    assert!(render_layout_source.contains("self.sync_recording_playback_slider(window, cx);"));

    for clock_contract in [
        "PLAYBACK_TICK_INTERVAL",
        "Instant::now()",
        "try_for_each_playback_advance_step",
        "terminal.advance_recording_playback(step)",
        "recording_playback_ticker = Some(cx.spawn",
    ] {
        assert!(
            controls_source.contains(clock_contract),
            "playback clock must retain `{clock_contract}`"
        );
    }
    assert!(
        !controls_source.contains(".detach()"),
        "the playback clock task must remain owned by TerminalView"
    );
}

#[test]
fn terminal_command_bar_keeps_oxideterm_keyboard_and_overlay_contracts() {
    let interaction_source = include_str!("../command_bar/interaction.rs");
    let quick_interaction_source = include_str!("../command_bar/quick_interaction.rs");
    let render_source = include_str!("../command_bar/render.rs");
    let quick_source = include_str!("../command_bar/quick_render.rs");
    let suggestion_source = include_str!("../command_bar/suggestion_render.rs");

    for key in ["\"tab\"", "\"escape\""] {
        assert!(interaction_source.contains(key));
    }
    assert!(
        render_source.contains(".vertical_navigation(false)"),
        "command input must delegate Up/Down instead of consuming them as cursor movement"
    );
    assert!(render_source.contains(".on_action(cx.listener(Self::handle_history_previous))"));
    assert!(render_source.contains(".on_action(cx.listener(Self::handle_history_next))"));
    assert!(interaction_source.contains("fn handle_history_previous"));
    assert!(interaction_source.contains("fn handle_history_next"));
    assert!(
        interaction_source.contains(".history_search_results(\"\", HISTORY_NAVIGATION_LIMIT)"),
        "command bar history navigation must include scoped history-command repository entries"
    );
    assert!(
        interaction_source.contains("self.history_input_value.as_deref()"),
        "programmatic history values must not reset navigation through InputEvent::Change"
    );
    let refresh = interaction_source
        .split("fn refresh_suggestions")
        .nth(1)
        .and_then(|source| source.split("fn submit").next())
        .expect("refresh suggestions implementation should exist");
    assert!(refresh.contains("build_command_suggestions"));
    assert!(refresh.contains("command_inline_suffix"));
    assert!(refresh.contains("set_inline_completion_text"));
    assert!(!refresh.contains("reset_overlays"));
    assert!(render_source.contains("toggle_collapsed"));
    assert!(interaction_source.contains("TerminalCommandBarEvent::FocusTerminal"));
    assert!(interaction_source.contains("auto_grow(4, 12)"));
    assert!(interaction_source.contains("collapsed: true"));
    assert!(render_source.contains("COMMAND_BAR_INPUT_MIN_HEIGHT: f32 = 80.0"));
    assert!(render_source.contains("fn popover_bottom_offset(&self) -> f32"));
    assert!(render_source.contains("self.input_height + COMMAND_BAR_POPOVER_GAP"));
    assert!(render_source.contains("struct CommandBarResize {"));
    assert!(render_source.contains("entity_id: EntityId"));
    assert!(render_source.contains("initial_height: f32"));
    assert!(render_source.contains("initial_y: Rc<Cell<Option<Pixels>>>"));
    assert!(render_source.contains("DragMoveEvent<CommandBarResize>"));
    assert!(!render_source.contains("ResizePanel"));
    assert!(render_source.contains("group(\"terminal-command-resize-handle\")"));
    assert!(render_source.contains("group_hover(\"terminal-command-resize-handle\""));
    assert!(render_source.contains("COMMAND_BAR_RESIZE_HANDLE_HEIGHT: f32 = 6.0"));
    assert!(render_source.contains("COMMAND_BAR_RESIZE_GRIP_WIDTH: f32 = 32.0"));
    assert!(render_source.contains("COMMAND_BAR_RESIZE_GRIP_HOVER_WIDTH: f32 = 48.0"));
    assert!(render_source.contains("COMMAND_BAR_RESIZE_GRIP_HEIGHT: f32 = 2.0"));
    assert!(
        !render_source.contains("handle.w(px(COMMAND_BAR_RESIZE_GRIP_HOVER_WIDTH)).h("),
        "hover must not change the command-bar grip height or cause a geometry jump"
    );
    assert!(render_source.contains("cx.theme().drag_border"));
    assert!(render_source.contains("Input::new(&self.input_state)"));
    assert!(!render_source.contains(".h_full()"));
    assert!(render_source.contains(".h(px(self.input_height))"));
    assert!(render_source.contains("drag.initial_height + delta"));
    assert!(render_source.contains("drag.initial_y.set(Some(window.mouse_position().y))"));
    assert!(!render_source.contains(".on_mouse_down(gpui::MouseButton::Left"));
    assert!(render_source.contains("initial_y - event.event.position.y"));
    assert!(!render_source.contains("event.bounds.center().y"));
    assert!(render_source.contains("if drag.entity_id != cx.entity_id()"));
    assert!(render_source.contains("with_size(Size::Medium)"));
    let expanded_row = render_source
        .split("fn render_input_row")
        .nth(1)
        .and_then(|source| source.split("impl Render for TerminalCommandBar").next())
        .expect("expanded command input row should exist");
    let terminal_toggle = render_source
        .split("fn render_terminal_toggle_button")
        .nth(1)
        .and_then(|source| source.split("fn render_quick_command_button").next())
        .expect("terminal toggle button should exist");
    assert!(terminal_toggle.contains("terminal-command-terminal-toggle"));
    assert!(terminal_toggle.contains("IconName::SquareTerminal"));
    assert!(terminal_toggle.contains("IconName::ChevronUp"));
    assert!(terminal_toggle.contains("IconName::ChevronDown"));
    assert!(!terminal_toggle.contains("self.target_label(cx)"));
    assert!(terminal_toggle.contains("when(!self.collapsed"));
    assert!(render_source.contains("this.toggle_collapsed(window, cx)"));
    assert!(expanded_row.contains("self.render_expanded_actions(cx)"));
    assert!(expanded_row.contains("self.render_terminal_toggle_button(cx)"));
    assert!(!expanded_row.contains("IconName::ChevronRight"));
    assert!(!render_source.contains("terminal-command-collapse-toggle-expanded"));
    assert!(!render_source.contains("terminal-command-collapse-toggle\""));
    assert!(expanded_row.contains("h_flex()"));
    assert!(expanded_row.contains(".min_w_0()"));
    assert!(expanded_row.contains(".flex_1()"));
    assert!(expanded_row.contains(".flex_shrink_0()"));
    assert!(!expanded_row.contains(".absolute()"));
    assert!(!render_source.contains("COMMAND_BAR_ACTIONS_WIDTH"));
    let terminal_toggle_position = expanded_row
        .find("self.render_terminal_toggle_button(cx)")
        .expect("terminal toggle should render in the expanded row");
    let input_position = expanded_row
        .find("Input::new(&self.input_state)")
        .expect("command input should render in the expanded row");
    let actions_position = expanded_row
        .find("self.render_expanded_actions(cx)")
        .expect("expanded actions should render in the expanded row");
    assert!(terminal_toggle_position < input_position);
    assert!(input_position < actions_position);
    assert!(render_source.contains("child(self.render_quick_command_button(cx))"));
    assert!(render_source.contains("when(self.quick_commands_open"));
    let choose_quick_command = quick_interaction_source
        .split("pub(super) fn choose_command")
        .nth(1)
        .and_then(|source| source.split("pub(super) fn select_quick_group").next())
        .expect("quick command selection implementation should exist");
    assert!(choose_quick_command.contains("if self.collapsed"));
    assert!(choose_quick_command.contains("TerminalCommandBarEvent::InputToPty(command)"));
    assert!(choose_quick_command.contains("set_command_input_value(state, command"));
    assert!(
        include_str!("../command_bar_events.rs").contains("self.paste_text(command, window, cx)")
    );
    assert!(interaction_source.contains("set_command_input_value(state, command"));
    assert!(
        include_str!("../command_bar/mod.rs")
            .contains("state.set_cursor_position(end_position, window, cx)")
    );
    for key in ["\"arrowup\"", "\"arrowdown\"", "\"home\"", "\"end\""] {
        assert!(quick_interaction_source.contains(key));
    }
    assert!(suggestion_source.contains("bottom(px(self.popover_bottom_offset()))"));
    assert!(suggestion_source.contains("bg(self.colors.background)"));
    assert!(suggestion_source.contains("let mut content"));
    assert!(suggestion_source.contains("overflow_y_scrollbar"));
    assert!(suggestion_source.contains("relative(0.96)"));
    assert!(quick_source.contains("group_quick_commands"));
    assert!(quick_source.contains("bottom(px(self.popover_bottom_offset()))"));
    assert!(quick_source.contains("relative(0.96)"));
    assert!(!quick_source.contains("on_scroll_wheel"));
    assert!(!quick_source.contains("on_mouse_down(MouseButton::Left"));
    assert!(quick_source.contains("on_mouse_down_out"));
    assert!(quick_source.contains("window.defer(cx"));
    assert!(quick_interaction_source.contains("window.defer(cx"));
    assert!(
        quick_interaction_source.contains("quick_scroll_handle = VirtualListScrollHandle::new()")
    );
    assert!(quick_source.contains("bottom(px(self.popover_bottom_offset()))"));
    assert!(
        include_str!("../command_bar/quick_render_list.rs")
            .contains("vertical_scrollbar(&self.quick_scroll_handle)")
    );
    assert!(include_str!("../command_bar/quick_render_list.rs").contains("v_virtual_list("));
    assert!(
        include_str!("../command_bar/quick_render_sidebar.rs")
            .contains("vertical_scrollbar(&self.quick_group_scroll_handle)")
    );

    let recording_source = include_str!("../command_bar/recording_render.rs");
    let command_bar_state_source = include_str!("../command_bar/mod.rs");
    assert!(command_bar_state_source.contains("recording_controls_open: bool"));
    assert!(render_source.contains("self.render_recording_button(cx)"));
    assert!(render_source.contains("when(self.recording_controls_open"));
    assert!(recording_source.contains("terminal-command-recording\""));
    assert!(recording_source.contains("TerminalRecording.control"));
    assert!(recording_source.contains("toggle_recording_controls"));
    assert!(recording_source.contains("render_recording_controls"));
    assert!(recording_source.contains("bottom(px(self.popover_bottom_offset()))"));
    assert!(recording_source.contains("relative(0.96)"));
    assert!(recording_source.contains("on_mouse_down_out"));
    assert!(recording_source.contains("window.defer(cx"));
}

#[test]
fn terminal_selection_has_window_mouse_up_fallback() {
    let source = [
        include_str!("../mouse_selection.rs"),
        include_str!("../resize_event_handler.rs"),
    ]
    .concat();

    assert!(source.matches("handle_window_mouse_up").count() >= 2);
    assert!(source.contains("window.on_mouse_event({"));
}

#[test]
fn terminal_reset_font_size_is_fifteen() {
    assert_eq!(TERMINAL_RESET_FONT_SIZE, 15.0);
}

#[test]
fn terminal_theme_source_does_not_define_font_settings() {
    let source = include_str!("../../theme.rs");

    assert!(!source.contains("pub font_size"));
    assert!(!source.contains("pub font_family"));
    assert!(!source.contains("pub font_fallbacks"));
    assert!(!source.contains("pub line_height_scale"));
}

#[test]
fn terminal_render_uses_cached_font_metrics() {
    let render_setup = include_str!("../render_layout.rs");
    let metric_source = include_str!("../terminal_layout.rs");

    assert!(metric_source.contains("fn refresh_terminal_font_metrics("));
    assert!(!render_setup.contains("cx.text_system().all_font_names()"));
}
