use super::*;

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
            kind,
            true,
            TermMode::ALT_SCREEN,
            None,
        ));
    }
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
