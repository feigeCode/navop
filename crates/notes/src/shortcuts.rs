use cditor_app::{
    CditorCommand, CditorCommandAction, CditorKeyBinding, CommandDescriptor, bind_command_keys,
    init_for_external_keymap,
};
use gpui::{App, KeyBinding};
use one_core::keybindings::{rebind_keybindings, shortcuts_for};

const CDITOR_KEY_CONTEXT: &str = "CditorEditor";

const DEFAULT_SHORTCUTS: &[(&str, &[&str])] = &[
    ("edit.undo", &["secondary-z"]),
    ("edit.redo", &["secondary-shift-z", "secondary-y"]),
    ("edit.select_all", &["secondary-a"]),
    ("format.toggle_bold", &["secondary-b"]),
    ("format.toggle_italic", &["secondary-i"]),
    ("format.toggle_underline", &["secondary-u"]),
    ("format.toggle_strike", &["secondary-shift-x"]),
    ("format.toggle_inline_code", &["secondary-e"]),
    ("block.set_paragraph", &["secondary-0"]),
    ("block.set_heading_1", &["secondary-1"]),
    ("block.set_heading_2", &["secondary-2"]),
    ("block.set_heading_3", &["secondary-3"]),
    ("block.set_heading_4", &["secondary-4"]),
    ("block.set_heading_5", &["secondary-5"]),
    ("block.set_heading_6", &["secondary-6"]),
    ("block.toggle_bullet_list", &["secondary-shift-8"]),
    ("block.toggle_ordered_list", &["secondary-shift-7"]),
    ("block.toggle_task_list", &["secondary-shift-t"]),
    ("block.toggle_quote", &["secondary-shift-q"]),
    ("block.toggle_code", &["secondary-alt-c"]),
    ("block.duplicate_selected", &["secondary-d"]),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesShortcutDescriptor {
    pub command_id: String,
    pub title: String,
    pub default_keys: Vec<String>,
}

pub fn init(cx: &mut App) {
    init_for_external_keymap(cx);
    if let Err(error) = bind_command_keys(cx, configured_bindings(cx)) {
        tracing::error!(%error, "failed to bind Notes editor shortcuts");
    }
}

pub fn refresh(cx: &mut App) {
    let mut keybindings = Vec::new();
    for descriptor in CditorCommand::shortcut_descriptors() {
        keybindings.extend(refreshable_bindings(&descriptor, cx));
    }
    cx.bind_keys(keybindings);
}

pub fn descriptors() -> Vec<NotesShortcutDescriptor> {
    CditorCommand::shortcut_descriptors()
        .into_iter()
        .map(|descriptor| NotesShortcutDescriptor {
            default_keys: default_keys(&descriptor.id)
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            command_id: descriptor.id,
            title: descriptor.title,
        })
        .collect()
}

fn configured_bindings(cx: &App) -> Vec<CditorKeyBinding> {
    descriptors()
        .into_iter()
        .flat_map(|descriptor| {
            shortcuts_for(
                cx,
                &descriptor.command_id,
                &default_keys(&descriptor.command_id),
            )
            .into_iter()
            .map(move |key| CditorKeyBinding::new(key, descriptor.command_id.clone()))
        })
        .collect()
}

fn refreshable_bindings(descriptor: &CommandDescriptor, cx: &App) -> Vec<KeyBinding> {
    rebind_keybindings(
        cx,
        &descriptor.id,
        default_keys(&descriptor.id),
        Some(CDITOR_KEY_CONTEXT),
        CditorCommandAction::new(&descriptor.id),
    )
}

fn default_keys(command_id: &str) -> &'static [&'static str] {
    DEFAULT_SHORTCUTS
        .iter()
        .find_map(|(id, keys)| (*id == command_id).then_some(*keys))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    #[test]
    fn default_shortcuts_use_valid_public_commands_and_keystrokes() {
        for (command_id, shortcuts) in DEFAULT_SHORTCUTS {
            assert!(CditorCommand::from_stable_id(command_id).is_some());
            for shortcut in *shortcuts {
                for key in shortcut.split_whitespace() {
                    Keystroke::parse(key).unwrap();
                }
            }
        }
    }

    #[test]
    fn descriptors_follow_cditor_catalog() {
        let expected = CditorCommand::shortcut_descriptors();
        let actual = descriptors();
        assert_eq!(expected.len(), actual.len());
        assert_eq!(expected[0].id, actual[0].command_id);
    }

    #[test]
    fn configurable_defaults_leave_core_input_keys_to_cditor() {
        let reserved = ["enter", "secondary-enter", "tab", "shift-tab"];
        for (_, shortcuts) in DEFAULT_SHORTCUTS {
            assert!(
                shortcuts
                    .iter()
                    .all(|shortcut| !reserved.contains(shortcut))
            );
        }
    }
}
