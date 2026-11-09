use std::cell::Cell as StdCell;

use alacritty_terminal::index::Point as AlacPoint;
use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers, MouseButton, Pixels};

const TERMINAL_SELECTION_AUTOSCROLL_MAX_ROWS: i32 = 4;
const LEGACY_MOUSE_BUTTON_RELEASE: u8 = 3;
const LEGACY_MOUSE_WHEEL_UP: u8 = 64;
const LEGACY_MOUSE_WHEEL_DOWN: u8 = 65;
const LEGACY_MOUSE_COORDINATE_OFFSET: u32 = 32;

pub(super) fn terminal_selection_autoscroll_delta_rows(
    position_y: Pixels,
    top: Pixels,
    bottom: Pixels,
    line_height: Pixels,
) -> i32 {
    let distance = if position_y < top {
        f32::from(top - position_y)
    } else if position_y > bottom {
        -f32::from(position_y - bottom)
    } else {
        return 0;
    };

    let line_height = f32::from(line_height).max(1.0);
    let rows = (distance.abs() / line_height)
        .ceil()
        .max(1.0)
        .min(TERMINAL_SELECTION_AUTOSCROLL_MAX_ROWS as f32) as i32;

    if distance > 0.0 { rows } else { -rows }
}

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

pub(super) fn mouse_button_report(
    button: u8,
    col: usize,
    row: usize,
    pressed: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if !mouse_reporting_enabled(mode) {
        return None;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        return Some(sgr_mouse_button_report(button, col, row, pressed).into_bytes());
    }

    legacy_mouse_button_report(
        button,
        col,
        row,
        pressed,
        mode.contains(TermMode::UTF8_MOUSE),
    )
}

pub(super) fn mouse_wheel_report(
    lines: i32,
    col: usize,
    row: usize,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if lines == 0 || !mouse_reporting_enabled(mode) {
        return None;
    }

    let button = if lines > 0 {
        LEGACY_MOUSE_WHEEL_UP
    } else {
        LEGACY_MOUSE_WHEEL_DOWN
    };
    if mode.contains(TermMode::SGR_MOUSE) {
        return sgr_mouse_wheel_report(lines, col, row).map(String::into_bytes);
    }

    legacy_mouse_button_report(button, col, row, true, mode.contains(TermMode::UTF8_MOUSE))
}

fn legacy_mouse_button_report(
    button: u8,
    col: usize,
    row: usize,
    pressed: bool,
    utf8: bool,
) -> Option<Vec<u8>> {
    let button = if pressed {
        button
    } else {
        LEGACY_MOUSE_BUTTON_RELEASE | (button & 0b0001_1100)
    };
    let button = u32::from(button).checked_add(LEGACY_MOUSE_COORDINATE_OFFSET)?;
    let col = u32::try_from(col)
        .ok()?
        .checked_add(1 + LEGACY_MOUSE_COORDINATE_OFFSET)?;
    let row = u32::try_from(row)
        .ok()?
        .checked_add(1 + LEGACY_MOUSE_COORDINATE_OFFSET)?;

    let mut report = Vec::with_capacity(if utf8 { 9 } else { 6 });
    report.extend_from_slice(b"\x1b[M");
    if utf8 {
        for value in [button, col, row] {
            let mut encoded = [0; 4];
            report.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut encoded).as_bytes());
        }
    } else {
        report.push(u8::try_from(button).ok()?);
        report.push(u8::try_from(col).ok()?);
        report.push(u8::try_from(row).ok()?);
    }
    Some(report)
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

pub(super) fn mouse_reporting_enabled(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

pub(super) fn sgr_mouse_mode_enabled(mode: TermMode) -> bool {
    mode.contains(TermMode::SGR_MOUSE) && mouse_reporting_enabled(mode)
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
