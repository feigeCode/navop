use super::MarkdownEditor;
use super::surface::{MarkdownInputMode, MarkdownSurfaceKey};
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
    key: MarkdownSurfaceKey,
    window: &mut Window,
    cx: &mut Context<MarkdownEditor>,
) -> Vec<Subscription> {
    vec![
        cx.subscribe_in(
            input,
            window,
            move |editor, _, event, window, cx| match event {
                InputEvent::Change => editor.surface_input_changed(key, window, cx),
                InputEvent::PressEnter { secondary } => {
                    editor.surface_input_entered(key, *secondary, window, cx);
                }
                InputEvent::Focus => {
                    editor.surface_focused(key, window, cx);
                }
                InputEvent::Blur => {
                    editor.surface_blurred(key, window, cx);
                }
            },
        ),
        cx.observe_in(input, window, move |editor, _, window, cx| {
            editor.surface_cursor_changed(key, window, cx);
        }),
    ]
}

pub(super) fn apply_surface_mode(
    input: &Entity<InputState>,
    mode: &MarkdownInputMode,
    window: &mut Window,
    cx: &mut Context<MarkdownEditor>,
) {
    input.update(cx, |input, cx| {
        match mode {
            MarkdownInputMode::RichText => input.set_rich_text_mode(window, cx),
            MarkdownInputMode::Code(language) => {
                input.set_code_editor_mode(language.clone(), window, cx);
            }
        }
        input.set_auto_grow_mode(1, usize::MAX, window, cx);
    });
}
