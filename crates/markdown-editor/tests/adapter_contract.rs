use std::sync::{Arc, Mutex};

use gpui::{AppContext, Hsla, TestAppContext};
use gpui_component::highlighter::HighlightTheme;
use markdown_editor::{
    MarkdownEditor, MarkdownEditorEvent, MarkdownEditorTheme, ViewMode,
    markdown_editor_host_services,
};

fn init_editor_test_app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
}

fn test_theme() -> MarkdownEditorTheme {
    MarkdownEditorTheme {
        background: Hsla::default(),
        foreground: Hsla::default(),
        muted_foreground: Hsla::default(),
        border: Hsla::default(),
        primary: Hsla::default(),
        highlight_theme: HighlightTheme::default_dark(),
    }
}

#[gpui::test]
async fn adapter_delegates_document_state_and_events(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| MarkdownEditor::from_markdown_embedded(cx, "alpha".to_owned()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_subscription = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&editor, move |_editor, event, _cx| {
            events_for_subscription
                .lock()
                .expect("event list should not be poisoned")
                .push(event.clone());
        })
    });

    editor.update(cx, |editor, cx| {
        assert_eq!("alpha", editor.markdown(cx));
        assert_eq!(0, editor.revision());
        assert!(!editor.is_dirty());

        assert!(editor.replace_markdown("# beta".to_owned(), cx));
        assert_eq!("# beta", editor.markdown(cx));
        assert_eq!(1, editor.revision());
        assert!(editor.is_dirty());
    });
    cx.run_until_parked();

    assert_eq!(
        vec![MarkdownEditorEvent::Changed { revision: 1 }],
        *events.lock().expect("event list should not be poisoned")
    );

    editor.update(cx, |editor, cx| {
        assert!(!editor.replace_markdown("# beta".to_owned(), cx));
        assert_eq!(1, editor.revision());
    });
    cx.run_until_parked();
    assert_eq!(
        1,
        events
            .lock()
            .expect("event list should not be poisoned")
            .len()
    );
}

#[gpui::test]
async fn adapter_uses_velotype_history_and_saved_state(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| MarkdownEditor::from_markdown_embedded(cx, "alpha".to_owned()));

    editor.update(cx, |editor, cx| {
        assert!(editor.replace_markdown("beta".to_owned(), cx));
        editor.mark_saved(cx);
        assert!(!editor.is_dirty());
        assert_eq!(1, editor.revision());

        assert!(editor.undo(cx));
        assert_eq!("alpha", editor.markdown(cx));
        assert!(editor.redo(cx));
        assert_eq!("beta", editor.markdown(cx));
        assert_eq!(3, editor.revision());
    });
}

#[gpui::test]
async fn adapter_delegates_rendered_and_source_views(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| MarkdownEditor::from_markdown_embedded(cx, "# title".to_owned()));

    editor.update(cx, |editor, cx| {
        assert_eq!(ViewMode::Rendered, editor.view_mode());
        assert!(editor.set_view_mode(ViewMode::Source, cx));
        assert_eq!(ViewMode::Source, editor.view_mode());
        assert!(!editor.set_view_mode(ViewMode::Source, cx));
        assert!(editor.set_view_mode(ViewMode::Rendered, cx));
        assert_eq!("# title", editor.markdown(cx));
        assert_eq!(0, editor.revision());
        assert!(!editor.is_dirty());
    });
}

#[gpui::test]
async fn adapter_keeps_a_real_paragraph_after_structural_blocks(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let editor = cx.new(|cx| {
        MarkdownEditor::from_markdown_embedded(cx, "```rust\nfn main() {}\n```".to_owned())
    });

    editor.update(cx, |editor, cx| {
        assert_eq!(2, editor.root_block_count());
        assert!(editor.trailing_block_is_paragraph(cx));
        assert_eq!("```rust\nfn main() {}\n```\n\n", editor.markdown(cx));
    });
}

#[gpui::test]
async fn adapter_focuses_the_real_embedded_editor(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let (editor, cx) = cx.add_window_view(|_window, cx| {
        MarkdownEditor::from_markdown_embedded(cx, "```rust\nfn main() {}\n```".to_owned())
    });

    let focused = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.focus(window, cx));
        });
        editor.read_with(cx, |editor, cx| editor.has_focus(window, cx))
    });

    assert!(focused);
}

#[gpui::test]
async fn adapter_installs_and_replaces_host_services_without_mutating_document_state(
    cx: &mut TestAppContext,
) {
    init_editor_test_app(cx);
    let services = markdown_editor_host_services(test_theme(), None);
    let editor = cx.new(|cx| {
        MarkdownEditor::from_markdown_embedded_with_host(cx, "alpha".to_owned(), services)
    });

    editor.update(cx, |editor, cx| {
        assert!(editor.has_code_highlight_provider());
        assert!(!editor.has_block_render_provider());
        assert_eq!(0, editor.host_services_revision());

        let markdown = editor.markdown(cx);
        let revision = editor.revision();
        let dirty = editor.is_dirty();
        editor.set_host_services(markdown_editor_host_services(test_theme(), None), cx);

        assert_eq!(markdown, editor.markdown(cx));
        assert_eq!(revision, editor.revision());
        assert_eq!(dirty, editor.is_dirty());
        assert_eq!(1, editor.host_services_revision());
        assert!(editor.has_code_highlight_provider());
    });
}

#[gpui::test]
async fn host_services_survive_view_rebuild_replace_reload_and_history(cx: &mut TestAppContext) {
    init_editor_test_app(cx);
    let services = markdown_editor_host_services(test_theme(), None);
    let editor = cx.new(|cx| {
        MarkdownEditor::from_markdown_embedded_with_host(cx, "# alpha".to_owned(), services)
    });

    editor.update(cx, |editor, cx| {
        assert!(editor.set_view_mode(ViewMode::Source, cx));
        assert!(editor.has_code_highlight_provider());
        assert!(editor.set_view_mode(ViewMode::Rendered, cx));
        assert!(editor.has_code_highlight_provider());

        assert!(editor.replace_markdown("```rs\nfn main() {}\n```".to_owned(), cx));
        assert!(editor.has_code_highlight_provider());
        assert!(editor.undo(cx));
        assert!(editor.has_code_highlight_provider());
        assert!(editor.redo(cx));
        assert!(editor.has_code_highlight_provider());

        assert!(editor.reload_markdown("$$\nx + y\n$$".to_owned(), cx));
        assert!(editor.has_code_highlight_provider());
        assert_eq!(0, editor.host_services_revision());
    });
}
