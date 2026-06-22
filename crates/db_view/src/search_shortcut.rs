use gpui::{App, Context, Entity, KeyBinding, Window};
use gpui_component::input::InputState;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

pub const DB_SEARCH_CONTEXT: &str = "DbSearch";
const MACOS_SEARCH_SHORTCUT: &str = "cmd-f";
const OTHER_SEARCH_SHORTCUT: &str = "ctrl-f";
const MACOS_TABLE_QUERY_SHORTCUT: &str = "cmd-shift-enter";
const OTHER_TABLE_QUERY_SHORTCUT: &str = "ctrl-shift-enter";

gpui::actions!(db_search, [FocusSearchInput, OpenSelectedTableQuery]);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

pub fn focus_search_input<T>(input: &Entity<InputState>, window: &mut Window, cx: &mut Context<T>) {
    input.update(cx, |state, cx| {
        state.focus(window, cx);
    });
}

fn default_search_shortcuts() -> [&'static str; 1] {
    default_search_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_table_query_shortcuts() -> [&'static str; 1] {
    default_table_query_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_search_shortcuts_for_platform(is_macos: bool) -> [&'static str; 1] {
    if is_macos {
        [MACOS_SEARCH_SHORTCUT]
    } else {
        [OTHER_SEARCH_SHORTCUT]
    }
}

fn default_table_query_shortcuts_for_platform(is_macos: bool) -> [&'static str; 1] {
    if is_macos {
        [MACOS_TABLE_QUERY_SHORTCUT]
    } else {
        [OTHER_TABLE_QUERY_SHORTCUT]
    }
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings: Vec<KeyBinding> =
        shortcuts_for(cx, action_id::DB_FOCUS_SEARCH, &default_search_shortcuts())
            .into_iter()
            .map(|key| KeyBinding::new(&key, FocusSearchInput, Some(DB_SEARCH_CONTEXT)))
            .collect();

    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::DB_OPEN_TABLE_QUERY,
            &default_table_query_shortcuts(),
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, OpenSelectedTableQuery, Some(DB_SEARCH_CONTEXT))),
    );
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = rebind_keybindings(
        cx,
        action_id::DB_FOCUS_SEARCH,
        &default_search_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        FocusSearchInput,
    );
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::DB_OPEN_TABLE_QUERY,
        &default_table_query_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        OpenSelectedTableQuery,
    ));
    keybindings
}

#[cfg(test)]
mod tests {
    use super::{
        default_search_shortcuts_for_platform, default_table_query_shortcuts_for_platform,
    };

    #[test]
    fn db_search_uses_cmd_f_on_macos() {
        assert_eq!(["cmd-f"], default_search_shortcuts_for_platform(true));
    }

    #[test]
    fn db_search_uses_ctrl_f_on_windows_and_linux() {
        assert_eq!(["ctrl-f"], default_search_shortcuts_for_platform(false));
    }

    #[test]
    fn table_query_uses_shift_enter_shortcuts() {
        assert_eq!(
            ["cmd-shift-enter"],
            default_table_query_shortcuts_for_platform(true)
        );
        assert_eq!(
            ["ctrl-shift-enter"],
            default_table_query_shortcuts_for_platform(false)
        );
    }
}
