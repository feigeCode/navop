use std::collections::{HashMap, HashSet};

use gpui::{Action, App, KeyBinding, Keystroke, NoAction, OperatingSystem};

use crate::settings::AppSettings;

pub type KeyBindingOverrides = HashMap<String, Vec<String>>;

pub mod action_id {
    pub const WINDOW_TOGGLE_ZOOM: &str = "window.toggle_zoom";
    pub const WINDOW_CLOSE_PANEL: &str = "window.close_panel";
    pub const WINDOW_CLOSE_ACTIVE_WINDOW: &str = WINDOW_CLOSE_PANEL;
    pub const WINDOW_TOGGLE_CONNECTION_SIDEBAR: &str = "window.toggle_connection_sidebar";
    pub const WINDOW_TOGGLE_FULLSCREEN: &str = "window.toggle_fullscreen";
    pub const WINDOW_TOGGLE_ALWAYS_ON_TOP: &str = "window.toggle_always_on_top";
    pub const APP_DUPLICATE_TAB: &str = "app.duplicate_tab";
    pub const APP_OPEN_TAB_SWITCHER: &str = "app.open_tab_switcher";
    pub const APP_SWITCH_NEXT_TAB: &str = "app.switch_next_tab";
    pub const APP_SWITCH_PREVIOUS_TAB: &str = "app.switch_previous_tab";
    pub const APP_CLOSE_ACTIVE_TAB: &str = "app.close_active_tab";
    pub const APP_QUIT: &str = "app.quit";
    pub const HOME_QUICK_OPEN: &str = "home.quick_open";
    pub const HOME_NEW_CONNECTION: &str = "home.new_connection";
    pub const HOME_OPEN_LOCAL_TERMINAL: &str = "home.open_local_terminal";
    pub const DB_FOCUS_SEARCH: &str = "db.focus_search";
    pub const DB_OPEN_TABLE_QUERY: &str = "db.open_table_query";
    pub const DB_OPEN_TABLE_DESIGNER: &str = "db.open_table_designer";
    pub const DB_SELECT_ALL_OBJECTS: &str = "db.select_all_objects";
    pub const DB_REFRESH: &str = "db.refresh";
    pub const DB_SAVE_CHANGES: &str = "db.save_changes";
    pub const SQL_RUN_QUERY: &str = "sql.run_query";
    pub const SQL_RUN_ALL_QUERY: &str = "sql.run_all_query";
    pub const SQL_TOGGLE_COMMENT: &str = "sql.toggle_comment";
    pub const SQL_RUN_CURSOR_STATEMENT: &str = "sql.run_cursor_statement";
    pub const SQL_DELETE_LINE: &str = "sql.delete_line";
    pub const SQL_DUPLICATE_LINE: &str = "sql.duplicate_line";
    pub const SQL_SWAP_LINE_UP: &str = "sql.swap_line_up";
    pub const SQL_UPPERCASE: &str = "sql.uppercase";
    pub const SQL_LOWERCASE: &str = "sql.lowercase";
    pub const SQL_EXECUTION_HISTORY: &str = "sql.execution_history";
    pub const TERMINAL_SEND_TAB: &str = "terminal.send_tab";
    pub const TERMINAL_SEND_SHIFT_TAB: &str = "terminal.send_shift_tab";
    pub const TERMINAL_SEND_CTRL_B: &str = "terminal.send_ctrl_b";
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
    pub const TERMINAL_SCROLL_LINE_UP: &str = "terminal.scroll_line_up";
    pub const TERMINAL_SCROLL_LINE_DOWN: &str = "terminal.scroll_line_down";
    pub const TERMINAL_SCROLL_PAGE_UP: &str = "terminal.scroll_page_up";
    pub const TERMINAL_SCROLL_PAGE_DOWN: &str = "terminal.scroll_page_down";
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
    pub const TABLE_FIND: &str = "table.find";
    pub const TABLE_FIND_NEXT: &str = "table.find_next";
    pub const TABLE_FIND_PREVIOUS: &str = "table.find_previous";
    pub const AI_CHAT_FIND: &str = "ai_chat.find";
    pub const AI_CHAT_FIND_NEXT: &str = "ai_chat.find_next";
    pub const AI_CHAT_FIND_PREVIOUS: &str = "ai_chat.find_previous";
    pub const AI_CHAT_SESSION_BACK: &str = "ai_chat.session_back";
    pub const AI_CHAT_SESSION_FORWARD: &str = "ai_chat.session_forward";
    pub const AI_CHAT_SESSION_SWITCHER: &str = "ai_chat.session_switcher";
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
    shortcut_spec_from_keystroke(keystroke).is_some_and(|spec| {
        shortcuts.iter().any(|shortcut| {
            Keystroke::parse_with_os(shortcut, current_os())
                .ok()
                .and_then(|shortcut| shortcut_spec_from_keystroke(&shortcut))
                .is_some_and(|shortcut| shortcut == spec)
        })
    })
}

