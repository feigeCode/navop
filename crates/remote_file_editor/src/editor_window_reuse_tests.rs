//! State-level tests deliberately avoid Tokio::spawn in GPUI's deterministic
//! scheduler. Native AppKit hide/reopen still needs a real Touch Bar machine.
use super::*;
use anyhow::Result;
use sftp::{DirectoryConflictPolicy, FileEntry, PathMetadata, ProgressCallback};
use std::sync::atomic::AtomicBool;

struct NoIoClient;

// These fixtures exercise delivery/close state, never a network operation.
macro_rules! no_io_client {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)*) => {
        #[async_trait::async_trait]
        impl RemoteFileClient for NoIoClient {
            $(async fn $name(&mut self, $($arg: $ty),*) -> Result<$out> {
                panic!("unexpected I/O in editor lifecycle test: {}", stringify!($name))
            })*
        }
    };
}

no_io_client! {
    list_dir(_path: &str) -> Vec<FileEntry>;
    stat(_path: &str) -> Option<PathMetadata>;
    download_with_progress(_remote: &str, _local: &str, _cancel: Arc<AtomicBool>, _progress: ProgressCallback) -> ();
    upload_with_progress(_local: &str, _remote: &str, _cancel: Arc<AtomicBool>, _progress: ProgressCallback) -> ();
    delete(_path: &str, _is_dir: bool) -> ();
    delete_recursive(_path: &str, _cancel: Arc<AtomicBool>, _progress: ProgressCallback) -> ();
    mkdir(_path: &str) -> ();
    rename(_old: &str, _new: &str) -> ();
    chmod(_path: &str, _mode: u32) -> ();
    read_file(_path: &str, _max: usize) -> Vec<u8>;
    write_file(_path: &str, _content: &[u8]) -> ();
    list_dir_recursive(_path: &str, _cancel: Arc<AtomicBool>) -> Vec<FileEntry>;
    download_dir_with_progress(_remote: &str, _local: &str, _cancel: Arc<AtomicBool>, _progress: ProgressCallback) -> ();
    upload_dir_with_progress(_local: &str, _remote: &str, _policy: DirectoryConflictPolicy, _cancel: Arc<AtomicBool>, _progress: ProgressCallback) -> ();
    disconnect() -> ();
    realpath(_path: &str) -> String;
}

fn client() -> SharedRemoteFileClient {
    Arc::new(tokio::sync::Mutex::new(Box::new(NoIoClient)))
}

fn empty_editor() -> RemoteFileEditorWindow {
    diagnostics::record_editor_view_created();
    RemoteFileEditorWindow {
        tabs: Vec::new(),
        active_tab: 0,
        close_prompt_open: false,
        pending_close_action: None,
        close_window_after_saves: false,
        next_tab_id: 1,
        prompt_generation: 0,
    }
}

fn add_tab(view: &mut RemoteFileEditorWindow, client: SharedRemoteFileClient) -> u64 {
    let id = view.next_tab_id;
    view.next_tab_id += 1;
    view.tabs.push(RemoteEditorTab::new(
        id,
        "/same/path.txt".into(),
        client,
        RemoteMutationCallback::new(|_| panic!("stale save notified the remote browser")),
    ));
    diagnostics::record_editor_tab_created();
    id
}

#[test]
fn hiding_releases_tab_connections_and_invalidates_close_state_without_reusing_ids() {
    let _guard = diagnostics::GAUGE_LOCK.lock().unwrap();
    let baseline = diagnostics::snapshot();
    let mut view = empty_editor();
    let old_client = client();
    let weak_client = Arc::downgrade(&old_client);
    let old_id = add_tab(&mut view, old_client);
    view.active_tab = 10;
    view.close_prompt_open = true;
    view.pending_close_action = Some(PendingCloseAction::Tab(old_id));
    view.close_window_after_saves = true;
    let old_prompt = view.prompt_generation;

    view.reset_for_reuse();
    assert!(view.tabs.is_empty());
    assert!(weak_client.upgrade().is_none());
    assert_eq!(view.active_tab, 0);
    assert!(!view.close_prompt_open);
    assert!(view.pending_close_action.is_none());
    assert!(!view.close_window_after_saves);
    assert_ne!(view.prompt_generation, old_prompt);
    assert!(add_tab(&mut view, client()) > old_id);

    // Repeated closes must not underflow the tab gauges or resurrect state.
    view.reset_for_reuse();
    view.reset_for_reuse();
    assert!(view.tabs.is_empty());
    assert!(view.next_tab_id > old_id);
    assert_eq!(
        diagnostics::snapshot().live_editor_tabs,
        baseline.live_editor_tabs
    );
    drop(view);
    assert_eq!(diagnostics::snapshot(), baseline);
}

