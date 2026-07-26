use super::*;
use crate::MarkdownEditorTheme;
use gpui::{
    AppContext, Context, Entity, IntoElement, Render, TestAppContext, VisualTestContext, Window,
    WindowHandle, WindowOptions,
};
use gpui_component::highlighter::HighlightTheme;
use markdown_source::{SourceMarkdownDocument, SourceSelection, TableCellAddress};
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

#[gpui::test]
fn switching_blocks_preserves_each_surface_selection_and_identity(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Alpha surface text\n\nBeta surface text\n\nFollowing paragraph";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let first_key = MarkdownSurfaceKey::block(document.blocks[0].id);
    let second_key = MarkdownSurfaceKey::block(document.blocks[1].id);
    let (window, editor) = open_test_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let (first_input, second_input) = editor.read_with(&cx, |editor, _| {
        (
            editor.surface(first_key).unwrap().input.clone(),
            editor.surface(second_key).unwrap().input.clone(),
        )
    });
    let first_input_id = first_input.entity_id();
    let second_input_id = second_input.entity_id();
    assert_ne!(first_input_id, second_input_id);

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(first_key, window, cx));
    });
    first_input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(2..5, false, window, cx);
    });
    cx.run_until_parked();

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(second_key, window, cx));
    });
    second_input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(5..12, false, window, cx);
    });
    cx.run_until_parked();

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(first_key, window, cx));
    });
    cx.run_until_parked();
    editor.read_with(&cx, |editor, cx| {
        assert_eq!(Some(first_key), editor.active_surface);
        assert_eq!(first_input_id, editor.input.entity_id());
        assert_eq!(
            first_input_id,
            editor.surface(first_key).unwrap().input.entity_id()
        );
        assert_eq!(
            second_input_id,
            editor.surface(second_key).unwrap().input.entity_id()
        );
        assert_eq!(
            2..5,
            editor
                .surface(first_key)
                .unwrap()
                .input
                .read(cx)
                .selected_range()
        );
        assert_eq!(
            5..12,
            editor
                .surface(second_key)
                .unwrap()
                .input
                .read(cx)
                .selected_range()
        );
    });

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(second_key, window, cx));
    });
    cx.run_until_parked();
    editor.read_with(&cx, |editor, cx| {
        assert_eq!(Some(second_key), editor.active_surface);
        assert_eq!(second_input_id, editor.input.entity_id());
        assert_eq!(2..5, first_input.read(cx).selected_range());
        assert_eq!(5..12, second_input.read(cx).selected_range());
    });

    second_input.update_in(&mut cx, |input, window, cx| {
        input.replace("local", window, cx);
    });
    cx.run_until_parked();

    editor.read_with(&cx, |editor, cx| {
        assert_eq!(
            "Alpha surface text\n\nBeta local text\n\nFollowing paragraph",
            editor.source()
        );
        assert_eq!(first_input_id, first_input.entity_id());
        assert_eq!(second_input_id, second_input.entity_id());
        assert_eq!(
            first_input_id,
            editor.surface(first_key).unwrap().input.entity_id()
        );
        assert_eq!(
            second_input_id,
            editor.surface(second_key).unwrap().input.entity_id()
        );
        assert_eq!(first_input.read(cx).value(), "Alpha surface text");
        assert_eq!(second_input.read(cx).value(), "Beta local text");
        assert_eq!(2..5, first_input.read(cx).selected_range());
    });
}

#[gpui::test]
fn switching_table_cells_preserves_local_selection_and_edits_only_the_target_cell(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let source = "| Left | Right |\n| --- | --- |\n| Alpha cell | Beta cell |\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let table_id = document.blocks[0].id;
    let left_address = TableCellAddress {
        block_id: table_id,
        row: 2,
        column: 0,
    };
    let right_address = TableCellAddress {
        block_id: table_id,
        row: 2,
        column: 1,
    };
    let left_key = MarkdownSurfaceKey::table_cell(left_address);
    let right_key = MarkdownSurfaceKey::table_cell(right_address);
    let (window, editor) = open_test_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let (left_input, right_input) = editor.read_with(&cx, |editor, _| {
        (
            editor.surface(left_key).unwrap().input.clone(),
            editor.surface(right_key).unwrap().input.clone(),
        )
    });
    let left_input_id = left_input.entity_id();
    let right_input_id = right_input.entity_id();
    assert_ne!(left_input_id, right_input_id);

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(left_key, window, cx));
    });
    left_input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(0..5, false, window, cx);
    });
    cx.run_until_parked();

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(right_key, window, cx));
    });
    right_input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(0..4, false, window, cx);
    });
    cx.run_until_parked();

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(left_key, window, cx));
    });
    cx.run_until_parked();
    editor.read_with(&cx, |editor, cx| {
        assert_eq!(Some(left_key), editor.active_surface);
        assert_eq!(Some(left_address), editor.active_table_cell());
        assert_eq!(left_input_id, editor.input.entity_id());
        assert_eq!(
            left_input_id,
            editor.surface(left_key).unwrap().input.entity_id()
        );
        assert_eq!(
            right_input_id,
            editor.surface(right_key).unwrap().input.entity_id()
        );
        assert_eq!(0..5, left_input.read(cx).selected_range());
        assert_eq!(0..4, right_input.read(cx).selected_range());
    });

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.focus_surface(right_key, window, cx));
    });
    cx.run_until_parked();
    editor.read_with(&cx, |editor, cx| {
        assert_eq!(Some(right_key), editor.active_surface);
        assert_eq!(Some(right_address), editor.active_table_cell());
        assert_eq!(right_input_id, editor.input.entity_id());
        assert_eq!(0..5, left_input.read(cx).selected_range());
        assert_eq!(0..4, right_input.read(cx).selected_range());
    });

    right_input.update_in(&mut cx, |input, window, cx| {
        input.replace("Gamma", window, cx);
    });
    cx.run_until_parked();

    editor.read_with(&cx, |editor, cx| {
        assert_eq!(
            "| Left | Right |\n| --- | --- |\n| Alpha cell | Gamma cell |\n",
            editor.source()
        );
        assert_eq!(left_input_id, left_input.entity_id());
        assert_eq!(right_input_id, right_input.entity_id());
        assert_eq!(
            left_input_id,
            editor.surface(left_key).unwrap().input.entity_id()
        );
        assert_eq!(
            right_input_id,
            editor.surface(right_key).unwrap().input.entity_id()
        );
        assert_eq!(left_input.read(cx).value(), "Alpha cell");
        assert_eq!(right_input.read(cx).value(), "Gamma cell");
        assert_eq!(0..5, left_input.read(cx).selected_range());
    });
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
