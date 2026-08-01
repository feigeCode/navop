use super::*;

#[test]
fn sgr_mouse_wheel_report_maps_positive_lines_to_wheel_up() {
    assert_eq!(
        sgr_mouse_wheel_report(1, 4, 2).as_deref(),
        Some("\x1b[<64;5;3M")
    );
}
#[test]
fn sgr_mouse_wheel_report_maps_negative_lines_to_wheel_down() {
    assert_eq!(
        sgr_mouse_wheel_report(-1, 4, 2).as_deref(),
        Some("\x1b[<65;5;3M")
    );
    assert_eq!(sgr_mouse_wheel_report(0, 4, 2), None);
}

#[test]
fn sgr_mouse_button_report_uses_capital_m_on_press() {
    // 左键按下，列 0、行 0 -> 转 1-based
    let s = sgr_mouse_button_report(0, 0, 0, true);
    assert_eq!(s, "\x1b[<0;1;1M");
}

#[test]
fn sgr_mouse_button_report_uses_lowercase_m_on_release() {
    let s = sgr_mouse_button_report(2, 9, 4, false);
    // 右键 (button=2) 释放在 1-based col=10 row=5
    assert_eq!(s, "\x1b[<2;10;5m");
}

#[test]
fn sgr_mouse_button_report_supports_modifier_encoded_buttons() {
    // 左键 + shift (4) + ctrl (16) -> button=20
    let s = sgr_mouse_button_report(20, 0, 0, true);
    assert_eq!(s, "\x1b[<20;1;1M");
}

#[test]
fn sgr_mouse_button_report_supports_drag_button_codes() {
    // 拖动事件：button + 32（xterm 拖动位）
    // 左键拖动 = 32
    let s = sgr_mouse_button_report(32, 7, 11, true);
    assert_eq!(s, "\x1b[<32;8;12M");
}

#[test]
fn mouse_button_code_maps_three_main_buttons() {
    assert_eq!(mouse_button_code(MouseButton::Left), Some(0));
    assert_eq!(mouse_button_code(MouseButton::Middle), Some(1));
    assert_eq!(mouse_button_code(MouseButton::Right), Some(2));
}

#[test]
fn encode_mouse_modifiers_packs_shift_alt_control() {
    let none = Modifiers::default();
    assert_eq!(encode_mouse_modifiers(none), 0);

    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    assert_eq!(encode_mouse_modifiers(shift), 4);

    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    assert_eq!(encode_mouse_modifiers(alt), 8);

    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    assert_eq!(encode_mouse_modifiers(ctrl), 16);

    let all = Modifiers {
        shift: true,
        alt: true,
        control: true,
        ..Default::default()
    };
    assert_eq!(encode_mouse_modifiers(all), 28);
}

#[test]
fn sgr_mouse_mode_enabled_requires_sgr_and_mouse_reporting() {
    assert!(!sgr_mouse_mode_enabled(TermMode::SGR_MOUSE));
    assert!(!sgr_mouse_mode_enabled(TermMode::MOUSE_REPORT_CLICK));
    assert!(sgr_mouse_mode_enabled(
        TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK
    ));
}

#[test]
fn should_defer_sgr_left_press_only_for_plain_left_mouse_in_sgr_mode() {
    let mode = TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK;
    let none = Modifiers::default();
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let control = Modifiers {
        control: true,
        ..Default::default()
    };
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    let platform = Modifiers {
        platform: true,
        ..Default::default()
    };

    assert!(should_defer_sgr_left_press(MouseButton::Left, none, mode));
    assert!(!should_defer_sgr_left_press(MouseButton::Right, none, mode));
    assert!(!should_defer_sgr_left_press(MouseButton::Left, shift, mode));
    assert!(!should_defer_sgr_left_press(
        MouseButton::Left,
        control,
        mode
    ));
    assert!(!should_defer_sgr_left_press(MouseButton::Left, alt, mode));
    assert!(!should_defer_sgr_left_press(
        MouseButton::Left,
        platform,
        mode
    ));
    assert!(!should_defer_sgr_left_press(
        MouseButton::Left,
        none,
        TermMode::default()
    ));
}

#[test]
fn alt_left_mouse_starts_block_selection() {
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };

    assert!(should_start_block_selection(MouseButton::Left, alt));
    assert!(!should_start_block_selection(MouseButton::Right, alt));
    assert!(!should_start_block_selection(
        MouseButton::Left,
        Modifiers::default()
    ));
}

#[test]
fn block_selection_text_extracts_same_columns_from_each_line() {
    let rows = vec!["alpha beta".to_string(), "bravo charlie".to_string()];
    let start = AlacPoint::new(Line(0), Column(2));
    let end = AlacPoint::new(Line(1), Column(6));

    let text = block_selection_text_from_rows(&rows, start, end);

    assert_eq!(Some("pha b\navo c".to_string()), text);
}

#[test]
fn pending_sgr_press_starts_selection_after_mouse_reaches_another_cell() {
    let start = AlacPoint::new(Line(1), Column(1));
    assert!(!should_start_selection_from_pending_sgr_press(start, start));
    assert!(should_start_selection_from_pending_sgr_press(
        start,
        AlacPoint::new(Line(1), Column(2))
    ));
    assert!(should_start_selection_from_pending_sgr_press(
        start,
        AlacPoint::new(Line(2), Column(1))
    ));
}

#[test]
fn shift_left_click_extends_existing_terminal_selection_only() {
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let none = Modifiers::default();

    assert!(should_extend_selection_on_shift_click(
        MouseButton::Left,
        shift,
        true
    ));
    assert!(!should_extend_selection_on_shift_click(
        MouseButton::Left,
        shift,
        false
    ));
    assert!(!should_extend_selection_on_shift_click(
        MouseButton::Left,
        none,
        true
    ));
    assert!(!should_extend_selection_on_shift_click(
        MouseButton::Right,
        shift,
        true
    ));
}

#[test]
fn playback_scroll_bypasses_live_alt_screen_and_vi_paths() {
    let source = include_str!("../scroll.rs");
    let handler = source
        .split("pub(super) fn handle_scroll")
        .nth(1)
        .and_then(|source| source.split("pub(super) fn pixel_to_point").next())
        .expect("scroll handler should exist");

    let capability = handler
        .find("let accepts_live_input = self.accepts_live_terminal_input(cx);")
        .expect("scroll routing must inspect the live input capability");
    let alt_screen = handler
        .find("if accepts_live_input && mode.contains(TermMode::ALT_SCREEN)")
        .expect("alt-screen forwarding must be live-only");
    let vi_cursor = handler
        .find("if accepts_live_input && mode.contains(TermMode::VI)")
        .expect("VI cursor scrolling must be live-only");
    let local_scrollback = handler
        .rfind("scroll_display")
        .expect("local display scrolling should remain available");

    assert!(capability < alt_screen);
    assert!(alt_screen < vi_cursor);
    assert!(vi_cursor < local_scrollback);
}

#[test]
fn playback_sgr_mode_never_defers_local_left_button_selection() {
    let source = include_str!("../mouse_down.rs");
    let handler = source
        .split("pub(super) fn handle_mouse_down")
        .nth(1)
        .and_then(|source| {
            source
                .split("pub(super) fn handle_middle_mouse_down")
                .next()
        })
        .expect("mouse-down handler should exist");

    let capability = handler
        .find("let accepts_live_input = self.accepts_live_terminal_input(cx);")
        .expect("mouse-down routing must inspect the live input capability");
    let deferred_press = handler
        .find("&& should_defer_sgr_left_press")
        .expect("pending SGR press must be restricted to live input");

    assert!(capability < deferred_press);
}
