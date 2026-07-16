use crate::NotesView;
use gpui::{AppContext, Context, Entity, Subscription, Window};
use gpui_component::input::{InputEvent, InputState};

pub(crate) fn create_source_editor(
    source: &str,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor("markdown")
            .line_number(true)
            .multi_line(true)
            .soft_wrap(true)
            .default_value(source)
    })
}

pub(crate) fn subscribe_source_changes(
    input: &Entity<InputState>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Subscription {
    cx.subscribe_in(input, window, move |view, input, event, window, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let source = input.read(cx).value().to_string();
        view.markdown_source_changed(&document_id, source, window, cx);
    })
}
