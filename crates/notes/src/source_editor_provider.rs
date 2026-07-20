use cditor_app::{SourceEditorConfig, SourceEditorProvider, SourceEditorSession};
use gpui::{App, AppContext, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::{Input, InputState};

#[derive(Default)]
pub(crate) struct NotesSourceEditorProvider;

impl SourceEditorProvider for NotesSourceEditorProvider {
    fn supports_language(&self, language: &str) -> bool {
        language.eq_ignore_ascii_case("html") || language.eq_ignore_ascii_case("markdown")
    }

    fn create(
        &self,
        config: SourceEditorConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> SourceEditorSession {
        let line_count = config.initial_value.lines().count().max(1);
        let height = (line_count as f32 * 21.0 + 28.0).clamp(84.0, 640.0);
        let language = config.language.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language)
                .line_number(config.line_numbers)
                .multi_line(true)
                .soft_wrap(config.soft_wrap)
                .default_value(config.initial_value)
        });
        input.update(cx, |input, cx| input.focus(window, cx));

        let value_input = input.clone();
        let focus_input = input.clone();
        let render_input = input;
        SourceEditorSession::new(
            move |cx| value_input.read(cx).value().to_string(),
            move |window, cx| {
                focus_input.update(cx, |input, cx| input.focus(window, cx));
            },
            move |_window, _cx| {
                div()
                    .w_full()
                    .h(px(height))
                    .min_h(px(84.0))
                    .max_h(px(640.0))
                    .child(Input::new(&render_input).size_full())
                    .into_any_element()
            },
        )
    }
}
