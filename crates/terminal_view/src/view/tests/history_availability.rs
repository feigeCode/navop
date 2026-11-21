use super::*;

#[test]
fn history_prompt_requires_global_autocomplete_switch() {
    let mode = TermMode::empty();

    assert!(history_prompt_available(
        true,
        TerminalConnectionKind::Local,
        mode,
        true,
    ));
    assert!(!history_prompt_available(
        false,
        TerminalConnectionKind::Local,
        mode,
        true,
    ));
}
#[test]
fn history_prompt_is_available_for_local_and_ssh_prompt_input() {
    let mode = TermMode::empty();

    assert!(history_prompt_available(
        true,
        TerminalConnectionKind::Local,
        mode,
        true,
    ));
    assert!(!history_prompt_available(
        true,
        TerminalConnectionKind::Serial,
        mode,
        true,
    ));
    assert!(history_prompt_available(
        true,
        TerminalConnectionKind::Ssh,
        mode,
        true,
    ));
}

#[test]
fn history_prompt_is_unavailable_in_terminal_application_modes() {
    for connection_kind in [TerminalConnectionKind::Local, TerminalConnectionKind::Ssh] {
        for mode in [
            TermMode::FOCUS_IN_OUT,
            TermMode::MOUSE_MODE,
            TermMode::DISAMBIGUATE_ESC_CODES,
            TermMode::ALT_SCREEN,
            TermMode::VI,
        ] {
            assert!(!history_prompt_available(true, connection_kind, mode, true));
        }
    }
}

#[test]
fn history_prompt_requires_active_shell_prompt_input() {
    assert!(!history_prompt_available(
        true,
        TerminalConnectionKind::Local,
        TermMode::empty(),
        false,
    ));
    assert!(history_prompt_available(
        true,
        TerminalConnectionKind::Local,
        TermMode::empty(),
        true,
    ));
}

#[test]
fn history_prompt_dropdown_flips_above_when_cursor_is_near_bottom() {
    let terminal_bounds = Bounds::new(Point::new(px(12.0), px(12.0)), size(px(800.0), px(280.0)));
    let line_height = px(20.0);
    let cursor_line = 11;
    let cursor_top = terminal_bounds.origin.y + line_height * cursor_line as f32;

    let origin = history_prompt_dropdown_origin(
        terminal_bounds,
        px(0.0),
        px(8.0),
        line_height,
        cursor_line,
        24,
        6,
        false,
    );

    assert!(origin.y < cursor_top);
    assert!(origin.y >= terminal_bounds.origin.y);
}

