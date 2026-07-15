use crate::{MarkdownViewMode, NotesView};
use cditor_app::{EditorSaveState, MarkdownCompatibility};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
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
        let session = self.markdown_sessions.get(document_id);
        let needs_acceptance = session.is_some_and(|session| {
            mode == MarkdownViewMode::Wysiwyg
                && !session.normalization_accepted
                && matches!(
                    session.compatibility,
                    MarkdownCompatibility::EditableWithNormalization(_)
                )
        });
        let save_state = session.map(|session| session.preview.save_state(cx));
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
                    .child(markdown_status(session, mode, save_state.as_ref())),
            )
            .when(needs_acceptance, |toolbar| {
                let id = document_id.to_owned();
                toolbar.child(
                    Button::new("accept-markdown-normalization")
                        .label("允许规范化并编辑")
                        .small()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.accept_markdown_normalization(&id, cx)
                        })),
                )
            })
    }

    fn markdown_mode_button(
        &self,
        document_id: &str,
        target: MarkdownViewMode,
        current: MarkdownViewMode,
        cx: &mut Context<Self>,
    ) -> Button {
        let id = document_id.to_owned();
        let disabled = self
            .markdown_sessions
            .get(document_id)
            .is_some_and(|session| {
                !matches!(
                    session.state.sync_state,
                    crate::markdown_session::MarkdownSyncState::Clean
                )
            });
        Button::new(match target {
            MarkdownViewMode::Source => "markdown-source-mode",
            MarkdownViewMode::Wysiwyg => "markdown-wysiwyg-mode",
        })
        .label(match target {
            MarkdownViewMode::Source => "源码",
            MarkdownViewMode::Wysiwyg => "所见即所得",
        })
        .small()
        .disabled(disabled)
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

fn markdown_status(
    session: Option<&crate::markdown_session::MarkdownSession>,
    mode: MarkdownViewMode,
    save_state: Option<&EditorSaveState>,
) -> String {
    if mode == MarkdownViewMode::Source {
        return source_status(session);
    }
    let Some(session) = session else {
        return "Markdown projection 不可用".to_owned();
    };
    match save_state {
        Some(EditorSaveState::Dirty) => return "等待自动保存…".to_owned(),
        Some(EditorSaveState::Saving) => return "正在保存 Markdown…".to_owned(),
        Some(EditorSaveState::SaveFailed { message }) => {
            return format!("保存失败：{message}");
        }
        _ => {}
    }
    match &session.compatibility {
        MarkdownCompatibility::Editable => "所见即所得编辑已启用".to_owned(),
        MarkdownCompatibility::EditableWithNormalization(_) if session.normalization_accepted => {
            "已允许规范化，所见即所得编辑已启用".to_owned()
        }
        MarkdownCompatibility::EditableWithNormalization(_) => {
            format!("需确认规范化（{} 项提示）", session.diagnostics.len())
        }
        MarkdownCompatibility::SourceOnly(_) => {
            format!("源码专用，只读预览（{} 项问题）", session.diagnostics.len())
        }
    }
}

fn source_status(session: Option<&crate::markdown_session::MarkdownSession>) -> String {
    use crate::markdown_session::MarkdownSyncState;
    match session.map(|session| &session.state.sync_state) {
        Some(MarkdownSyncState::SourceDirty | MarkdownSyncState::SavingSource) => {
            "正在保存 Markdown…".to_owned()
        }
        Some(MarkdownSyncState::Conflict) => "外部文件已修改，未覆盖".to_owned(),
        Some(MarkdownSyncState::Failed(message)) => format!("保存失败：{message}"),
        _ => "Markdown 文件为唯一真源".to_owned(),
    }
}
