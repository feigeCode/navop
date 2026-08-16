use notes::NotesShortcutDescriptor;
use rust_i18n::t;

pub(super) fn command_label(descriptor: &NotesShortcutDescriptor) -> String {
    command_translation_key(&descriptor.command_id)
        .map(|key| t!(key).to_string())
        .unwrap_or_else(|| descriptor.title.clone())
}

fn command_translation_key(command_id: &str) -> Option<&'static str> {
    EDIT_COMMAND_KEYS
        .iter()
        .chain(NAVIGATION_COMMAND_KEYS)
        .chain(SELECTION_COMMAND_KEYS)
        .chain(FORMAT_COMMAND_KEYS)
        .chain(BLOCK_COMMAND_KEYS)
        .chain(VIEW_COMMAND_KEYS)
        .find_map(|(id, key)| (*id == command_id).then_some(*key))
}

const EDIT_COMMAND_KEYS: &[(&str, &str)] = &[
    ("edit.undo", "Settings.Shortcuts.notes_undo"),
    ("edit.redo", "Settings.Shortcuts.notes_redo"),
    ("edit.select_all", "Settings.Shortcuts.notes_select_all"),
    ("edit.newline", "Settings.Shortcuts.notes_newline"),
    (
        "edit.delete_backward",
        "Settings.Shortcuts.notes_delete_backward",
    ),
    (
        "edit.delete_forward",
        "Settings.Shortcuts.notes_delete_forward",
    ),
    (
        "edit.delete_word_backward",
        "Settings.Shortcuts.notes_delete_word_backward",
    ),
    (
        "edit.delete_word_forward",
        "Settings.Shortcuts.notes_delete_word_forward",
    ),
    ("edit.copy", "Settings.Shortcuts.notes_copy"),
    ("edit.cut", "Settings.Shortcuts.notes_cut"),
    ("edit.paste", "Settings.Shortcuts.notes_paste"),
];

const NAVIGATION_COMMAND_KEYS: &[(&str, &str)] = &[
    (
        "navigation.focus_previous",
        "Settings.Shortcuts.notes_focus_previous",
    ),
    (
        "navigation.focus_next",
        "Settings.Shortcuts.notes_focus_next",
    ),
    ("navigation.move_left", "Settings.Shortcuts.notes_move_left"),
    (
        "navigation.move_right",
        "Settings.Shortcuts.notes_move_right",
    ),
    (
        "navigation.move_word_left",
        "Settings.Shortcuts.notes_move_word_left",
    ),
    (
        "navigation.move_word_right",
        "Settings.Shortcuts.notes_move_word_right",
    ),
    (
        "navigation.home",
        "Settings.Shortcuts.notes_move_line_start",
    ),
    ("navigation.end", "Settings.Shortcuts.notes_move_line_end"),
    (
        "navigation.block_up",
        "Settings.Shortcuts.notes_focus_previous_block",
    ),
    (
        "navigation.block_down",
        "Settings.Shortcuts.notes_focus_next_block",
    ),
    ("navigation.page_up", "Settings.Shortcuts.notes_page_up"),
    ("navigation.page_down", "Settings.Shortcuts.notes_page_down"),
    (
        "navigation.jump_to_top",
        "Settings.Shortcuts.notes_jump_to_top",
    ),
    (
        "navigation.jump_to_bottom",
        "Settings.Shortcuts.notes_jump_to_bottom",
    ),
];

const SELECTION_COMMAND_KEYS: &[(&str, &str)] = &[
    (
        "selection.extend_left",
        "Settings.Shortcuts.notes_extend_selection_left",
    ),
    (
        "selection.extend_right",
        "Settings.Shortcuts.notes_extend_selection_right",
    ),
    (
        "selection.extend_word_left",
        "Settings.Shortcuts.notes_extend_selection_word_left",
    ),
    (
        "selection.extend_word_right",
        "Settings.Shortcuts.notes_extend_selection_word_right",
    ),
    (
        "selection.extend_home",
        "Settings.Shortcuts.notes_extend_selection_home",
    ),
    (
        "selection.extend_end",
        "Settings.Shortcuts.notes_extend_selection_end",
    ),
];

const FORMAT_COMMAND_KEYS: &[(&str, &str)] = &[
    ("format.toggle_bold", "Settings.Shortcuts.notes_bold"),
    ("format.toggle_italic", "Settings.Shortcuts.notes_italic"),
    (
        "format.toggle_underline",
        "Settings.Shortcuts.notes_underline",
    ),
    ("format.toggle_strike", "Settings.Shortcuts.notes_strike"),
    (
        "format.toggle_inline_code",
        "Settings.Shortcuts.notes_inline_code",
    ),
];

const BLOCK_COMMAND_KEYS: &[(&str, &str)] = &[
    ("block.set_paragraph", "Settings.Shortcuts.notes_paragraph"),
    ("block.set_heading_1", "Settings.Shortcuts.notes_heading_1"),
    ("block.set_heading_2", "Settings.Shortcuts.notes_heading_2"),
    ("block.set_heading_3", "Settings.Shortcuts.notes_heading_3"),
    ("block.set_heading_4", "Settings.Shortcuts.notes_heading_4"),
    ("block.set_heading_5", "Settings.Shortcuts.notes_heading_5"),
    ("block.set_heading_6", "Settings.Shortcuts.notes_heading_6"),
    (
        "block.toggle_bullet_list",
        "Settings.Shortcuts.notes_bullet_list",
    ),
    (
        "block.toggle_ordered_list",
        "Settings.Shortcuts.notes_ordered_list",
    ),
    (
        "block.toggle_task_list",
        "Settings.Shortcuts.notes_task_list",
    ),
    ("block.toggle_quote", "Settings.Shortcuts.notes_quote"),
    ("block.toggle_code", "Settings.Shortcuts.notes_code_block"),
    ("block.move_up", "Settings.Shortcuts.notes_move_block_up"),
    (
        "block.move_down",
        "Settings.Shortcuts.notes_move_block_down",
    ),
    (
        "block.duplicate_selected",
        "Settings.Shortcuts.notes_duplicate_blocks",
    ),
    (
        "block.delete_current",
        "Settings.Shortcuts.notes_delete_block",
    ),
    ("block.indent", "Settings.Shortcuts.notes_indent"),
    ("block.outdent", "Settings.Shortcuts.notes_outdent"),
    (
        "block.exit_code_block",
        "Settings.Shortcuts.notes_exit_code_block",
    ),
];

const VIEW_COMMAND_KEYS: &[(&str, &str)] = &[
    (
        "view.dismiss_transient_ui",
        "Settings.Shortcuts.notes_dismiss_transient_ui",
    ),
    (
        "view.toggle_source_mode",
        "Settings.Shortcuts.notes_toggle_source_mode",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_markdown_command_has_a_label() {
        for descriptor in notes::shortcut_descriptors() {
            assert!(
                command_translation_key(&descriptor.command_id).is_some(),
                "missing translation key for {}",
                descriptor.command_id
            );
            assert!(!command_label(&descriptor).is_empty());
        }
    }
}
