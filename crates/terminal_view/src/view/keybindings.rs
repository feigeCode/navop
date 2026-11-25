use super::*;

pub(super) const TERMINAL_CONTEXT: &str = "TerminalView";
pub(super) const TERMINAL_SEARCH_CONTEXT: &str = "TerminalSearch";

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
// 回滚历史滚动：沿用 Windows Terminal / Alacritty 的约定，翻页用 Shift+PageUp/PageDown，
// 逐行用 Ctrl+Shift+Up/Down。不占用 Shift+Up/Down，它们在历史提示中仍用于导航。
pub(super) const TERMINAL_SCROLL_LINE_UP_SHORTCUT: &str = "ctrl-shift-up";
pub(super) const TERMINAL_SCROLL_LINE_DOWN_SHORTCUT: &str = "ctrl-shift-down";
pub(super) const TERMINAL_SCROLL_PAGE_UP_SHORTCUT: &str = "shift-pageup";
pub(super) const TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT: &str = "shift-pagedown";
// Escape is terminal input (for example, Vim uses it to leave insert mode).
// ClearSelection remains available from the context menu and can still be
// assigned by users, but must not consume Escape by default.
pub(super) const TERMINAL_CLEAR_SELECTION_DEFAULTS: &[&str] = &[];
pub(super) fn terminal_shortcut_label(shortcut: &str) -> SharedString {
    Kbd::format(
        &Keystroke::parse_with_os(shortcut, one_core::keybindings::current_os())
            .expect("终端快捷键定义非法"),
    )
    .into()
}

/// 搜索入口展示当前生效的快捷键；用户禁用快捷键时不再展示无效提示。
pub(crate) fn terminal_search_shortcut_label(cx: &App) -> SharedString {
    shortcuts_for(
        cx,
        action_id::TERMINAL_SEARCH_FORWARD,
        &[TERMINAL_SEARCH_FORWARD_SHORTCUT],
    )
    .first()
    .map(|shortcut| terminal_shortcut_label(shortcut))
    .unwrap_or_default()
}

