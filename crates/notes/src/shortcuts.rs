use gpui::{Action, App, KeyBinding};
use markdown_editor::*;
use one_core::keybindings::rebind_keybindings;

const INPUT_CONTEXT: &str = "MarkdownEditor > Input";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesShortcutDescriptor {
    pub command_id: String,
    pub title: String,
    pub default_keys: Vec<String>,
}

macro_rules! bind_commands {
    ($bindings:expr, $cx:expr, $(($id:expr, $title:expr, $keys:expr, $action:expr),)*) => {
        $(append_bindings(&mut $bindings, $cx, $id, $keys, $action);)*
    };
}

macro_rules! collect_descriptors {
    ($descriptors:expr, $(($id:expr, $title:expr, $keys:expr, $action:expr),)*) => {
        $descriptors.extend([$(descriptor($id, $title, $keys)),*]);
    };
}

macro_rules! command_list {
    ($visitor:ident $(, $args:expr)*) => {
        $visitor! {
            $($args,)*
            ("edit.undo", "Undo", &["secondary-z"], UndoSourceEdit),
            ("edit.redo", "Redo", &["secondary-shift-z", "secondary-y"], RedoSourceEdit),
            ("edit.select_all", "Select All", &["secondary-a"], SelectAll),
            ("format.toggle_bold", "Bold", &["secondary-b"], ToggleBold),
            ("format.toggle_italic", "Italic", &["secondary-i"], ToggleItalic),
            ("format.toggle_underline", "Underline", &["secondary-u"], ToggleUnderline),
            ("format.toggle_strike", "Strikethrough", &["secondary-shift-x"], ToggleStrike),
            ("format.toggle_inline_code", "Inline Code", &["secondary-e"], ToggleInlineCode),
            ("block.set_paragraph", "Paragraph", &["secondary-0"], SetParagraph),
            ("block.set_heading_1", "Heading 1", &["secondary-1"], SetHeading1),
            ("block.set_heading_2", "Heading 2", &["secondary-2"], SetHeading2),
            ("block.set_heading_3", "Heading 3", &["secondary-3"], SetHeading3),
            ("block.set_heading_4", "Heading 4", &["secondary-4"], SetHeading4),
            ("block.set_heading_5", "Heading 5", &["secondary-5"], SetHeading5),
            ("block.set_heading_6", "Heading 6", &["secondary-6"], SetHeading6),
            ("block.toggle_bullet_list", "Bullet List", &["secondary-shift-8"], ToggleBulletList),
            ("block.toggle_ordered_list", "Ordered List", &["secondary-shift-7"], ToggleOrderedList),
            ("block.toggle_task_list", "Task List", &["secondary-shift-t"], ToggleTaskList),
            ("block.toggle_quote", "Quote", &["secondary-shift-q"], ToggleQuote),
            ("block.toggle_code", "Code Block", &["secondary-alt-c"], ToggleCodeBlock),
            ("block.move_up", "Move Block Up", &["secondary-shift-up"], MoveBlockUp),
            ("block.move_down", "Move Block Down", &["secondary-shift-down"], MoveBlockDown),
            ("block.duplicate_selected", "Duplicate Block", &["secondary-d"], DuplicateBlock),
            ("block.delete_current", "Delete Block", &["secondary-shift-backspace"], DeleteBlock),
        }
    };
}

pub fn init(cx: &mut App) {
    refresh(cx);
}

pub fn refresh(cx: &mut App) {
    let mut bindings = Vec::new();
    command_list!(bind_commands, bindings, cx);
    cx.bind_keys(bindings);
}

pub fn descriptors() -> Vec<NotesShortcutDescriptor> {
    let mut descriptors = Vec::new();
    command_list!(collect_descriptors, descriptors);
    descriptors
}

fn append_bindings<A: Action + Clone>(
    bindings: &mut Vec<KeyBinding>,
    cx: &App,
    id: &'static str,
    defaults: &'static [&'static str],
    action: A,
) {
    bindings.extend(rebind_keybindings(
        cx,
        id,
        defaults,
        Some(INPUT_CONTEXT),
        action,
    ));
}

fn descriptor(
    id: &'static str,
    title: &'static str,
    defaults: &'static [&'static str],
) -> NotesShortcutDescriptor {
    NotesShortcutDescriptor {
        command_id: id.to_owned(),
        title: title.to_owned(),
        default_keys: defaults.iter().map(|key| (*key).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    #[test]
    fn markdown_shortcuts_have_valid_unique_commands_and_keys() {
        let descriptors = descriptors();
        let mut ids = std::collections::HashSet::new();
        for descriptor in descriptors {
            assert!(ids.insert(descriptor.command_id));
            for key in descriptor.default_keys {
                Keystroke::parse(&key).unwrap();
            }
        }
    }
}
