use alacritty_terminal::term::TermMode;
use gpui::{Bounds, Hsla, Keystroke, MouseButton, Pixels, Point, px};
use one_core::storage::TerminalHistoryScope;
use terminal::terminal::{TerminalConnectionKind, TerminalModelEvent};

pub(super) fn should_dismiss_history_prompt_for_keystroke(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;
    let key = keystroke.key.as_str();

    if modifiers.platform {
        return true;
    }

    if modifiers.control && !modifiers.alt {
        return !matches!(key, "r" | "u" | "c");
    }

    if modifiers.alt && !modifiers.control {
        return key != "f";
    }

    if !modifiers.control && !modifiers.alt {
        return matches!(
            key,
            "left" | "home" | "end" | "delete" | "pageup" | "pagedown" | "escape" | "tab"
        );
    }

    true
}

pub(super) fn should_dismiss_history_prompt_for_mouse(button: MouseButton) -> bool {
    matches!(
        button,
        MouseButton::Left | MouseButton::Middle | MouseButton::Right
    )
}

pub(super) fn should_dismiss_history_prompt_for_scroll(lines: i32) -> bool {
    lines != 0
}

pub(super) fn should_reset_history_prompt_for_terminal_event(event: &TerminalModelEvent) -> bool {
    matches!(
        event,
        TerminalModelEvent::PromptStart
            | TerminalModelEvent::InputStart
            | TerminalModelEvent::CommandStart
    )
}

#[cfg(test)]
pub(super) fn should_refresh_history_commands_for_terminal_event(
    event: &TerminalModelEvent,
) -> bool {
    matches!(event, TerminalModelEvent::CommandHistoryChanged)
}

pub(super) fn history_prompt_available(
    autocomplete_enabled: bool,
    connection_kind: TerminalConnectionKind,
    mode: TermMode,
    shell_prompt_input_active: bool,
) -> bool {
    autocomplete_enabled
        && history_prompt_connection_supported(connection_kind)
        && shell_prompt_input_active
        && !terminal_application_mode_active(mode)
        && !mode.contains(TermMode::ALT_SCREEN)
        && !mode.contains(TermMode::VI)
}

fn history_prompt_connection_supported(connection_kind: TerminalConnectionKind) -> bool {
    matches!(
        connection_kind,
        TerminalConnectionKind::Local | TerminalConnectionKind::Ssh
    )
}

pub(super) fn terminal_history_scope(
    live_connection_kind: Option<TerminalConnectionKind>,
    connection_id: Option<i64>,
) -> Option<TerminalHistoryScope> {
    match live_connection_kind {
        Some(TerminalConnectionKind::Local) => Some(TerminalHistoryScope::local()),
        Some(TerminalConnectionKind::Ssh) => connection_id.map(TerminalHistoryScope::ssh),
        Some(TerminalConnectionKind::Serial) | None => None,
    }
}

fn terminal_application_mode_active(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
        || mode.contains(TermMode::FOCUS_IN_OUT)
        || mode.contains(TermMode::DISAMBIGUATE_ESC_CODES)
}

fn local_tui_application_active(mode: TermMode) -> bool {
    mode.contains(TermMode::ALT_SCREEN) || terminal_application_mode_active(mode)
}

pub(super) fn should_confirm_local_terminal_close(
    live_connection_kind: Option<TerminalConnectionKind>,
    command_running: bool,
    mode: TermMode,
    child_exited: Option<i32>,
) -> bool {
    live_connection_kind == Some(TerminalConnectionKind::Local)
        && child_exited.is_none()
        && (command_running || local_tui_application_active(mode))
}

pub(super) const HISTORY_PROMPT_DROPDOWN_MIN_WIDTH: f32 = 300.0;
pub(super) const HISTORY_PROMPT_DROPDOWN_MAX_WIDTH: f32 = 500.0;
const HISTORY_PROMPT_DROPDOWN_BACKGROUND_OPACITY: f32 = 0.72;
const HISTORY_PROMPT_ACTIVE_BACKGROUND_OPACITY: f32 = 0.32;
const HISTORY_PROMPT_DROPDOWN_GAP_Y: f32 = 6.0;
const HISTORY_PROMPT_DROPDOWN_EDGE_PADDING: f32 = 8.0;
const HISTORY_PROMPT_DROPDOWN_ROW_PADDING_Y: f32 = 12.0;
const HISTORY_PROMPT_DROPDOWN_CONTAINER_PADDING_Y: f32 = 16.0;
const HISTORY_PROMPT_DROPDOWN_SEARCH_HEADER_HEIGHT: f32 = 20.0;
const HISTORY_PROMPT_DROPDOWN_ROW_GAP: f32 = 4.0;
const HISTORY_PROMPT_DROPDOWN_BORDER_Y: f32 = 2.0;
const HISTORY_PROMPT_DROPDOWN_INPUT_CLEARANCE: f32 = 8.0;