/// 动态字符串构造 KeyBinding（gpui-fast v0.1.12 起 KeyBinding::new 只收 &'static str）。
pub fn load_dynamic_keybinding<A: Action>(
    keystrokes: &str,
    action: A,
    context: Option<&str>,
    cx: &App,
) -> KeyBinding {
    let context_predicate = context
        .map(|context| gpui::KeyBindingContextPredicate::parse(context).unwrap().into());
    KeyBinding::load(
        keystrokes,
        Box::new(action),
        context_predicate,
        true,
        None,
        cx,
    )
    .unwrap()
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
        .map(|binding| shortcut_spec_from_binding(binding, cx.platform_info().os))
        .collect::<Vec<_>>();
    let mut keybindings = rebind_shadow_shortcuts(defaults, &current, active)
        .into_iter()
        .map(|shortcut| load_dynamic_keybinding(&shortcut, NoAction, context, cx))
        .collect::<Vec<_>>();
    keybindings.extend(
        current
            .into_iter()
            .map(|shortcut| load_dynamic_keybinding(&shortcut, action.clone(), context, cx)),
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
    if shortcuts.is_empty() {
        // 空 override 表示用户显式清空/禁用该快捷键，不再回退默认值。
        return Vec::new();
    }
    if shortcuts
        .iter()
        .any(|shortcut| !is_valid_shortcut(shortcut))
    {
        return fallback();
    }
    shortcuts.clone()
}

fn is_valid_shortcut(shortcut: &str) -> bool {
    Keystroke::parse_with_os(shortcut, current_os()).is_ok()
}

/// 编译目标对应的 gpui `OperatingSystem`（gpui-fast v0.1.12 起动态解析键位需要）。
pub fn current_os() -> OperatingSystem {
    match std::env::consts::OS {
        "macos" => OperatingSystem::Mac,
        "windows" => OperatingSystem::Windows,
        "linux" => OperatingSystem::Linux,
        "freebsd" => OperatingSystem::FreeBsd,
        "ios" => OperatingSystem::Ios,
        "android" => OperatingSystem::Android,
        _ => OperatingSystem::Unknown,
    }
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

fn shortcut_spec_from_binding(binding: &KeyBinding, os: OperatingSystem) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke.display(os).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn shortcut_spec_from_keystroke(keystroke: &Keystroke) -> Option<String> {
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

    use super::{
        keystroke_matches_shortcuts, rebind_shadow_shortcuts, resolve_shortcuts,
        shortcut_spec_from_keystroke,
    };

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
    fn resolve_shortcuts_empty_override_means_disabled() {
        let overrides = HashMap::from([("app.quit".to_string(), Vec::<String>::new())]);

        assert!(resolve_shortcuts(&overrides, "app.quit", &["cmd-q"]).is_empty());
    }

    #[test]
    fn resolve_shortcuts_falls_back_for_invalid_override() {
        let overrides = HashMap::from([(
            "app.open".to_string(),
            vec!["cmd-not-a-real-key".to_string()],
        )]);

        assert_eq!(
            vec!["cmd-o".to_string()],
            resolve_shortcuts(&overrides, "app.open", &["cmd-o"])
        );
    }

    #[test]
    fn resolve_shortcuts_supports_custom_shortcuts_with_empty_defaults() {
        let overrides = HashMap::from([(
            "terminal.clear_selection".to_string(),
            vec!["ctrl-x".to_string()],
        )]);

        assert_eq!(
            vec!["ctrl-x".to_string()],
            resolve_shortcuts(&overrides, "terminal.clear_selection", &[])
        );
        assert!(resolve_shortcuts(&HashMap::new(), "terminal.clear_selection", &[]).is_empty());
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
    fn rebind_shadow_shortcuts_supports_empty_defaults_with_custom_shortcut() {
        assert_eq!(
            vec!["escape".to_string(), "ctrl-x".to_string()],
            rebind_shadow_shortcuts(&[], &["ctrl-x".to_string()], ["escape".to_string()])
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

    #[test]
    fn shortcut_matching_normalizes_stored_specs_and_rejects_invalid_specs() {
        let noncanonical = vec!["alt-ctrl-c".to_string()];
        let invalid = vec!["cmd-not-a-real-key".to_string(), "ctrl".to_string()];

        assert!(keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl-alt-c").expect("parse shortcut"),
            &noncanonical
        ));
        assert!(!keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl-c").expect("parse shortcut"),
            &invalid
        ));
        assert!(!keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl").expect("parse modifier"),
            &invalid
        ));
    }

    #[test]
    fn shortcut_spec_normalizes_modifier_order_and_rejects_modifier_only_keys() {
        assert_eq!(
            Some("ctrl-alt-shift-cmd-k".to_string()),
            shortcut_spec_from_keystroke(
                &Keystroke::parse("cmd-shift-alt-ctrl-k").expect("parse shortcut")
            )
        );
        assert_eq!(
            None,
            shortcut_spec_from_keystroke(&Keystroke::parse("ctrl").expect("parse modifier"))
        );
    }

    #[test]
    fn shortcut_matching_requires_the_complete_modifier_set() {
        let shortcuts = vec!["ctrl-alt-c".to_string()];

        assert!(keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl-alt-c").expect("parse exact shortcut"),
            &shortcuts
        ));
        assert!(!keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl-c").expect("parse missing modifier"),
            &shortcuts
        ));
        assert!(!keystroke_matches_shortcuts(
            &Keystroke::parse("ctrl-alt-shift-c").expect("parse extra modifier"),
            &shortcuts
        ));
    }
}
