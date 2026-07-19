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