/// 对路径进行简单 shell 转义（用单引号包裹，处理内部单引号）
pub fn init(cx: &mut App) {
    crate::settings::init_settings(cx);
    crate::public_mcp::init(cx);
    init_broadcast_input_registry(cx);
    crate::quick_command_sync::init_quick_command_sync(cx);
    sftp_transfer::init(cx);
    workspace_explorer::init(cx);
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

pub(super) fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(crate::sidebar::file_manager_panel::init_keybindings());
    keybindings.push(terminal_search_close_keybinding());
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_SEND_TAB, &["tab"])
            .into_iter()
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, SendTab, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_SEND_SHIFT_TAB, &["shift-tab"])
            .into_iter()
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, SendShiftTab, Some(TERMINAL_CONTEXT), cx)),
    );
    // tmux 默认 prefix 是 Ctrl+B。终端内必须让这个 C0 字节透传到 PTY，
    // 不能被子栏切换等 app 级快捷键吞掉；否则用户无法使用 tmux 默认分屏/换窗键。
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_SEND_CTRL_B, &["ctrl-b"])
            .into_iter()
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, SendCtrlB, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_COPY, &[TERMINAL_COPY_SHORTCUT])
            .into_iter()
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, Copy, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::TERMINAL_PASTE, &terminal_paste_defaults())
            .into_iter()
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, Paste, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SELECT_ALL,
            &[TERMINAL_SELECT_ALL_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, SelectAll, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_CLEAR_SCREEN,
            &[TERMINAL_CLEAR_SCREEN_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ClearScreen, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_CLEAR_SELECTION,
            TERMINAL_CLEAR_SELECTION_DEFAULTS,
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ClearSelection, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SEARCH_FORWARD,
            &[TERMINAL_SEARCH_FORWARD_SHORTCUT],
        )
        .into_iter()
        .flat_map(|key| {
            [TERMINAL_CONTEXT, TERMINAL_SEARCH_CONTEXT]
                .into_iter()
                .map(|context| {
                    one_core::keybindings::load_dynamic_keybinding(
                        &key,
                        SearchForward,
                        Some(context),
                        cx,
                    )
                })
                .collect::<Vec<_>>()
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SEARCH_BACKWARD,
            &[TERMINAL_SEARCH_BACKWARD_SHORTCUT],
        )
        .into_iter()
        .flat_map(|key| {
            [TERMINAL_CONTEXT, TERMINAL_SEARCH_CONTEXT]
                .into_iter()
                .map(|context| {
                    one_core::keybindings::load_dynamic_keybinding(
                        &key,
                        SearchBackward,
                        Some(context),
                        cx,
                    )
                })
                .collect::<Vec<_>>()
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_TOGGLE_VI_MODE,
            &[TERMINAL_TOGGLE_VI_MODE_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ToggleViMode, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_INCREASE_FONT,
            &terminal_increase_font_defaults(),
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, IncreaseFont, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_DECREASE_FONT,
            &[terminal_platform_shortcut("cmd--", "ctrl--")],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, DecreaseFont, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_RESET_FONT,
            &[terminal_platform_shortcut("cmd-0", "ctrl-0")],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ResetFont, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SCROLL_LINE_UP,
            &[TERMINAL_SCROLL_LINE_UP_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ScrollLineUp, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SCROLL_LINE_DOWN,
            &[TERMINAL_SCROLL_LINE_DOWN_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ScrollLineDown, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SCROLL_PAGE_UP,
            &[TERMINAL_SCROLL_PAGE_UP_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ScrollPageUp, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TERMINAL_SCROLL_PAGE_DOWN,
            &[TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ScrollPageDown, Some(TERMINAL_CONTEXT), cx)),
    );
    keybindings
}

pub(super) fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(crate::sidebar::file_manager_panel::init_keybindings());
    keybindings.push(terminal_search_close_keybinding());
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
        action_id::TERMINAL_SEND_CTRL_B,
        &["ctrl-b"],
        Some(TERMINAL_CONTEXT),
        SendCtrlB,
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
        TERMINAL_CLEAR_SELECTION_DEFAULTS,
        Some(TERMINAL_CONTEXT),
        ClearSelection,
    ));
    for context in [TERMINAL_CONTEXT, TERMINAL_SEARCH_CONTEXT] {
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::TERMINAL_SEARCH_FORWARD,
            &[TERMINAL_SEARCH_FORWARD_SHORTCUT],
            Some(context),
            SearchForward,
        ));
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::TERMINAL_SEARCH_BACKWARD,
            &[TERMINAL_SEARCH_BACKWARD_SHORTCUT],
            Some(context),
            SearchBackward,
        ));
    }
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
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SCROLL_LINE_UP,
        &[TERMINAL_SCROLL_LINE_UP_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ScrollLineUp,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SCROLL_LINE_DOWN,
        &[TERMINAL_SCROLL_LINE_DOWN_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ScrollLineDown,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SCROLL_PAGE_UP,
        &[TERMINAL_SCROLL_PAGE_UP_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ScrollPageUp,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TERMINAL_SCROLL_PAGE_DOWN,
        &[TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT],
        Some(TERMINAL_CONTEXT),
        ScrollPageDown,
    ));
    keybindings
}

