use cditor_app::{SourceEditorConfig, SourceEditorProvider, SourceEditorSession};
use gpui::{App, AppContext, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::{Input, InputState};

#[derive(Default)]
pub(crate) struct NotesSourceEditorProvider;

impl SourceEditorProvider for NotesSourceEditorProvider {
    fn supports_language(&self, language: &str) -> bool {
        !language.trim().is_empty()
    }

    fn create(
        &self,
        config: SourceEditorConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> SourceEditorSession {
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
        let render_input = input.clone();
        let height_input = input;
        SourceEditorSession::new(
            move |cx| value_input.read(cx).value().to_string(),
            move |window, cx| {
                focus_input.update(cx, |input, cx| input.focus(window, cx));
            },
            move |_window, cx| {
                let height = source_editor_height(&render_input.read(cx).value());
                div()
                    .min_w_0()
                    .w_full()
                    .h(px(height))
                    .min_h(px(48.0))
                    .max_h(px(640.0))
                    .overflow_hidden()
                    .child(Input::new(&render_input).size_full())
                    .into_any_element()
            },
        )
        .with_preferred_height_provider(move |cx| {
            source_editor_height(&height_input.read(cx).value())
        })
    }
}

fn source_editor_height(value: &str) -> f32 {
    let line_count = value.split('\n').count().max(1);
    (line_count as f32 * 21.0 + 18.0).clamp(48.0, 640.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_editor_accepts_document_and_code_languages() {
        let provider = NotesSourceEditorProvider;

        for language in [
            "html", "markdown", "latex", "math", "mermaid", "rust", "sql", "text",
        ] {
            assert!(provider.supports_language(language), "language={language}");
        }
    }

    #[test]
    fn source_editor_height_tracks_current_lines_without_large_short_value_gap() {
        assert_eq!(source_editor_height(""), 48.0);
        assert_eq!(source_editor_height("x"), 48.0);
        assert_eq!(source_editor_height("x\ny"), 60.0);
        assert_eq!(source_editor_height("x\n"), 60.0);
        assert_eq!(source_editor_height(&"x\n".repeat(100)), 640.0);
    }
}
