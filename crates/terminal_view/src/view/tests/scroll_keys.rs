use super::super::scroll_keys::terminal_page_scroll_lines;

#[test]
fn page_scroll_keeps_one_line_of_context() {
    assert_eq!(23, terminal_page_scroll_lines(24));
    assert_eq!(1, terminal_page_scroll_lines(2));
}

#[test]
fn page_scroll_never_degenerates_to_zero() {
    assert_eq!(1, terminal_page_scroll_lines(1));
    assert_eq!(1, terminal_page_scroll_lines(0));
}

#[test]
fn scroll_actions_are_registered_and_bound_in_terminal_context() {
    let surface = include_str!("../render_surface.rs");
    let bindings = include_str!("../keybindings.rs");

    for (handler, action) in [
        ("Self::scroll_line_up", "ScrollLineUp"),
        ("Self::scroll_line_down", "ScrollLineDown"),
        ("Self::scroll_page_up", "ScrollPageUp"),
        ("Self::scroll_page_down", "ScrollPageDown"),
    ] {
        assert!(surface.contains(handler), "{handler} 必须注册为 action");
        assert!(
            bindings.matches(&format!("{action},")).count() >= 2,
            "{action} 必须同时出现在初始绑定与可刷新绑定中"
        );
    }
}

#[test]
fn ctrl_b_is_routed_to_terminal_as_tmux_prefix() {
    let surface = include_str!("../render_surface.rs");
    let bindings = include_str!("../keybindings.rs");
    let tool_dock = include_str!("../tool_dock.rs");

    assert!(surface.contains(".on_action(cx.listener(Self::send_ctrl_b))"));
    assert_eq!(2, bindings.matches("SendCtrlB,").count());
    assert!(bindings.contains("action_id::TERMINAL_SEND_CTRL_B"));
    assert!(tool_dock.contains("self.write_to_pty(b\"\\x02\".to_vec(), cx);"));
}

#[test]
fn scroll_keys_yield_to_alt_screen_programs() {
    let source = include_str!("../scroll_keys.rs");
    let body = source
        .split("fn scroll_history_by")
        .nth(1)
        .expect("scroll_history_by should exist");
    let alt_screen = body
        .find("mode.contains(TermMode::ALT_SCREEN)")
        .expect("alt-screen 必须被识别");
    let propagate = body
        .find("cx.propagate()")
        .expect("alt-screen 必须放行按键");
    let scroll = body
        .find("try_scroll_display_delta")
        .expect("普通屏幕应滚动历史");

    assert!(alt_screen < propagate && propagate < scroll);
}