fn terminal_search_close_keybinding() -> KeyBinding {
    KeyBinding::new("escape", CloseTerminalSearch, Some(TERMINAL_SEARCH_CONTEXT))
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

pub(super) fn is_terminal_action_shortcut(keystroke: &Keystroke, cx: &App) -> bool {
    [
        (action_id::TERMINAL_SEND_TAB, vec!["tab"]),
        (action_id::TERMINAL_SEND_SHIFT_TAB, vec!["shift-tab"]),
        (action_id::TERMINAL_SEND_CTRL_B, vec!["ctrl-b"]),
        (action_id::TERMINAL_COPY, vec![TERMINAL_COPY_SHORTCUT]),
        (action_id::TERMINAL_PASTE, terminal_paste_defaults()),
        (
            action_id::TERMINAL_SELECT_ALL,
            vec![TERMINAL_SELECT_ALL_SHORTCUT],
        ),
        (
            action_id::TERMINAL_CLEAR_SCREEN,
            vec![TERMINAL_CLEAR_SCREEN_SHORTCUT],
        ),
        (
            action_id::TERMINAL_CLEAR_SELECTION,
            TERMINAL_CLEAR_SELECTION_DEFAULTS.to_vec(),
        ),
        (
            action_id::TERMINAL_SEARCH_FORWARD,
            vec![TERMINAL_SEARCH_FORWARD_SHORTCUT],
        ),
        (
            action_id::TERMINAL_SEARCH_BACKWARD,
            vec![TERMINAL_SEARCH_BACKWARD_SHORTCUT],
        ),
        (
            action_id::TERMINAL_TOGGLE_VI_MODE,
            vec![TERMINAL_TOGGLE_VI_MODE_SHORTCUT],
        ),
        (
            action_id::TERMINAL_INCREASE_FONT,
            terminal_increase_font_defaults(),
        ),
        (
            action_id::TERMINAL_DECREASE_FONT,
            vec![terminal_platform_shortcut("cmd--", "ctrl--")],
        ),
        (
            action_id::TERMINAL_RESET_FONT,
            vec![terminal_platform_shortcut("cmd-0", "ctrl-0")],
        ),
        (
            action_id::TERMINAL_SCROLL_LINE_UP,
            vec![TERMINAL_SCROLL_LINE_UP_SHORTCUT],
        ),
        (
            action_id::TERMINAL_SCROLL_LINE_DOWN,
            vec![TERMINAL_SCROLL_LINE_DOWN_SHORTCUT],
        ),
        (
            action_id::TERMINAL_SCROLL_PAGE_UP,
            vec![TERMINAL_SCROLL_PAGE_UP_SHORTCUT],
        ),
        (
            action_id::TERMINAL_SCROLL_PAGE_DOWN,
            vec![TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT],
        ),
    ]
    .into_iter()
    .any(|(action_id, defaults)| {
        keystroke_matches_shortcuts(keystroke, &shortcuts_for(cx, action_id, &defaults))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        TERMINAL_CLEAR_SELECTION_DEFAULTS, TERMINAL_SCROLL_LINE_DOWN_SHORTCUT,
        TERMINAL_SCROLL_LINE_UP_SHORTCUT, TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT,
        TERMINAL_SCROLL_PAGE_UP_SHORTCUT,
    };
    use gpui::Keystroke;

    #[test]
    fn clear_selection_does_not_consume_escape_by_default() {
        assert!(TERMINAL_CLEAR_SELECTION_DEFAULTS.is_empty());
    }

    #[test]
    fn scroll_shortcuts_are_valid_keystrokes() {
        for shortcut in [
            TERMINAL_SCROLL_LINE_UP_SHORTCUT,
            TERMINAL_SCROLL_LINE_DOWN_SHORTCUT,
            TERMINAL_SCROLL_PAGE_UP_SHORTCUT,
            TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT,
        ] {
            assert!(Keystroke::parse(shortcut).is_ok(), "{shortcut} 必须可解析");
        }
    }

    #[test]
    fn scroll_shortcuts_do_not_shadow_plain_terminal_keys() {
        // 无修饰的 PageUp/PageDown 与 Shift+Up/Down 属于 shell/TUI 输入，不能被占用。
        for shortcut in [
            TERMINAL_SCROLL_LINE_UP_SHORTCUT,
            TERMINAL_SCROLL_LINE_DOWN_SHORTCUT,
            TERMINAL_SCROLL_PAGE_UP_SHORTCUT,
            TERMINAL_SCROLL_PAGE_DOWN_SHORTCUT,
        ] {
            let modifiers = Keystroke::parse(shortcut).unwrap().modifiers;
            assert!(modifiers.shift, "{shortcut} 必须带 Shift");
        }
        assert_ne!("shift-up", TERMINAL_SCROLL_LINE_UP_SHORTCUT);
        assert_ne!("shift-down", TERMINAL_SCROLL_LINE_DOWN_SHORTCUT);
    }
}
