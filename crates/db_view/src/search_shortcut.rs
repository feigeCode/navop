use gpui::{App, Context, Entity, KeyBinding, Window};
use gpui_component::input::InputState;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

pub const DB_SEARCH_CONTEXT: &str = "DbSearch";
const MACOS_SEARCH_SHORTCUT: &str = "cmd-f";
const OTHER_SEARCH_SHORTCUT: &str = "ctrl-f";
const MACOS_TABLE_QUERY_SHORTCUT: &str = "cmd-shift-enter";
const OTHER_TABLE_QUERY_SHORTCUT: &str = "ctrl-shift-enter";
const MACOS_TABLE_DESIGNER_SHORTCUT: &str = "cmd-shift-d";
const OTHER_TABLE_DESIGNER_SHORTCUT: &str = "ctrl-shift-d";
const MACOS_SELECT_ALL_SHORTCUT: &str = "cmd-a";
const OTHER_SELECT_ALL_SHORTCUT: &str = "ctrl-a";
const REFRESH_SHORTCUT: &str = "f5";
const MACOS_SAVE_SHORTCUT: &str = "cmd-s";
const OTHER_SAVE_SHORTCUT: &str = "ctrl-s";

gpui::actions!(
    db_search,
    [
        FocusSearchInput,
        OpenSelectedTableQuery,
        OpenTableDesigner,
        SelectAllObjects,
        RefreshDatabasePanel,
        SaveDatabaseChanges
    ]
);

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

fn default_table_designer_shortcuts() -> [&'static str; 1] {
    default_table_designer_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_select_all_shortcuts() -> [&'static str; 1] {
    default_select_all_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_refresh_shortcuts() -> [&'static str; 1] {
    [REFRESH_SHORTCUT]
}

fn default_save_shortcuts() -> [&'static str; 1] {
    default_save_shortcuts_for_platform(cfg!(target_os = "macos"))
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

fn default_table_designer_shortcuts_for_platform(is_macos: bool) -> [&'static str; 1] {
    if is_macos {
        [MACOS_TABLE_DESIGNER_SHORTCUT]
    } else {
        [OTHER_TABLE_DESIGNER_SHORTCUT]
    }
}

fn default_select_all_shortcuts_for_platform(is_macos: bool) -> [&'static str; 1] {
    if is_macos {
        [MACOS_SELECT_ALL_SHORTCUT]
    } else {
        [OTHER_SELECT_ALL_SHORTCUT]
    }
}

fn default_save_shortcuts_for_platform(is_macos: bool) -> [&'static str; 1] {
    if is_macos {
        [MACOS_SAVE_SHORTCUT]
    } else {
        [OTHER_SAVE_SHORTCUT]
    }
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings: Vec<KeyBinding> =
        shortcuts_for(cx, action_id::DB_FOCUS_SEARCH, &default_search_shortcuts())
            .into_iter()
            .map(|key| {
                one_core::keybindings::load_dynamic_keybinding(
                    &key,
                    FocusSearchInput,
                    Some(DB_SEARCH_CONTEXT),
                    cx,
                )
            })
            .collect();

    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::DB_OPEN_TABLE_QUERY,
            &default_table_query_shortcuts(),
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(
                &key,
                OpenSelectedTableQuery,
                Some(DB_SEARCH_CONTEXT),
                cx,
            )
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::DB_OPEN_TABLE_DESIGNER,
            &default_table_designer_shortcuts(),
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(
                &key,
                OpenTableDesigner,
                Some(DB_SEARCH_CONTEXT),
                cx,
            )
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::DB_SELECT_ALL_OBJECTS,
            &default_select_all_shortcuts(),
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(
                &key,
                SelectAllObjects,
                Some(DB_SEARCH_CONTEXT),
                cx,
            )
        }),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::DB_REFRESH, &default_refresh_shortcuts())
            .into_iter()
            .map(|key| {
                one_core::keybindings::load_dynamic_keybinding(
                    &key,
                    RefreshDatabasePanel,
                    Some(DB_SEARCH_CONTEXT),
                    cx,
                )
            }),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::DB_SAVE_CHANGES, &default_save_shortcuts())
            .into_iter()
            .map(|key| {
                one_core::keybindings::load_dynamic_keybinding(
                    &key,
                    SaveDatabaseChanges,
                    Some(DB_SEARCH_CONTEXT),
                    cx,
                )
            }),
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
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::DB_OPEN_TABLE_DESIGNER,
        &default_table_designer_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        OpenTableDesigner,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::DB_SELECT_ALL_OBJECTS,
        &default_select_all_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        SelectAllObjects,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::DB_REFRESH,
        &default_refresh_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        RefreshDatabasePanel,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::DB_SAVE_CHANGES,
        &default_save_shortcuts(),
        Some(DB_SEARCH_CONTEXT),
        SaveDatabaseChanges,
    ));
    keybindings
}

#[cfg(test)]
mod tests {
    use super::{
        default_refresh_shortcuts, default_save_shortcuts_for_platform,
        default_search_shortcuts_for_platform, default_select_all_shortcuts_for_platform,
        default_table_designer_shortcuts_for_platform, default_table_query_shortcuts_for_platform,
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

    #[test]
    fn object_list_select_all_uses_cmd_a_on_macos() {
        assert_eq!(["cmd-a"], default_select_all_shortcuts_for_platform(true));
    }

    #[test]
    fn object_list_select_all_uses_ctrl_a_on_windows_and_linux() {
        assert_eq!(["ctrl-a"], default_select_all_shortcuts_for_platform(false));
    }

    #[test]
    fn table_designer_uses_shift_d_shortcuts() {
        assert_eq!(
            ["cmd-shift-d"],
            default_table_designer_shortcuts_for_platform(true)
        );
        assert_eq!(
            ["ctrl-shift-d"],
            default_table_designer_shortcuts_for_platform(false)
        );
    }

    #[test]
    fn database_refresh_uses_f5() {
        assert_eq!(["f5"], default_refresh_shortcuts());
    }

    #[test]
    fn database_save_uses_cmd_s_on_macos_and_ctrl_s_elsewhere() {
        assert_eq!(["cmd-s"], default_save_shortcuts_for_platform(true));
        assert_eq!(["ctrl-s"], default_save_shortcuts_for_platform(false));
    }
}
