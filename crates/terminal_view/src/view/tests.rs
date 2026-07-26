use super::{
    TERMINAL_RESET_FONT_SIZE, TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH, TerminalDuplicateSource,
    UnbracketedPasteHazard, WrappedLineSegment, block_selection_text_from_rows,
    clipboard_image_from_item, detect_unbracketed_paste_hazard, encode_mouse_modifiers,
    has_trailing_line_continuation, has_unterminated_shell_quote, history_prompt_available,
    history_prompt_dropdown_origin, history_prompt_overlay_bounds, mouse_button_code,
    multiline_non_empty_line_count, remote_clipboard_image_path, resolve_ssh_reconnect_source,
    sgr_mouse_button_report, sgr_mouse_mode_enabled, sgr_mouse_wheel_report,
    should_confirm_local_terminal_close, should_defer_inline_history_prompt_input_to_text_system,
    should_defer_sgr_left_press, should_direct_paste_on_right_click,
    should_dismiss_history_prompt_for_keystroke, should_dismiss_history_prompt_for_mouse,
    should_dismiss_history_prompt_for_scroll, should_extend_selection_on_shift_click,
    should_refresh_history_commands_for_terminal_event,
    should_reset_history_prompt_for_terminal_event, should_scroll_to_bottom_on_user_input,
    should_start_block_selection, should_start_selection_from_pending_sgr_press,
    should_upload_clipboard_image_to_remote_cli, take_whole_scroll_lines,
    terminal_duplicate_source_with_cwd, terminal_history_scope, terminal_paste_bytes,
    terminal_tab_duplicate_supported, wrapped_addon_line_text,
};
use crate::history_prompt::{HistoryPromptAccept, HistoryPromptState};
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::TermMode;
use gpui::{
    Bounds, ClipboardItem, Image, ImageFormat, Keystroke, Modifiers, MouseButton, Point, px, size,
};
use one_core::storage::models::{SerialParams, SshAuthMethod, SshParams, StoredConnection};
use std::cell::Cell as StdCell;
use terminal::LocalConfig;
use terminal::terminal::{TerminalConnectionKind, TerminalModelEvent};

mod core;
mod history_availability;
mod history_interaction;
mod layout;
mod mouse;
mod paste;
