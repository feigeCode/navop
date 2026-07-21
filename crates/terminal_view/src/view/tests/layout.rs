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
fn terminal_command_bar_keeps_oxideterm_keyboard_and_overlay_contracts() {
    let interaction_source = include_str!("../command_bar/interaction.rs");
    let quick_interaction_source = include_str!("../command_bar/quick_interaction.rs");
    let render_source = include_str!("../command_bar/render.rs");
    let quick_source = include_str!("../command_bar/quick_render.rs");
    let suggestion_source = include_str!("../command_bar/suggestion_render.rs");

    for key in ["\"up\"", "\"down\"", "\"tab\"", "\"escape\""] {
        assert!(interaction_source.contains(key));
    }
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
    assert!(render_source.contains("COMMAND_BAR_INPUT_MIN_HEIGHT: f32 = 80.0"));
    assert!(render_source.contains("with_size(Size::Medium)"));
    assert!(render_source.contains("child(self.render_quick_command_button(cx))"));
    assert!(render_source.contains("when(self.quick_commands_open"));
    assert!(quick_interaction_source.contains("self.collapsed = false"));
    assert!(quick_interaction_source.contains("set_command_input_value(state, command"));
    assert!(interaction_source.contains("set_command_input_value(state, command"));
    assert!(
        include_str!("../command_bar/mod.rs")
            .contains("state.set_cursor_position(end_position, window, cx)")
    );
    for key in ["\"arrowup\"", "\"arrowdown\"", "\"home\"", "\"end\""] {
        assert!(quick_interaction_source.contains(key));
    }
    assert!(suggestion_source.contains("bottom(px(88.0))"));
    assert!(suggestion_source.contains("bg(self.colors.background)"));
    assert!(suggestion_source.contains("let mut content"));
    assert!(suggestion_source.contains("overflow_y_scrollbar"));
    assert!(suggestion_source.contains("relative(0.96)"));
    assert!(quick_source.contains("group_quick_commands"));
    assert!(quick_source.contains("bottom(px(bottom_offset))"));
    assert!(quick_source.contains("relative(0.96)"));
    assert!(!quick_source.contains("on_scroll_wheel"));
    assert!(!quick_source.contains("on_mouse_down(MouseButton::Left"));
    assert!(quick_source.contains("on_mouse_down_out"));
    assert!(quick_source.contains("window.defer(cx"));
    assert!(quick_interaction_source.contains("window.defer(cx"));
    assert!(
        quick_interaction_source.contains("quick_scroll_handle = VirtualListScrollHandle::new()")
    );
    assert!(quick_source.contains("bottom(px(bottom_offset))"));
    assert!(
        include_str!("../command_bar/quick_render_list.rs")
            .contains("vertical_scrollbar(&self.quick_scroll_handle)")
    );
    assert!(include_str!("../command_bar/quick_render_list.rs").contains("v_virtual_list("));
    assert!(
        include_str!("../command_bar/quick_render_sidebar.rs")
            .contains("vertical_scrollbar(&self.quick_group_scroll_handle)")
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
