use crate::NotesView;
use gpui::{App, AppContext, Context, Entity, KeyBinding, Subscription, Window, actions};
use gpui_component::input::{InputEvent, InputState};

pub(crate) const SOURCE_CONTEXT: &str = "NotesMarkdownSource";
pub(crate) const MARKDOWN_CONTEXT: &str = "NotesMarkdown";
const SOURCE_INPUT_CONTEXT: &str = "NotesMarkdownSource > Input";
actions!(
    notes_markdown_source,
    [UndoSourceMode, RedoSourceMode, SaveMarkdown]
);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-s", SaveMarkdown, Some(MARKDOWN_CONTEXT)),
        KeyBinding::new("secondary-s", SaveMarkdown, Some(SOURCE_INPUT_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-z", UndoSourceMode, Some(SOURCE_INPUT_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-z", RedoSourceMode, Some(SOURCE_INPUT_CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-z", UndoSourceMode, Some(SOURCE_INPUT_CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", RedoSourceMode, Some(SOURCE_INPUT_CONTEXT)),
    ]);
}

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
    preview: &Entity<markdown_editor::MarkdownEditor>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Subscription {
    let preview = preview.clone();
    cx.subscribe_in(input, window, move |_, input, event, window, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let input = input.read(cx);
        let source = input.value();
        let range = input.selected_range();
        let selection = markdown_source::SourceSelection {
            anchor: range.start,
            head: range.end,
        };
        let applied = preview.update(cx, |editor, cx| {
            editor.apply_source_value(source.as_ref(), selection, window, cx)
        });
        match applied {
            Ok(true) => {}
            Ok(false) => {}
            Err(error) => {
                crate::notes_notifications::notify_operation_error(window, cx, error);
            }
        }
    })
}
