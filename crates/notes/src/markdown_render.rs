use crate::{MarkdownViewMode, NotesView};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

impl NotesView {
    pub(crate) fn render_markdown_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(document_id) = self.active_document_id.as_ref() else {
            return div().into_any_element();
        };
        let Some(session) = self.markdown_sessions.get(document_id) else {
            return div().into_any_element();
        };
        let mode = session.state.mode;
        let content = match mode {
            MarkdownViewMode::Source => Input::new(&session.source_editor)
                .size_full()
                .into_any_element(),
            MarkdownViewMode::Wysiwyg => session.preview.entity().clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(self.render_markdown_toolbar(document_id, mode, cx))
            .child(div().flex_1().min_h_0().min_w_0().child(content))
            .into_any_element()
    }

    fn render_markdown_toolbar(
        &self,
        document_id: &str,
        mode: MarkdownViewMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .h_9()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.markdown_mode_button(document_id, MarkdownViewMode::Source, mode, cx))
            .child(self.markdown_mode_button(document_id, MarkdownViewMode::Wysiwyg, mode, cx))
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(match mode {
                        MarkdownViewMode::Source => "Markdown 文件为唯一真源",
                        MarkdownViewMode::Wysiwyg => "当前为只读预览，编辑请切回源码",
                    }),
            )
    }

    fn markdown_mode_button(
        &self,
        document_id: &str,
        target: MarkdownViewMode,
        current: MarkdownViewMode,
        cx: &mut Context<Self>,
    ) -> Button {
        let id = document_id.to_owned();
        Button::new(match target {
            MarkdownViewMode::Source => "markdown-source-mode",
            MarkdownViewMode::Wysiwyg => "markdown-wysiwyg-mode",
        })
        .label(match target {
            MarkdownViewMode::Source => "源码",
            MarkdownViewMode::Wysiwyg => "所见即所得",
        })
        .small()
        .when(current == target, |button| button.primary())
        .on_click(cx.listener(move |view, _, window, cx| {
            view.set_markdown_mode(id.clone(), target, window, cx)
        }))
    }

    fn set_markdown_mode(
        &mut self,
        document_id: String,
        mode: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .markdown_sessions
            .get(&document_id)
            .map(|session| session.state.mode);
        if current != Some(mode) {
            self.toggle_markdown_mode(document_id, window, cx);
        }
    }
}
