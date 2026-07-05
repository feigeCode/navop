use std::cell::Cell as StdCell;

use alacritty_terminal::index::Point as AlacPoint;
use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers, MouseButton};

pub(super) fn take_whole_scroll_lines(scroll_lines_accumulated: &mut f32) -> i32 {
    let lines = scroll_lines_accumulated.trunc() as i32;
    *scroll_lines_accumulated -= lines as f32;
    lines
}

pub(super) fn sgr_mouse_wheel_report(lines: i32, col: usize, row: usize) -> Option<String> {
    if lines == 0 {
        return None;
    }

    let button = if lines > 0 { 64 } else { 65 };
    Some(format!("\x1b[<{};{};{}M", button, col + 1, row + 1))
}

pub(super) fn sgr_mouse_button_report(button: u8, col: usize, row: usize, pressed: bool) -> String {
    let suffix = if pressed { 'M' } else { 'm' };
    format!("\x1b[<{};{};{}{}", button, col + 1, row + 1, suffix)
}

pub(super) fn mouse_button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

pub(super) fn encode_mouse_modifiers(modifiers: Modifiers) -> u8 {
    let mut bits = 0u8;
    if modifiers.shift {
        bits |= 4;
    }
    if modifiers.alt {
        bits |= 8;
    }
    if modifiers.control {
        bits |= 16;
    }
    bits
}

pub(super) fn sgr_mouse_mode_enabled(mode: TermMode) -> bool {
    mode.contains(TermMode::SGR_MOUSE) && mode.intersects(TermMode::MOUSE_MODE)
}

pub(super) fn should_defer_sgr_left_press(
    button: MouseButton,
    modifiers: Modifiers,
    mode: TermMode,
) -> bool {
    button == MouseButton::Left
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.control
        && !modifiers.platform
        && sgr_mouse_mode_enabled(mode)
}

pub(super) fn should_start_selection_from_pending_sgr_press(
    start: AlacPoint,
    current: AlacPoint,
) -> bool {
    start != current
}

pub(super) fn should_extend_selection_on_shift_click(
    button: MouseButton,
    modifiers: Modifiers,
    has_selection: bool,
) -> bool {
    button == MouseButton::Left && modifiers.shift && has_selection
}

pub(super) fn should_scroll_to_bottom_on_user_input(
    display_offset: usize,
    pending_display_offset: &StdCell<Option<usize>>,
) -> bool {
    pending_display_offset.take();
    display_offset > 0
}

pub(super) fn should_defer_inline_history_prompt_input_to_text_system(
    keystroke: &Keystroke,
) -> bool {
    let modifiers = keystroke.modifiers;
    !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
        && (keystroke.key == "space" || keystroke.key.chars().count() == 1)
}
