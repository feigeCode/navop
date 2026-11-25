//! Windows / 非 macOS 下 Ctrl+C 中断信号保护与智能复制退化测试。

use gpui::{KeyBinding, KeyContext, Keymap, Keystroke, NoAction};
use gpui_component::input::Copy as RootCopy;

use crate::view::actions::Copy as TerminalCopy;
use crate::view::keybindings::TERMINAL_CONTEXT;
use crate::view::text_input::{is_plain_ctrl_c, should_copy_instead_of_interrupt};

fn keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse_with_os(shortcut, one_core::keybindings::current_os()).expect("解析快捷键失败")
}

#[test]
fn plain_ctrl_c_detection_matches_only_unmodified_ctrl_c() {
    assert!(is_plain_ctrl_c(&keystroke("ctrl-c")));
    assert!(!is_plain_ctrl_c(&keystroke("ctrl-shift-c")));
    assert!(!is_plain_ctrl_c(&keystroke("ctrl-alt-c")));
    assert!(!is_plain_ctrl_c(&keystroke("c")));
    assert!(!is_plain_ctrl_c(&keystroke("ctrl-d")));
}

#[test]
fn smart_ctrl_c_copies_only_when_selection_exists_otherwise_falls_through_to_interrupt() {
    let ctrl_c = keystroke("ctrl-c");
    let ctrl_shift_c = keystroke("ctrl-shift-c");

    // 纯 Ctrl+C：有选区时复制并清选区，无选区时放行发送 \x03。
    assert!(should_copy_instead_of_interrupt(&ctrl_c, true));
    assert!(!should_copy_instead_of_interrupt(&ctrl_c, false));

    // Ctrl+Shift+C 等非纯 Ctrl+C 组合键：始终走纯复制，不发送中断。
    assert!(should_copy_instead_of_interrupt(&ctrl_shift_c, true));
    assert!(should_copy_instead_of_interrupt(&ctrl_shift_c, false));
}

#[test]
fn terminal_context_ctrl_c_no_action_shadows_root_copy_while_allowing_custom_terminal_copy() {
    let context_stack = [
        KeyContext::parse("Root").unwrap(),
        KeyContext::parse(TERMINAL_CONTEXT).unwrap(),
    ];
    let ctrl_c = [keystroke("ctrl-c")];

    // 1. 默认配置下（terminal.copy = ctrl-shift-c）：TerminalView 上下文的 NoAction
    //    必须屏蔽外层 Root 的 input::Copy，避免存在外部 TextSelection 时吃掉 Ctrl+C。
    let mut default_keymap = Keymap::default();
    default_keymap.add_bindings([
        KeyBinding::new("ctrl-c", RootCopy, Some("Root")),
        KeyBinding::new("ctrl-c", NoAction, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some(TERMINAL_CONTEXT)),
    ]);
    let (default_bindings, _) = default_keymap.bindings_for_input(&ctrl_c, &context_stack);
    assert!(
        default_bindings.is_empty(),
        "默认配置下 TerminalView 内的 ctrl-c 不应匹配到 Root::Copy，实际匹配：{default_bindings:?}"
    );

    // 2. 用户显式把 terminal.copy 自定义为 ctrl-c 时：由于 TERMINAL_COPY 排在 NoAction 之后，
    //    仍应匹配到 TerminalView 自己的 Copy action（由其决定有选区复制 / 无选区透传 \x03）。
    let mut custom_keymap = Keymap::default();
    custom_keymap.add_bindings([
        KeyBinding::new("ctrl-c", RootCopy, Some("Root")),
        KeyBinding::new("ctrl-c", NoAction, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("ctrl-c", TerminalCopy, Some(TERMINAL_CONTEXT)),
    ]);
    let (custom_bindings, _) = custom_keymap.bindings_for_input(&ctrl_c, &context_stack);
    assert_eq!(1, custom_bindings.len());
    assert!(custom_bindings[0].action().partial_eq(&TerminalCopy));
}

#[test]
fn ctrl_c_root_guard_is_registered_before_terminal_copy_in_both_init_and_refresh() {
    let source = include_str!("../keybindings.rs");
    for fn_name in ["fn init_keybindings(", "fn refreshable_keybindings("] {
        let region = source
            .split(fn_name)
            .nth(1)
            .expect("keybindings function should exist");
        let guard = region
            .find("terminal_ctrl_c_root_guard_keybinding(cx)")
            .unwrap_or_else(|| panic!("{fn_name} 必须注册 ctrl-c Root 屏蔽绑定"));
        let copy = region
            .find("action_id::TERMINAL_COPY")
            .unwrap_or_else(|| panic!("{fn_name} 必须保留 TERMINAL_COPY 绑定"));
        assert!(
            guard < copy,
            "{fn_name} 中 ctrl-c NoAction 必须排在 TERMINAL_COPY 之前，以确保用户自定义 ctrl-c 复制仍可覆盖"
        );
    }
}

#[test]
fn ctrl_c_empty_copy_action_propagates_to_handle_key_event_and_sends_etx() {
    let clipboard = include_str!("../clipboard.rs");
    let copy_fn = clipboard
        .split("pub(super) fn copy(")
        .nth(1)
        .expect("copy should exist")
        .split("pub(super) fn has_copyable_selection")
        .next()
        .unwrap();
    assert!(
        copy_fn.contains("should_propagate_empty_copy_as_ctrl_c(cx)")
            && copy_fn.contains("cx.propagate()"),
        "无选区且 terminal.copy 含 ctrl-c 时，copy action 必须 propagate 让事件到达 handle_key_event"
    );

    let text_input = include_str!("../text_input.rs");
    let key_event = text_input
        .split("pub(super) fn handle_key_event")
        .nth(1)
        .expect("handle_key_event should exist");
    assert!(
        key_event.contains("should_copy_instead_of_interrupt(&event.keystroke, self.has_copyable_selection(cx))"),
        "handle_key_event 必须按选区决定 ctrl-c 是复制还是透传"
    );
    assert!(
        key_event.contains("if !allow_ctrl_c_interrupt_passthrough && is_terminal_action_shortcut"),
        "无选区的 ctrl-c 透传不能被 is_terminal_action_shortcut 拦截"
    );

    // 验证终端按键编码层确实将 ctrl-c 编成 \x03。
    let encoded = crate::keys::to_esc_str(&keystroke("ctrl-c"), &alacritty_terminal::term::TermMode::NONE, false);
    assert_eq!(Some("\x03"), encoded.as_deref());
}
