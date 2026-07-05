use alacritty_terminal::term::TermMode;
use gpui::{Bounds, Keystroke, MouseButton, Pixels, Point, px};
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
    connection_kind: TerminalConnectionKind,
    connection_id: Option<i64>,
) -> Option<TerminalHistoryScope> {
    match connection_kind {
        TerminalConnectionKind::Local => Some(TerminalHistoryScope::local()),
        TerminalConnectionKind::Ssh => connection_id.map(TerminalHistoryScope::ssh),
        TerminalConnectionKind::Serial => None,
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
    connection_kind: TerminalConnectionKind,
    command_running: bool,
    mode: TermMode,
    child_exited: Option<i32>,
) -> bool {
    connection_kind == TerminalConnectionKind::Local
        && child_exited.is_none()
        && (command_running || local_tui_application_active(mode))
}

pub(super) const HISTORY_PROMPT_DROPDOWN_MIN_WIDTH: f32 = 300.0;
pub(super) const HISTORY_PROMPT_DROPDOWN_MAX_WIDTH: f32 = 500.0;
const HISTORY_PROMPT_DROPDOWN_GAP_Y: f32 = 6.0;
const HISTORY_PROMPT_DROPDOWN_EDGE_PADDING: f32 = 8.0;
const HISTORY_PROMPT_DROPDOWN_ROW_PADDING_Y: f32 = 12.0;
const HISTORY_PROMPT_DROPDOWN_CONTAINER_PADDING_Y: f32 = 16.0;
const HISTORY_PROMPT_DROPDOWN_SEARCH_HEADER_HEIGHT: f32 = 20.0;

fn estimate_history_prompt_dropdown_height(
    line_height: Pixels,
    match_count: usize,
    search_mode: bool,
) -> Pixels {
    let row_count = match_count.max(1) as f32;
    let rows_height = (line_height + px(HISTORY_PROMPT_DROPDOWN_ROW_PADDING_Y)) * row_count;
    let header_height = if search_mode {
        px(HISTORY_PROMPT_DROPDOWN_SEARCH_HEADER_HEIGHT)
    } else {
        px(0.0)
    };

    px(HISTORY_PROMPT_DROPDOWN_CONTAINER_PADDING_Y) + header_height + rows_height
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
    let preferred_above_top = cursor_top - dropdown_height - px(HISTORY_PROMPT_DROPDOWN_GAP_Y);
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
