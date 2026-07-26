use super::*;
use crate::MarkdownEditorTheme;
use gpui::{
    AppContext, Context, Entity, IntoElement, Render, TestAppContext, VisualTestContext, Window,
    WindowHandle, WindowOptions,
};
use gpui_component::highlighter::HighlightTheme;
use markdown_source::{SourceMarkdownDocument, SourceSelection};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn open_test_editor(
    source: &'static str,
    cx: &mut TestAppContext,
) -> (WindowHandle<EmptyView>, Entity<MarkdownEditor>) {
    cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                editor = Some(
                    cx.new(|cx| MarkdownEditor::new(source, test_theme(), window, cx).unwrap()),
                );
                cx.new(|_| EmptyView)
            })
            .unwrap();
        (window, editor.unwrap())
    })
}

#[test]
fn position_for_offset_snaps_offsets_inside_multibyte_characters() {
    // '新' occupies bytes 3..6; an offset inside it must not panic and
    // resolves to the character start.
    assert_eq!(position_for_offset("新新新", 4), Position::new(0, 1));
    assert_eq!(position_for_offset("新新新", 5), Position::new(0, 1));
    assert_eq!(position_for_offset("新新新", 6), Position::new(0, 2));
    assert_eq!(
        position_for_offset("第一行\n新段落", 11),
        Position::new(1, 0)
    );
    assert_eq!(
        position_for_offset("第一行\n新段落", 13),
        Position::new(1, 1)
    );
    assert_eq!(position_for_offset("abc", usize::MAX), Position::new(0, 3));
}

#[gpui::test]
fn repeated_projection_sync_does_not_notify_an_unchanged_input_mode(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = open_test_editor("A **stable** paragraph", cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.sync_projection(0, window, cx);
    });
    cx.run_until_parked();

    let notifications = Arc::new(AtomicUsize::new(0));
    let subscription = editor.update_in(&mut cx, |editor, _, cx| {
        let input = editor.input.clone();
        let notifications = notifications.clone();
        cx.observe(&input, move |_, _, _| {
            notifications.fetch_add(1, Ordering::Relaxed);
        })
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.sync_projection(0, window, cx);
    });
    cx.run_until_parked();

    assert_eq!(0, notifications.load(Ordering::Relaxed));
    drop(subscription);
}

#[gpui::test]
fn replacing_source_resets_the_active_surface_to_empty(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "First\n\nSecond\n\nThird";
    let old_block = SourceMarkdownDocument::parse(source).unwrap().blocks[2].id;
    let (window, editor) = open_test_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(old_block, window, cx));
        editor.replace_source("Replacement", window, cx).unwrap();
    });

    editor.read_with(&cx, |editor, _| {
        let empty_input = &editor
            .surface(MarkdownSurfaceKey::Empty)
            .expect("empty surface must survive replacement")
            .input;
        assert_eq!(None, editor.active_block());
        assert_eq!(None, editor.active_table_cell());
        assert_eq!(Some(MarkdownSurfaceKey::Empty), editor.active_surface);
        assert_eq!(empty_input.entity_id(), editor.input.entity_id());
        assert!(
            editor
                .surface(MarkdownSurfaceKey::block(old_block))
                .is_none()
        );
    });
}

#[gpui::test]
fn deactivating_collapses_markers_without_unmounting_the_surface(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Before **bold** after\n\nNext";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let block_id = document.blocks[0].id;
    let inline_id = document
        .inline_node_at(source.find("bold").unwrap())
        .unwrap()
        .id;
    let key = MarkdownSurfaceKey::block(block_id);
    let (window, editor) = open_test_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let input_id = editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(block_id, window, cx));
        let cursor = source.find("bold").unwrap() + 1;
        editor.sync_surface_selection(
            key,
            SourceSelection {
                anchor: cursor,
                head: cursor,
            },
            window,
            cx,
        );
        let surface = editor.surface(key).unwrap();
        assert_eq!(Some(inline_id), surface.projection.active_inline);
        surface.input.entity_id()
    });

    editor.update_in(&mut cx, |editor, window, cx| {
        editor.deactivate_block(window, cx);
    });

    editor.read_with(&cx, |editor, _| {
        let surface = editor.surface(key).expect("surface must remain mounted");
        assert_eq!(input_id, surface.input.entity_id());
        assert_eq!(None, surface.projection.active_inline);
        assert_eq!(None, editor.active_block());
        assert_eq!(None, editor.active_table_cell());
        assert_eq!(Some(MarkdownSurfaceKey::Empty), editor.active_surface);
    });
}

#[gpui::test]
fn theme_changes_refresh_inactive_surface_highlights(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "First\n\n[Second](https://example.com)";
    let block_id = SourceMarkdownDocument::parse(source).unwrap().blocks[1].id;
    let (window, editor) = open_test_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let notifications = Arc::new(AtomicUsize::new(0));
    let subscription = editor.update(&mut cx, |editor, cx| {
        let input = editor
            .surface(MarkdownSurfaceKey::block(block_id))
            .unwrap()
            .input
            .clone();
        let notifications = notifications.clone();
        cx.observe(&input, move |_, _, _| {
            notifications.fetch_add(1, Ordering::Relaxed);
        })
    });

    editor.update(&mut cx, |editor, cx| {
        let mut theme = test_theme();
        theme.primary = gpui::rgb(0xff8844).into();
        editor.set_theme(theme, cx);
    });
    cx.run_until_parked();

    assert_ne!(0, notifications.load(Ordering::Relaxed));
    drop(subscription);
}

fn test_theme() -> MarkdownEditorTheme {
    MarkdownEditorTheme {
        background: gpui::rgb(0x111111).into(),
        foreground: gpui::rgb(0xffffff).into(),
        muted_foreground: gpui::rgb(0x999999).into(),
        border: gpui::rgb(0x333333).into(),
        primary: gpui::rgb(0x4488ff).into(),
        highlight_theme: HighlightTheme::default_dark(),
    }
}
