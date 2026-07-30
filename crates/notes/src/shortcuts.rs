use gpui::{Action, App, KeyBinding};
use markdown_editor::{
    BoldSelection, CodeSelection, ItalicSelection, Redo, SelectAll, UnderlineSelection, Undo,
};
use one_core::keybindings::rebind_keybindings;

const INPUT_CONTEXT: &str = "BlockEditor";

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
            ("edit.undo", "Undo", &["secondary-z"], Undo),
            ("edit.redo", "Redo", &["secondary-shift-z", "secondary-y"], Redo),
            ("edit.select_all", "Select All", &["secondary-a"], SelectAll),
            ("format.toggle_bold", "Bold", &["secondary-b"], BoldSelection),
            ("format.toggle_italic", "Italic", &["secondary-i"], ItalicSelection),
            ("format.toggle_underline", "Underline", &["secondary-u"], UnderlineSelection),
            ("format.toggle_inline_code", "Inline Code", &["secondary-e"], CodeSelection),
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
