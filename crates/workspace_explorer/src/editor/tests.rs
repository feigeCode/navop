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
fn review_diff_identity_never_collides_with_the_file_tab() {
    let path = PathBuf::from("/repo/src/lib.rs");
    let file = DocumentKey::File(path.clone());
    let review = DocumentKey::ReviewFile(path.clone());
    let other = DocumentKey::ReviewFile(PathBuf::from("/repo/src/main.rs"));

    assert_ne!(
        file.identity_path(),
        review.identity_path(),
        "审阅页不能顶掉同名文件页"
    );
    assert_ne!(
        review.identity_path(),
        other.identity_path(),
        "不同文件各占一页"
    );
}

#[test]
fn file_size_is_compact() {
    assert_eq!("12 B", format_size(12));
    assert_eq!("2.0 KiB", format_size(2048));
    assert_eq!("2.0 MiB", format_size(2 * 1024 * 1024));
}

#[test]
fn diff_documents_are_read_only() {
    let document = LoadedDocument::from_diff("@@ -1 +1 @@\n-old\n+new\n".to_string());

    assert!(document.read_only);
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
        selection: gpui::rgb(0x55a0fc).into(),
        caret: gpui::rgb(0xffffff).into(),
        danger: gpui::rgb(0xff0000).into(),
        warning: gpui::rgb(0xffaa00).into(),
        success: gpui::rgb(0x00aa00).into(),
    }
}

#[test]
fn snapshot_diff_documents_are_read_only() {
    let document = LoadedDocument::from_snapshot_diff("diff --git a/x b/x\n".to_string());

    assert!(document.read_only);
    assert!(matches!(document.policy, DocumentPolicy::Diff));
}

#[gpui::test]
fn snapshot_diff_reuses_one_tab_across_turns(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        notes::init(cx);
    });
    let (window, editor) = open_test_editor(cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    for turn in 0..3 {
        editor.update_in(&mut cx, |editor, window, cx| {
            editor.open_snapshot_diff(format!("turn {turn}"), snapshot_patch(turn), window, cx);
        });
        cx.run_until_parked();
    }

    let (tab_count, text, name) = editor.read_with(&cx, |editor, _| {
        let tab = editor.active_tab().expect("review tab should be active");
        (
            editor.tabs.len(),
            tab.saved_text.clone(),
            tab.display_name.clone(),
        )
    });
    assert_eq!(
        1, tab_count,
        "last-turn review 必须复用同一标签页，而不是每轮开新页"
    );
    assert_eq!(
        snapshot_patch(2),
        text,
        "复用同一页不等于停在第一页：每轮都得换成这一轮的 patch"
    );
    assert_eq!("turn 2", name, "标签标题也要跟着这一轮走");
}

/// 每轮一份可区分的 patch：内容不同才测得出「刷新了没有」。
fn snapshot_patch(turn: usize) -> String {
    format!(
        "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,1 @@\n-old\n+turn {turn}\n"
    )
}

#[gpui::test]
fn diff_tabs_start_in_a_single_column(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        notes::init(cx);
    });
    let (window, editor) = open_test_editor(cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    editor.update_in(&mut cx, |editor, window, cx| {
        editor.open_snapshot_diff("turn 0".to_string(), snapshot_patch(0), window, cx);
    });
    cx.run_until_parked();

    let side_by_side = editor.read_with(&cx, |editor, _| {
        editor
            .active_tab()
            .expect("review tab should be active")
            .diff_side_by_side
    });
    assert!(
        !side_by_side,
        "审阅面板默认单栏：它是可调宽的侧栏，窄下来时 Split 的右栏会被父级 \
         overflow_hidden 整块裁掉，看起来就像「diff 少了一半」"
    );
}

#[gpui::test]
fn reopening_a_file_keeps_unsaved_edits(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        notes::init(cx);
    });
    let path = test_file_path();
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (window, editor) = open_test_editor(cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.open_file(path.clone(), window, cx));
    });
    cx.run_until_parked();
    let state = editor.read_with(&cx, |editor, _| {
        editor
            .active_tab()
            .expect("file tab should be active")
            .editor
            .clone()
            .expect("file tab owns an editor")
    });
    cx.update(|window, cx| {
        state.update(cx, |state, cx| {
            state.set_value("fn main() { /* edited */ }\n".to_string(), window, cx);
        });
    });

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.open_file(path.clone(), window, cx));
    });
    cx.run_until_parked();

    let (tab_count, text, dirty) = editor.read_with(&cx, |editor, cx| {
        let tab = editor.active_tab().expect("file tab should be active");
        (
            editor.tabs.len(),
            tab.editor.as_ref().unwrap().read(cx).text().to_string(),
            tab.is_dirty(cx),
        )
    });
    assert_eq!(1, tab_count, "同一个文件只占一页");
    assert_eq!(
        "fn main() { /* edited */ }\n", text,
        "重开文件只能聚焦，不能重载——重载会吞掉未保存的改动"
    );
    assert!(dirty);
    let _ = std::fs::remove_file(path);
}
