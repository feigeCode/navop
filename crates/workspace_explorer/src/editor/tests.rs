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
    let document =
        LoadedDocument::from_diff("@@ -1 +1 @@\n-old\n+new\n".to_string(), "text".to_string());

    assert!(document.read_only);
    assert_eq!("diff", document.language);
    assert!(matches!(document.policy, DocumentPolicy::Diff));
}

#[test]
fn notes_markdown_theme_uses_workspace_colors() {
    let workspace = test_theme();
    let markdown = super::markdown::markdown_editor_theme(workspace);

    assert_eq!(workspace.background, markdown.background);
    assert_eq!(workspace.foreground, markdown.foreground);
    assert_eq!(workspace.accent, markdown.primary);
    assert_eq!(workspace.accent_foreground, markdown.primary_foreground);
    assert!(markdown.highlight_theme.appearance.is_dark());
}

#[gpui::test]
fn markdown_file_uses_notes_markdown_editor(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        notes::init(cx);
    });
    let path = test_file_path_with_extension("md");
    let source = concat!(
        "# Workspace Markdown\n\n",
        "> <https://example.com/path_(item)>\n\n",
        "Use `snake_case(value)` and [README](README_CN.md).\n\n",
        "1. First\n2. Second\n",
    );
    std::fs::write(&path, source).unwrap();
    let (window, editor) = open_test_editor(cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.open_file(path.clone(), window, cx);
        });
    });
    cx.run_until_parked();

    editor.read_with(&cx, |editor, _| {
        let tab = editor.active_tab().expect("opened tab should be active");
        assert!(matches!(tab.policy, DocumentPolicy::Markdown));
        assert!(tab.markdown.is_some());
        assert!(tab.editor.is_none());
    });
    assert_eq!(source, std::fs::read_to_string(&path).unwrap());

    let _ = std::fs::remove_file(path);
}

#[gpui::test]
fn local_file_opens_in_window_and_last_tab_can_close(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let path = test_file_path();
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (window, editor) = open_test_editor(cx);
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
    test_file_path_with_extension("rs")
}

fn test_file_path_with_extension(extension: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "workspace-editor-{}-{nonce}.{extension}",
        std::process::id(),
    ))
}

fn open_test_editor(
    cx: &mut TestAppContext,
) -> (gpui::AnyWindowHandle, gpui::Entity<WorkspaceEditor>) {
    cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|_| WorkspaceEditor::new(test_theme()));
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .expect("workspace editor window should open");
        (
            window.into(),
            editor.expect("workspace editor should be created"),
        )
    })
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
