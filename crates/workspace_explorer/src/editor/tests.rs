use super::*;
use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowOptions};
use gpui_component::Root;

#[test]
fn file_and_diff_documents_use_distinct_identity_paths() {
    let regular = DocumentKey::File(PathBuf::from("/repo/src/lib.rs"));
    let diff = DocumentKey::Diff {
        repository: PathBuf::from("/repo"),
        path: PathBuf::from("src/lib.rs"),
    };

    assert_ne!(regular.identity_path(), diff.identity_path());
}

#[test]
fn file_size_is_compact() {
    assert_eq!("12 B", format_size(12));
    assert_eq!("2.0 KiB", format_size(2048));
    assert_eq!("2.0 MiB", format_size(2 * 1024 * 1024));
}

#[test]
fn diff_documents_are_read_only_and_use_diff_language() {
    let document = LoadedDocument::from_diff("@@ -1 +1 @@\n-old\n+new\n".to_string());

    assert!(document.read_only);
    assert_eq!("diff", document.language);
    assert!(matches!(document.policy, DocumentPolicy::Diff));
}

#[gpui::test]
fn local_file_opens_in_window_and_last_tab_can_close(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let path = test_file_path();
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|_| WorkspaceEditor::new(test_theme()));
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .expect("workspace editor window should open");
        (window, editor.expect("workspace editor should be created"))
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.open_file(path.clone(), window, cx);
        });
    });
    cx.run_until_parked();

    let (tab_count, text, read_only) = editor.read_with(&cx, |editor, cx| {
        let tab = editor.active_tab().expect("opened tab should be active");
        (
            editor.tabs.len(),
            tab.editor
                .as_ref()
                .expect("file input should load")
                .read(cx)
                .text()
                .to_string(),
            tab.read_only,
        )
    });
    assert_eq!(1, tab_count);
    assert_eq!("fn main() {}\n", text);
    assert!(!read_only);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.close_clean_tab(0, window, cx);
        });
    });
    assert!(!editor.read_with(&cx, |editor, _| editor.has_open_tabs()));
    let _ = std::fs::remove_file(path);
}

fn test_file_path() -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "workspace-editor-{}-{nonce}.rs",
        std::process::id()
    ))
}

fn test_theme() -> WorkspaceTheme {
    WorkspaceTheme {
        background: gpui::rgb(0x111111).into(),
        foreground: gpui::rgb(0xffffff).into(),
        muted: gpui::rgb(0x222222).into(),
        muted_foreground: gpui::rgb(0x999999).into(),
        border: gpui::rgb(0x333333).into(),
        accent: gpui::rgb(0x444444).into(),
        accent_foreground: gpui::rgb(0xffffff).into(),
        danger: gpui::rgb(0xff0000).into(),
        warning: gpui::rgb(0xffaa00).into(),
        success: gpui::rgb(0x00aa00).into(),
    }
}
