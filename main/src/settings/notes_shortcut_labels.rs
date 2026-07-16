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
        .chain(FORMAT_COMMAND_KEYS)
        .chain(BLOCK_COMMAND_KEYS)
        .chain(STRUCTURE_COMMAND_KEYS)
        .find_map(|(id, key)| (*id == command_id).then_some(*key))
}

const EDIT_COMMAND_KEYS: &[(&str, &str)] = &[
    ("edit.undo", "Settings.Shortcuts.notes_undo"),
    ("edit.redo", "Settings.Shortcuts.notes_redo"),
    ("edit.select_all", "Settings.Shortcuts.notes_select_all"),
    (
        "edit.delete_selection",
        "Settings.Shortcuts.notes_delete_selection",
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
    ("block.toggle_callout", "Settings.Shortcuts.notes_callout"),
    ("block.toggle_toggle", "Settings.Shortcuts.notes_toggle"),
    ("block.toggle_code", "Settings.Shortcuts.notes_code_block"),
    ("block.toggle_math", "Settings.Shortcuts.notes_math_block"),
    ("block.toggle_mermaid", "Settings.Shortcuts.notes_mermaid"),
    (
        "block.toggle_todo_checked",
        "Settings.Shortcuts.notes_todo_checked",
    ),
];

const STRUCTURE_COMMAND_KEYS: &[(&str, &str)] = &[
    (
        "block.insert_paragraph_after",
        "Settings.Shortcuts.notes_insert_below",
    ),
    ("block.indent", "Settings.Shortcuts.notes_indent"),
    ("block.outdent", "Settings.Shortcuts.notes_outdent"),
    (
        "block.delete_current",
        "Settings.Shortcuts.notes_delete_block",
    ),
    (
        "block.delete_selected",
        "Settings.Shortcuts.notes_delete_blocks",
    ),
    (
        "block.duplicate_selected",
        "Settings.Shortcuts.notes_duplicate_blocks",
    ),
    ("heading.fold", "Settings.Shortcuts.notes_fold_heading"),
    ("heading.unfold", "Settings.Shortcuts.notes_unfold_heading"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_cditor_command_has_a_localized_label() {
        for descriptor in notes::shortcut_descriptors() {
            assert!(command_translation_key(&descriptor.command_id).is_some());
        }
    }
}
