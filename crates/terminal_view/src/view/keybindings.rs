use super::*;

pub(super) const TERMINAL_CONTEXT: &str = "TerminalView";

#[cfg(target_os = "macos")]
pub(super) const TERMINAL_COPY_SHORTCUT: &str = "cmd-c";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_COPY_SHORTCUT: &str = "ctrl-shift-c";
#[cfg(target_os = "macos")]
pub(super) const TERMINAL_PASTE_SHORTCUT: &str = "cmd-v";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_PASTE_SHORTCUT: &str = "ctrl-shift-v";
#[cfg(target_os = "macos")]
pub(super) const TERMINAL_SELECT_ALL_SHORTCUT: &str = "cmd-a";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_SELECT_ALL_SHORTCUT: &str = "ctrl-shift-a";
#[cfg(target_os = "macos")]
pub(super) const TERMINAL_CLEAR_SCREEN_SHORTCUT: &str = "cmd-k";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_CLEAR_SCREEN_SHORTCUT: &str = "ctrl-l";
#[cfg(target_os = "macos")]
pub(super) const TERMINAL_SEARCH_FORWARD_SHORTCUT: &str = "cmd-f";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_SEARCH_FORWARD_SHORTCUT: &str = "ctrl-shift-f";
#[cfg(target_os = "macos")]
pub(super) const TERMINAL_SEARCH_BACKWARD_SHORTCUT: &str = "cmd-g";
#[cfg(not(target_os = "macos"))]
pub(super) const TERMINAL_SEARCH_BACKWARD_SHORTCUT: &str = "ctrl-shift-g";
pub(super) const TERMINAL_TOGGLE_VI_MODE_SHORTCUT: &str = "f7";
pub(super) fn terminal_shortcut_label(shortcut: &str) -> SharedString {
    Kbd::format(&Keystroke::parse(shortcut).expect("终端快捷键定义非法")).into()
}

/// 对路径进行简单 shell 转义（用单引号包裹，处理内部单引号）
pub fn init(cx: &mut App) {
    crate::settings::init_settings(cx);
    crate::public_mcp::init(cx);
    init_broadcast_input_registry(cx);
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

pub(super) fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(crate::sidebar::file_manager_panel::init_keybindings());
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_SEND_TAB, &["tab"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, SendTab, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_SEND_SHIFT_TAB, &["shift-tab"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, SendShiftTab, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_COPY, &[TERMINAL_COPY_SHORTCUT])
            .into_iter()
            .map(|key| KeyBinding::new(&key, Copy, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_PASTE, &terminal_paste_defaults())
            .into_iter()
            .map(|key| KeyBinding::new(&key, Paste, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SELECT_ALL,
            &[TERMINAL_SELECT_ALL_SHORTCUT],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, SelectAll, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_CLEAR_SCREEN,
            &[TERMINAL_CLEAR_SCREEN_SHORTCUT],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ClearScreen, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_CLEAR_SELECTION, &["escape"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, ClearSelection, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SEARCH_FORWARD,
            &[TERMINAL_SEARCH_FORWARD_SHORTCUT],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, SearchForward, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SEARCH_BACKWARD,
            &[TERMINAL_SEARCH_BACKWARD_SHORTCUT],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, SearchBackward, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_TOGGLE_VI_MODE,
            &[TERMINAL_TOGGLE_VI_MODE_SHORTCUT],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ToggleViMode, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_INCREASE_FONT,
            &terminal_increase_font_defaults(),
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, IncreaseFont, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_DECREASE_FONT,
            &[terminal_platform_shortcut("cmd--", "ctrl--")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, DecreaseFont, Some(TERMINAL_CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_RESET_FONT,
            &[terminal_platform_shortcut("cmd-0", "ctrl-0")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ResetFont, Some(TERMINAL_CONTEXT))),
    );
    keybindings
}

pub(super) fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(crate::sidebar::file_manager_panel::init_keybindings());
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SEND_TAB,
        &["tab"],
        Some(TERMINAL_CONTEXT),
        SendTab,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SEND_SHIFT_TAB,
        &["shift-tab"],
        Some(TERMINAL_CONTEXT),
        SendShiftTab,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_COPY,
        &[TERMINAL_COPY_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        Copy,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_PASTE,
        &terminal_paste_defaults(),
        Some(TERMINAL_CONTEXT),
        Paste,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SELECT_ALL,
        &[TERMINAL_SELECT_ALL_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        SelectAll,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_CLEAR_SCREEN,
        &[TERMINAL_CLEAR_SCREEN_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ClearScreen,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_CLEAR_SELECTION,
        &["escape"],
        Some(TERMINAL_CONTEXT),
        ClearSelection,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SEARCH_FORWARD,
        &[TERMINAL_SEARCH_FORWARD_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        SearchForward,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SEARCH_BACKWARD,
        &[TERMINAL_SEARCH_BACKWARD_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        SearchBackward,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_TOGGLE_VI_MODE,
        &[TERMINAL_TOGGLE_VI_MODE_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ToggleViMode,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_INCREASE_FONT,
        &terminal_increase_font_defaults(),
        Some(TERMINAL_CONTEXT),
        IncreaseFont,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_DECREASE_FONT,
        &[terminal_platform_shortcut("cmd--", "ctrl--")],
        Some(TERMINAL_CONTEXT),
        DecreaseFont,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_RESET_FONT,
        &[terminal_platform_shortcut("cmd-0", "ctrl-0")],
        Some(TERMINAL_CONTEXT),
        ResetFont,
    ));
    keybindings
}

pub(super) fn terminal_paste_defaults() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec![TERMINAL_PASTE_SHORTCUT]
    } else {
        vec![TERMINAL_PASTE_SHORTCUT, "shift-insert"]
    }
}

pub(super) fn terminal_increase_font_defaults() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec!["cmd-+", "cmd-="]
    } else {
        vec!["ctrl-+", "ctrl-="]
    }
}

pub(super) fn terminal_platform_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}