pub(super) fn history_prompt_dropdown_background(background: Hsla) -> Hsla {
    background.opacity(HISTORY_PROMPT_DROPDOWN_BACKGROUND_OPACITY)
}

pub(super) fn history_prompt_active_background(foreground: Hsla) -> Hsla {
    foreground.opacity(HISTORY_PROMPT_ACTIVE_BACKGROUND_OPACITY)
}

fn estimate_history_prompt_dropdown_height(
    line_height: Pixels,
    match_count: usize,
    search_mode: bool,
) -> Pixels {
    let row_count = match_count.max(1) as f32;
    let rows_height = (line_height + px(HISTORY_PROMPT_DROPDOWN_ROW_PADDING_Y)) * row_count;
    let row_gaps = px(HISTORY_PROMPT_DROPDOWN_ROW_GAP) * (row_count - 1.0).max(0.0);
    let header_height = if search_mode {
        px(HISTORY_PROMPT_DROPDOWN_SEARCH_HEADER_HEIGHT + HISTORY_PROMPT_DROPDOWN_ROW_GAP)
    } else {
        px(0.0)
    };

    px(HISTORY_PROMPT_DROPDOWN_CONTAINER_PADDING_Y + HISTORY_PROMPT_DROPDOWN_BORDER_Y)
        + header_height
        + rows_height
        + row_gaps
}

pub(super) fn history_prompt_dropdown_origin(
    terminal_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    cursor_line: i32,
    cursor_col: usize,
    match_count: usize,
    search_mode: bool,
) -> Point<Pixels> {
    let cursor_left = terminal_bounds.origin.x + cell_width * cursor_col as f32;
    let cursor_top = terminal_bounds.origin.y + line_height * cursor_line as f32;
    let dropdown_width = (terminal_bounds.size.width
        - px(HISTORY_PROMPT_DROPDOWN_EDGE_PADDING * 2.0))
    .min(px(HISTORY_PROMPT_DROPDOWN_MAX_WIDTH))
    .max(px(HISTORY_PROMPT_DROPDOWN_MIN_WIDTH));
    let dropdown_height =
        estimate_history_prompt_dropdown_height(line_height, match_count, search_mode);
    let min_left = terminal_bounds.origin.x;
    let max_left = (terminal_bounds.right() - dropdown_width).max(min_left);
    let left = cursor_left.min(max_left).max(min_left);
    let below_top = cursor_top + line_height + px(HISTORY_PROMPT_DROPDOWN_GAP_Y);
    let min_top = terminal_bounds.origin.y;
    let max_top = (terminal_bounds.bottom() - dropdown_height).max(min_top);
    let fits_below = below_top + dropdown_height <= terminal_bounds.bottom();
    let preferred_above_top = cursor_top
        - dropdown_height
        - px(HISTORY_PROMPT_DROPDOWN_GAP_Y + HISTORY_PROMPT_DROPDOWN_INPUT_CLEARANCE);
    let top = if fits_below {
        below_top.min(max_top)
    } else {
        preferred_above_top.max(min_top).min(max_top)
    };

    Point::new(left, top)
}

pub(super) fn history_prompt_overlay_bounds(terminal_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(Point::new(px(0.0), px(0.0)), terminal_bounds.size)
}

#[cfg(test)]
mod tests {
    use super::{history_prompt_active_background, history_prompt_dropdown_background};
    use gpui::{Hsla, rgb};

    #[test]
    fn history_prompt_dropdown_applies_translucent_background() {
        let background: Hsla = rgb(0x1E1E1E).into();

        let dropdown = history_prompt_dropdown_background(background);

        assert_eq!(background.h, dropdown.h);
        assert_eq!(background.s, dropdown.s);
        assert_eq!(background.l, dropdown.l);
        assert!((dropdown.a - 0.72).abs() < f32::EPSILON);
    }

    #[test]
    fn history_prompt_active_row_remains_distinct_over_translucent_content() {
        let foreground: Hsla = rgb(0xE4E4E4).into();

        let active = history_prompt_active_background(foreground);

        assert_eq!(foreground.h, active.h);
        assert_eq!(foreground.s, active.s);
        assert_eq!(foreground.l, active.l);
        assert!((active.a - 0.32).abs() < f32::EPSILON);
    }
}
