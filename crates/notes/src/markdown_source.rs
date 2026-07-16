use crate::NotesView;
use gpui::{AppContext, Context, Entity, Subscription, WeakEntity, Window};
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
    view: WeakEntity<NotesView>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Subscription {
    cx.subscribe_in(input, window, move |_view, input, event, window, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let source = input.read(cx).value().to_string();
        let _ = view.update(cx, |view, cx| {
            view.markdown_source_changed(&document_id, source, window, cx);
        });
    })
}