#[test]
fn history_prompt_dropdown_stays_near_cursor_when_match_count_is_huge() {
    // 终端较高、光标靠上：即便 cd 补全返回上百条，高度封顶后仍应贴着光标下方展开，
    // 而不是被未封顶高度夹到窗口顶部铺满整屏。
    let terminal_bounds = Bounds::new(Point::new(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
    let line_height = px(20.0);
    let cursor_line = 2;
    let cursor_top = line_height * cursor_line as f32;

    let origin = history_prompt_dropdown_origin(
        terminal_bounds,
        px(0.0),
        px(8.0),
        line_height,
        cursor_line,
        4,
        200,
        false,
    );

    let below_top = cursor_top + line_height + px(6.0);
    assert_eq!(origin.y, below_top);
    assert!(origin.y + px(280.0) <= terminal_bounds.bottom());
}

#[test]
fn history_prompt_overlay_renders_max_height_and_scrollbar() {
    let render_source = include_str!("../history_render.rs");
    let rules_source = include_str!("../history_prompt_rules.rs");
    let actions_source = include_str!("../history_actions.rs");

    assert!(rules_source.contains("HISTORY_PROMPT_DROPDOWN_MAX_HEIGHT"));
    assert!(rules_source.contains("content_height.min(px(HISTORY_PROMPT_DROPDOWN_MAX_HEIGHT))"));
    // 弹层背景必须走带透明度治理的 helper（issue #73），不允许渲染层直接裸用背景色；
    // 透明度必须来自用户设置，而不是又一个写死的常量。
    assert!(render_source.contains("history_prompt_dropdown_background("));
    assert!(render_source.contains("self.suggestion_popup_opacity"));
    assert!(render_source.contains(".overflow_y_scroll()"));
    assert!(render_source.contains(".track_scroll(&self.history_prompt_scroll_handle)"));
    assert!(render_source.contains("scroll_to_item(index)"));
    assert!(actions_source.contains("scroll_history_prompt_selection_into_view()"));
}

#[test]
fn suggestion_popup_opacity_travels_from_the_settings_panel_to_the_dropdown() {
    // issue #73：弹层背景不透明度现在由用户设置驱动，而这条链有 6 段
    // （设置面板滑块 → sidebar 事件 → 视图 setter → settings.json 落盘 →
    // 回灌视图 → 渲染弹层）。任何一段漏接都会静默失效 —— 弹层永远停在默认值，
    // 单测却全绿。所以这里逐段按源码锚点锁住接线，而不是只测两端。
    let panel = include_str!("../../sidebar/settings_panel.rs");
    assert!(panel.contains("SuggestionPopupOpacityChanged"));
    assert!(panel.contains("suggestion_popup_opacity_slider"));

    let sidebar = include_str!("../../sidebar/mod.rs");
    assert!(sidebar.contains("SettingsPanelEvent::SuggestionPopupOpacityChanged"));
    assert!(sidebar.contains("TerminalSidebarEvent::SuggestionPopupOpacityChanged"));

    let events = include_str!("../sidebar_events.rs");
    assert!(events.contains("TerminalSidebarEvent::SuggestionPopupOpacityChanged("));
    assert!(events.contains("self.set_suggestion_popup_opacity("));

    let preferences = include_str!("../preferences.rs");
    // 弹层透明度改成会话覆盖后不再写全局设置。
    assert!(
        preferences.contains("self.session_overrides.suggestion_popup_opacity = Some(opacity);")
    );

    let settings = include_str!("../../settings.rs");
    assert!(
        settings.contains(
            "settings.terminal_suggestion_popup_opacity = next.suggestion_popup_opacity;"
        )
    );

    let appearance = include_str!("../appearance.rs");
    assert!(appearance.contains("self.suggestion_popup_opacity ="));
    assert!(appearance.contains("sidebar.set_suggestion_popup_opacity("));

    let render = include_str!("../history_render.rs");
    assert!(render.contains("self.suggestion_popup_opacity"));
}

#[test]
fn history_prompt_overlay_bounds_reset_origin_for_local_overlay_positioning() {
    let terminal_bounds = Bounds::new(Point::new(px(96.0), px(144.0)), size(px(800.0), px(280.0)));

    let overlay_bounds = history_prompt_overlay_bounds(terminal_bounds);

    assert_eq!(overlay_bounds.origin, Point::new(px(0.0), px(0.0)));
    assert_eq!(overlay_bounds.size, terminal_bounds.size);
}

#[test]
fn history_prompt_accepts_selected_suggestion_suffix() {
    let mut state = HistoryPromptState::from_input("git st");
    state.set_matches(vec!["git status".to_string()]);

    let accepted = state.accept_selected_suggestion();

    assert_eq!(
        accepted,
        Some(HistoryPromptAccept::AppendSuffix("atus".to_string()))
    );
    assert_eq!(state.input(), "git status");
}

#[test]
fn history_prompt_navigation_restores_original_input() {
    let mut state = HistoryPromptState::from_input("git");
    state.set_matches(vec![
        "git status".to_string(),
        "git stash".to_string(),
        "git switch".to_string(),
    ]);

    assert_eq!(state.navigate_previous().as_deref(), Some("git status"));
    assert_eq!(state.navigate_previous().as_deref(), Some("git stash"));
    assert_eq!(state.navigate_next().as_deref(), Some("git status"));
    assert_eq!(state.navigate_next().as_deref(), Some("git"));
}

#[test]
fn history_prompt_keeps_query_prefix_while_browsing_matches() {
    let mut state = HistoryPromptState::from_input("git s");
    state.set_matches(vec![
        "git status".to_string(),
        "git stash".to_string(),
        "git switch".to_string(),
    ]);

    assert_eq!(state.query_input(), "git s");
    assert_eq!(state.navigate_previous().as_deref(), Some("git status"));
    assert_eq!(state.query_input(), "git s");
    assert_eq!(state.navigate_previous().as_deref(), Some("git stash"));
    assert_eq!(state.query_input(), "git s");
}