#[gpui::test]
fn old_io_completions_cannot_touch_reopened_same_path(cx: &mut gpui::TestAppContext) {
    let _guard = diagnostics::GAUGE_LOCK.lock().unwrap();
    let cx = cx.add_empty_window();
    let view = cx.new(|_| empty_editor());
    view.update_in(cx, |view, window, cx| {
        let old_id = add_tab(view, client());
        view.reset_for_reuse();
        let new_id = add_tab(view, client());
        view.tabs[0].saved_text = "new connection".into();
        view.close_window_after_saves = true;

        assert!(!view.apply_load_error(old_id, "/same/path.txt", "old read error".into(), cx));
        assert!(!view.apply_save_error(old_id, "/same/path.txt", "old write error".into(), cx));
        view.apply_loaded_file(
            old_id,
            "/same/path.txt",
            LoadedFile {
                text: "old server".into(),
                policy: FilePolicy {
                    mode: EditorMode::Code,
                    is_large_file: false,
                },
                file_size: 10,
                language: "plain".into(),
            },
            window,
            cx,
        );
        // close_after_save=true must not close the newly opened window/tab.
        view.apply_saved_file(
            old_id,
            "/same/path.txt",
            "old save".into(),
            true,
            window,
            cx,
        );
        assert_eq!(view.tabs.len(), 1);
        assert_eq!(view.tabs[0].id, new_id);
        assert_eq!(view.tabs[0].saved_text, "new connection");
        assert!(view.tabs[0].editor.is_none());
        assert!(view.tabs[0].loading);
        assert!(view.close_window_after_saves);
        assert!(view.apply_load_error(new_id, "/same/path.txt", "current error".into(), cx));
        assert!(!view.tabs[0].loading);
    });
    drop(view);
    cx.run_until_parked();
}

#[gpui::test]
fn close_prompt_targets_tab_identity_not_shifted_index(cx: &mut gpui::TestAppContext) {
    let _guard = diagnostics::GAUGE_LOCK.lock().unwrap();
    let cx = cx.add_empty_window();
    let view = cx.new(|_| empty_editor());
    view.update_in(cx, |view, window, cx| {
        let first = add_tab(view, client());
        let target = add_tab(view, client());
        let last = add_tab(view, client());
        view.close_clean_tab(0, window, cx);
        assert!(view.tabs.iter().all(|tab| tab.id != first));
        view.discard_close_action(PendingCloseAction::Tab(target), window, cx);
        assert_eq!(view.tabs.len(), 1);
        assert_eq!(view.tabs[0].id, last);
        // A response for an already closed tab must be a no-op.
        view.discard_close_action(PendingCloseAction::Tab(target), window, cx);
        assert_eq!(view.tabs.len(), 1);
    });
    drop(view);
    cx.run_until_parked();
}

#[gpui::test]
fn same_path_on_different_connections_focuses_the_matching_tab(cx: &mut gpui::TestAppContext) {
    let _guard = diagnostics::GAUGE_LOCK.lock().unwrap();
    let cx = cx.add_empty_window();
    let view = cx.new(|_| empty_editor());
    view.update_in(cx, |view, window, cx| {
        let first_client = client();
        let second_client = client();
        add_tab(view, first_client.clone());
        add_tab(view, second_client.clone());
        view.open_or_focus_tab(
            "/same/path.txt".into(),
            second_client,
            RemoteMutationCallback::new(|_| {}),
            window,
            cx,
        );
        assert_eq!(view.active_tab, 1);
        assert_eq!(view.tabs.len(), 2);
        view.open_or_focus_tab(
            "/same/path.txt".into(),
            first_client,
            RemoteMutationCallback::new(|_| {}),
            window,
            cx,
        );
        assert_eq!(view.active_tab, 0);
        assert_eq!(view.tabs.len(), 2);
    });
    drop(view);
    cx.run_until_parked();
}

#[gpui::test]
fn prompt_cancel_preserves_tabs_and_old_prompt_cannot_close_reused_session(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = diagnostics::GAUGE_LOCK.lock().unwrap();
    let cx = cx.add_empty_window();
    let view = cx.new(|_| empty_editor());
    view.update_in(cx, |view, window, cx| {
        add_tab(view, client());
        view.show_unsaved_changes_prompt(PendingCloseAction::Window, window, cx);
        // Closing again while a sheet is up must not hide/destroy the window.
        assert!(!view.handle_window_should_close(window, cx));
    });
    cx.simulate_prompt_answer(&t!("RemoteFileEditor.action.cancel"));
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        assert_eq!(view.tabs.len(), 1);
        assert!(!view.close_prompt_open);
        assert!(view.pending_close_action.is_none());
        view.show_unsaved_changes_prompt(PendingCloseAction::Window, window, cx);
        view.reset_for_reuse();
        add_tab(view, client());
        // A later session has its own pending action. The previous answer
        // must not take it or clear its prompt flag.
        view.pending_close_action = Some(PendingCloseAction::Window);
        view.close_prompt_open = true;
    });
    cx.simulate_prompt_answer(&t!("RemoteFileEditor.action.discard"));
    cx.run_until_parked();
    view.update(cx, |view, _| {
        assert_eq!(view.tabs.len(), 1);
        assert!(view.close_prompt_open);
        assert_eq!(view.pending_close_action, Some(PendingCloseAction::Window));
    });
    drop(view);
    cx.run_until_parked();
}
