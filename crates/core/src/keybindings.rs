use std::collections::{HashMap, HashSet};

use gpui::{Action, App, KeyBinding, Keystroke, NoAction};

use crate::settings::AppSettings;

pub type KeyBindingOverrides = HashMap<String, Vec<String>>;

pub mod action_id {
    pub const WINDOW_TOGGLE_ZOOM: &str = "window.toggle_zoom";
    pub const WINDOW_CLOSE_PANEL: &str = "window.close_panel";
    pub const WINDOW_TOGGLE_FULLSCREEN: &str = "window.toggle_fullscreen";
    pub const WINDOW_TOGGLE_ALWAYS_ON_TOP: &str = "window.toggle_always_on_top";
    pub const APP_DUPLICATE_TAB: &str = "app.duplicate_tab";
    pub const APP_QUIT: &str = "app.quit";
    pub const HOME_QUICK_OPEN: &str = "home.quick_open";
    pub const HOME_NEW_CONNECTION: &str = "home.new_connection";
    pub const DB_FOCUS_SEARCH: &str = "db.focus_search";
    pub const DB_OPEN_TABLE_QUERY: &str = "db.open_table_query";
    pub const SQL_RUN_QUERY: &str = "sql.run_query";
    pub const SQL_RUN_ALL_QUERY: &str = "sql.run_all_query";
    pub const TERMINAL_SEND_TAB: &str = "terminal.send_tab";
    pub const TERMINAL_SEND_SHIFT_TAB: &str = "terminal.send_shift_tab";
    pub const TERMINAL_COPY: &str = "terminal.copy";
    pub const TERMINAL_PASTE: &str = "terminal.paste";
    pub const TERMINAL_SELECT_ALL: &str = "terminal.select_all";
    pub const TERMINAL_CLEAR_SCREEN: &str = "terminal.clear_screen";
    pub const TERMINAL_CLEAR_SELECTION: &str = "terminal.clear_selection";
    pub const TERMINAL_SEARCH_FORWARD: &str = "terminal.search_forward";
    pub const TERMINAL_SEARCH_BACKWARD: &str = "terminal.search_backward";
    pub const TERMINAL_TOGGLE_VI_MODE: &str = "terminal.toggle_vi_mode";
    pub const TERMINAL_INCREASE_FONT: &str = "terminal.increase_font";
    pub const TERMINAL_DECREASE_FONT: &str = "terminal.decrease_font";
    pub const TERMINAL_RESET_FONT: &str = "terminal.reset_font";
    pub const REMOTE_EDITOR_SEARCH: &str = "remote_editor.search";
    pub const REMOTE_EDITOR_REPLACE: &str = "remote_editor.replace";
    pub const REDIS_CLEAR_OUTPUT: &str = "redis.clear_output";
    pub const REDIS_COPY: &str = "redis.copy";
    pub const REDIS_PASTE: &str = "redis.paste";
    pub const REDIS_SELECT_ALL: &str = "redis.select_all";
    pub const REDIS_CLEAR_SELECTION: &str = "redis.clear_selection";
    pub const REDIS_COMPLETE_COMMAND: &str = "redis.complete_command";
    pub const TABLE_COPY: &str = "table.copy";
    pub const TABLE_PASTE: &str = "table.paste";
    pub const TABLE_SELECT_ALL: &str = "table.select_all";
    pub const TABLE_CANCEL: &str = "table.cancel";
}

pub fn shortcuts_for(cx: &App, action_id: &str, defaults: &[&str]) -> Vec<String> {
    let overrides = cx
        .try_global::<AppSettings>()
        .map(|settings| &settings.custom_keybindings);
    match overrides {
        Some(overrides) => resolve_shortcuts(overrides, action_id, defaults),
        None => defaults
            .iter()
            .map(|shortcut| shortcut.to_string())
            .collect(),
    }
}

pub fn keystroke_matches_shortcuts(keystroke: &Keystroke, shortcuts: &[String]) -> bool {
    shortcut_spec_from_keystroke(keystroke)
        .is_some_and(|spec| shortcuts.iter().any(|shortcut| shortcut == &spec))
}

pub fn rebind_keybindings<A>(
    cx: &App,
    action_id: &str,
    defaults: &[&str],
    context: Option<&str>,
    action: A,
) -> Vec<KeyBinding>
where
    A: Action + Clone,
{
    let current = shortcuts_for(cx, action_id, defaults);
    let active = cx
        .key_bindings()
        .borrow()
        .bindings_for_action(&action)
        .map(|binding| shortcut_spec_from_binding(binding))
        .collect::<Vec<_>>();
    let mut keybindings = rebind_shadow_shortcuts(defaults, &current, active)
        .into_iter()
        .map(|shortcut| KeyBinding::new(&shortcut, NoAction, context))
        .collect::<Vec<_>>();
    keybindings.extend(
        current
            .into_iter()
            .map(|shortcut| KeyBinding::new(&shortcut, action.clone(), context)),
    );
    keybindings
}

