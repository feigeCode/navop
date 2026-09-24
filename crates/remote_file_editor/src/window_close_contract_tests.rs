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

/// Returns the source of the method introduced by `signature`, up to the next
/// method in the same `impl` block.
fn method_source<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature).expect(signature);
    let body_start = start + signature.len();
    let end = source[body_start..]
        .find("\n    fn ")
        .map(|offset| body_start + offset)
        .unwrap_or(source.len());
    &source[start..end]
}

/// Returns a bounded region starting at `marker`, counted in characters so the
/// slice can never split a multi-byte UTF-8 sequence.
fn region<'a>(source: &'a str, marker: &str, max_chars: usize) -> &'a str {
    let start = source.find(marker).expect(marker);
    let rest = &source[start..];
    match rest.char_indices().nth(max_chars) {
        Some((offset, _)) => &rest[..offset],
        None => rest,
    }
}

/// A slow SFTP round-trip must not keep the closed editor window alive. That
/// is only true while the detached task holds a weak reference, so this test
/// pins the reference *kind* down instead of the surrounding plumbing.
#[test]
fn detached_reload_and_save_tasks_hold_a_weak_view_reference() {
    let source = include_str!("editor_window.rs");

    for signature in ["fn reload_tab(", "fn save_tab("] {
        let body = method_source(source, signature);
        let name = signature.trim_end_matches('(');

        assert!(
            body.contains("cx.entity().downgrade()"),
            "{name} must capture a weak view reference so a slow remote round-trip cannot pin a closed editor window"
        );
        assert!(
            !body.contains("cx.entity().clone()"),
            "{name} must not capture a strong view reference in its detached task"
        );
        assert!(
            body.contains("EditorTaskGuard::begin"),
            "{name} must account for its in-flight work in the lifecycle counters"
        );
        assert!(
            body.contains("log_task_result_discarded"),
            "{name}'s cancellation strategy must be observable, not a silently swallowed result"
        );
        assert!(
            !body.contains("let _ = view.update_in"),
            "{name} must not swallow a delivery failure on a released view"
        );
    }
}

/// The counters have to return to their baseline on the real release paths,
/// otherwise a memory-growth report would show an ever-rising editor count.
#[test]
fn editor_lifecycle_counters_are_released_on_every_close_path() {
    let source = include_str!("editor_window.rs");

    let drop_impl = region(source, "impl Drop for RemoteFileEditorWindow", 320);
    assert!(drop_impl.contains("record_editor_view_released_with_tabs(self.tabs.len())"));

    let close_tab = method_source(source, "fn close_clean_tab(");
    assert!(close_tab.contains("self.tabs.remove(index)"));
    assert!(close_tab.contains("record_editor_tab_dropped()"));

    let open = method_source(source, "fn open_or_focus_tab(");
    assert!(open.contains("record_editor_tab_created()"));
    assert!(source.contains("record_editor_view_created()"));
}

#[test]
fn all_editor_close_paths_use_the_reusable_window_boundary() {
    let source = include_str!("editor_window.rs");
    let native_close = method_source(source, "fn handle_window_should_close(");
    assert!(native_close.contains("self.prepare_window_close(window, cx)"));
    for signature in [
        "fn apply_saved_file(",
        "fn close_clean_tab(",
        "fn discard_close_action(",
        "fn save_dirty_tabs_and_close_window(",
    ] {
        let body = method_source(source, signature);
        assert!(
            body.contains("self.finish_window_close(window, cx)"),
            "{signature}"
        );
        assert!(!body.contains("window.remove_window()"), "{signature}");
    }
}

#[test]
fn reused_editor_tabs_keep_their_own_remote_connection() {
    let source = include_str!("editor_window.rs");
    let open = method_source(source, "fn open_or_focus_tab(");
    assert!(open.contains("Arc::ptr_eq(&tab.client, &client)"));
    for signature in ["fn reload_tab(", "fn save_tab("] {
        let body = method_source(source, signature);
        assert!(body.contains("tab.client.clone()"), "{signature}");
        assert!(!body.contains("self.client"), "{signature}");
        // A closed tab's error must not appear on a later use of this window.
        assert!(body.contains("if this.apply_"), "{signature}");
    }
}

#[test]
fn native_hide_is_macos_only_and_keeps_the_window_registered() {
    let platform = include_str!("editor_window_visibility.rs");
    assert!(platform.contains("#[cfg(target_os = \"macos\")]"));
    assert!(platform.contains("MainThreadMarker::new()"));
    assert!(platform.contains("native.orderOut(None)"));
    assert!(platform.contains("!native.isVisible()"));
    let other_platforms = platform
        .split("#[cfg(not(target_os = \"macos\"))]")
        .nth(1)
        .expect("non-macOS close behavior");
    assert!(other_platforms.contains("Ok(false)"));

    let source = include_str!("editor_window.rs");
    let prepare = method_source(source, "fn prepare_window_close(");
    let hidden = prepare
        .split("Ok(true) =>")
        .nth(1)
        .unwrap()
        .split("Ok(false)")
        .next()
        .unwrap();
    assert!(hidden.contains("self.reset_for_reuse()"));
    assert!(hidden.contains("false"));
    assert!(!hidden.contains("clear_editor_window"));
    assert!(!hidden.contains("remove_window"));
    let reset = method_source(source, "fn reset_for_reuse(");
    assert!(!reset.contains("self.next_tab_id ="));
    assert!(reset.contains("self.tabs.clear()"));
}
