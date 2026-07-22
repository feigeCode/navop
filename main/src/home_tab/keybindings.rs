use super::*;

pub(crate) const OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS: &str = "cmd-alt-t";
pub(crate) const OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER: &str = "alt-t";

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let quick_open_default = home_default_shortcut("cmd-o", "alt-o");
    let new_connection_default = home_default_shortcut("cmd-n", "alt-n");
    let mut keybindings = Vec::new();
    keybindings.extend(
        shortcuts_for(cx, action_id::HOME_QUICK_OPEN, &[quick_open_default])
            .into_iter()
            .map(|key| KeyBinding::new(&key, OpenConnectionQuickOpen, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::HOME_NEW_CONNECTION,
            &[new_connection_default],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, NewConnectionShortcut, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::HOME_OPEN_LOCAL_TERMINAL,
            &[open_local_terminal_default_shortcut()],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, OpenLocalTerminalShortcut, None)),
    );
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::HOME_QUICK_OPEN,
        &[home_default_shortcut("cmd-o", "alt-o")],
        None,
        OpenConnectionQuickOpen,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::HOME_NEW_CONNECTION,
        &[home_default_shortcut("cmd-n", "alt-n")],
        None,
        NewConnectionShortcut,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::HOME_OPEN_LOCAL_TERMINAL,
        &[open_local_terminal_default_shortcut()],
        None,
        OpenLocalTerminalShortcut,
    ));
    keybindings
}

fn home_default_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}

fn open_local_terminal_default_shortcut() -> &'static str {
    home_default_shortcut(
        OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS,
        OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER,
    )
}