pub fn resolve_shortcuts(
    overrides: &KeyBindingOverrides,
    action_id: &str,
    defaults: &[&str],
) -> Vec<String> {
    let fallback = || {
        defaults
            .iter()
            .map(|shortcut| shortcut.to_string())
            .collect()
    };
    let Some(shortcuts) = overrides.get(action_id) else {
        return fallback();
    };
    if shortcuts.is_empty()
        || shortcuts
            .iter()
            .any(|shortcut| !is_valid_shortcut(shortcut))
    {
        return fallback();
    }
    shortcuts.clone()
}

fn is_valid_shortcut(shortcut: &str) -> bool {
    Keystroke::parse(shortcut).is_ok()
}

pub fn rebind_shadow_shortcuts(
    defaults: &[&str],
    current: &[String],
    active: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut shortcuts = Vec::new();
    for shortcut in defaults
        .iter()
        .map(|shortcut| shortcut.to_string())
        .chain(active)
        .chain(current.iter().cloned())
    {
        if is_valid_shortcut(&shortcut) && seen.insert(shortcut.clone()) {
            shortcuts.push(shortcut);
        }
    }
    shortcuts
}

fn shortcut_spec_from_binding(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn shortcut_spec_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.as_str();
    if matches!(key, "ctrl" | "control" | "alt" | "shift" | "cmd" | "win") {
        return None;
    }

    let mut tokens: Vec<&str> = Vec::with_capacity(5);
    if keystroke.modifiers.control {
        tokens.push("ctrl");
    }
    if keystroke.modifiers.alt {
        tokens.push("alt");
    }
    if keystroke.modifiers.shift {
        tokens.push("shift");
    }
    if keystroke.modifiers.platform {
        tokens.push("cmd");
    }
    tokens.push(key);
    Some(tokens.join("-"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gpui::{KeyBinding, KeyContext, Keymap, Keystroke, NoAction, actions};

    use super::{keystroke_matches_shortcuts, rebind_shadow_shortcuts, resolve_shortcuts};

    actions!(keybindings_tests, [TestAction]);

    #[test]
    fn resolve_shortcuts_uses_valid_override() {
        let overrides = HashMap::from([(
            "app.quit".to_string(),
            vec!["cmd-shift-q".to_string(), "ctrl-shift-q".to_string()],
        )]);

        assert_eq!(
            vec!["cmd-shift-q".to_string(), "ctrl-shift-q".to_string()],
            resolve_shortcuts(&overrides, "app.quit", &["cmd-q"])
        );
    }

    #[test]
    fn resolve_shortcuts_falls_back_for_empty_or_invalid_override() {
        let overrides = HashMap::from([
            ("app.quit".to_string(), Vec::<String>::new()),
            (
                "app.open".to_string(),
                vec!["cmd-not-a-real-key".to_string()],
            ),
        ]);

        assert_eq!(
            vec!["cmd-q".to_string()],
            resolve_shortcuts(&overrides, "app.quit", &["cmd-q"])
        );
        assert_eq!(
            vec!["cmd-o".to_string()],
            resolve_shortcuts(&overrides, "app.open", &["cmd-o"])
        );
    }

    #[test]
    fn rebind_shadow_shortcuts_collects_old_default_and_current_shortcuts() {
        let current = vec!["cmd-j".to_string()];
        let active = vec![
            "cmd-f".to_string(),
            "cmd-j".to_string(),
            "cmd-not-a-real-key".to_string(),
        ];

        assert_eq!(
            vec![
                "cmd-f".to_string(),
                "cmd-g".to_string(),
                "cmd-j".to_string(),
            ],
            rebind_shadow_shortcuts(&["cmd-f", "cmd-g"], &current, active)
        );
    }

    #[test]
    fn no_action_shadows_previous_shortcut_in_same_context() {
        let mut keymap = Keymap::default();
        keymap.add_bindings([
            KeyBinding::new("ctrl-a", TestAction, Some("Test")),
            KeyBinding::new("ctrl-a", NoAction, Some("Test")),
            KeyBinding::new("ctrl-b", TestAction, Some("Test")),
        ]);

        let context = [KeyContext::parse("Test").unwrap()];
        let (old_bindings, _) =
            keymap.bindings_for_input(&[Keystroke::parse("ctrl-a").unwrap()], &context);
        let (new_bindings, _) =
            keymap.bindings_for_input(&[Keystroke::parse("ctrl-b").unwrap()], &context);

        assert!(
            old_bindings.is_empty(),
            "old binding should be shadowed, got {old_bindings:?}"
        );
        assert_eq!(1, new_bindings.len());
        assert!(new_bindings[0].action().partial_eq(&TestAction));
    }

    #[test]
    fn keystroke_matching_uses_current_shortcut_specs() {
        let old = Keystroke::parse("ctrl-shift-c").unwrap();
        let new = Keystroke::parse("ctrl-alt-c").unwrap();
        let shortcuts = vec!["ctrl-alt-c".to_string()];

        assert!(!keystroke_matches_shortcuts(&old, &shortcuts));
        assert!(keystroke_matches_shortcuts(&new, &shortcuts));
    }
}
