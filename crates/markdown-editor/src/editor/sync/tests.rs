use super::*;
use crate::MarkdownEditorTheme;
use gpui::{
    AppContext, Context, IntoElement, Render, TestAppContext, VisualTestContext, Window,
    WindowOptions,
};
use gpui_component::highlighter::HighlightTheme;
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
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                editor = Some(cx.new(|cx| {
                    MarkdownEditor::new("A **stable** paragraph", test_theme(), window, cx).unwrap()
                }));
                cx.new(|_| EmptyView)
            })
            .unwrap();
        (window, editor.unwrap())
    });
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
