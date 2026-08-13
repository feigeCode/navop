#[test]
fn remote_editor_registers_its_dirty_close_guard_with_the_shared_router() {
    let source = include_str!("editor_window.rs");
    let open_start = source
        .find("pub fn open_remote_file_editor")
        .expect("remote editor opener");
    let open_end = source[open_start..]
        .find("\nfn open_in_existing_window")
        .map(|offset| open_start + offset)
        .expect("remote editor opener end");
    let open = &source[open_start..open_end];

    assert!(open.contains("register_window_close_handler"));
    assert!(source.contains("set_window_close_handler"));
    assert!(source.contains("this.request_close_window(window, cx)"));
    assert!(!source.contains("request_close_window_if_editor"));
}
