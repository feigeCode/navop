use super::MarkdownEditor;
use crate::{MarkdownEditorTheme, MarkdownProjection};
use gpui::{AppContext, Context, Entity, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};

pub(super) fn create_input(
    value: &str,
    window: &mut Window,
    cx: &mut Context<MarkdownEditor>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .multi_line(true)
            .soft_wrap(true)
            .default_value(value)
    })
}

pub(super) fn create_property_input(
    window: &mut Window,
    cx: &mut Context<MarkdownEditor>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("Image property"))
}

pub(super) fn apply_projection_styles(
    input: &Entity<InputState>,
    projection: &MarkdownProjection,
    theme: &MarkdownEditorTheme,
    cx: &mut Context<MarkdownEditor>,
) {
    input.update(cx, |input, cx| {
        input.set_text_highlights(super::projection_highlights(projection, theme), cx);
    });
}

pub(super) fn subscribe_to_input(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<MarkdownEditor>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe_in(input, window, |editor, _, event, window, cx| match event {
            InputEvent::Change => editor.input_changed(window, cx),
            InputEvent::PressEnter { secondary } => {
                editor.input_entered(*secondary, window, cx);
            }
            _ => {}
        }),
        cx.observe_in(input, window, |editor, _, window, cx| {
            editor.cursor_changed(window, cx);
        }),
    ]
}
