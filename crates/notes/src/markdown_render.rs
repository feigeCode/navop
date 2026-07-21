use crate::markdown_session::MarkdownSyncState;
use crate::{MarkdownViewMode, NotesView};
use cditor_app::{EditorSaveState, MarkdownCompatibility};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use rust_i18n::t;

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
            .when(
                matches!(session.state.sync_state, MarkdownSyncState::Conflict),
                |this| this.child(self.render_conflict_banner(document_id, cx)),
            )
            .child(div().flex_1().min_h_0().min_w_0().child(content))
            .into_any_element()
    }

    fn render_conflict_banner(
        &self,
        document_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let keep_id = document_id.to_owned();
        let external_id = document_id.to_owned();
        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().warning.opacity(0.12))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(cx.theme().warning),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .child(t!("Notes.markdown_conflict_banner").to_string()),
            )
            .child(
                Button::new("markdown-conflict-keep-local")
                    .label(t!("Notes.markdown_conflict_keep_local").to_string())
                    .small()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.resolve_markdown_conflict_keep_local(&keep_id, window, cx)
                    })),
            )
            .child(
                Button::new("markdown-conflict-use-external")
                    .label(t!("Notes.markdown_conflict_use_external").to_string())
                    .small()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.resolve_markdown_conflict_use_external(&external_id, window, cx)
                    })),
            )
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
        let status = markdown_status(session, mode, save_state.as_ref());
        h_flex()
            .h_9()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.render_source_toggle(document_id, mode, cx))
            .child(div().flex_1().when_some(status, |status_view, status| {
                status_view
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
            }))
            .when(needs_acceptance, |toolbar| {
                let id = document_id.to_owned();
                toolbar.child(
                    Button::new("accept-markdown-normalization")
                        .label(t!("Notes.markdown_confirm_adjustment").to_string())
                        .small()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.accept_markdown_normalization(&id, window, cx)
                        })),
                )
            })
    }

    fn render_source_toggle(
        &self,
        document_id: &str,
        mode: MarkdownViewMode,
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
        Button::new("markdown-source-mode")
            .label(t!("Notes.markdown_source").to_string())
            .small()
            .disabled(disabled)
            .when(mode == MarkdownViewMode::Source, |button| button.primary())
            .on_click(cx.listener(move |view, _, window, cx| {
                view.toggle_markdown_mode(id.clone(), window, cx)
            }))
    }
}

fn markdown_status(
    session: Option<&crate::markdown_session::MarkdownSession>,
    mode: MarkdownViewMode,
    save_state: Option<&EditorSaveState>,
) -> Option<String> {
    if mode == MarkdownViewMode::Source {
        return source_status(session);
    }
    let session = session?;
    match save_state {
        Some(EditorSaveState::Dirty) => {
            return Some(t!("Notes.markdown_waiting_autosave").to_string());
        }
        Some(EditorSaveState::Saving) => {
            return Some(t!("Notes.markdown_saving").to_string());
        }
        Some(EditorSaveState::SaveFailed { .. }) => return None,
        _ => {}
    }
    match &session.compatibility {
        MarkdownCompatibility::Editable => None,
        MarkdownCompatibility::EditableWithNormalization(_) if session.normalization_accepted => {
            None
        }
        MarkdownCompatibility::EditableWithNormalization(_) => Some(
            t!(
                "Notes.markdown_adjustment_required",
                count = session.diagnostics.len()
            )
            .to_string(),
        ),
        MarkdownCompatibility::SourceOnly(_) => Some(
            t!(
                "Notes.markdown_source_only",
                count = session.diagnostics.len()
            )
            .to_string(),
        ),
    }
}

fn source_status(session: Option<&crate::markdown_session::MarkdownSession>) -> Option<String> {
    use crate::markdown_session::MarkdownSyncState;
    match session.map(|session| &session.state.sync_state) {
        Some(MarkdownSyncState::SourceDirty | MarkdownSyncState::SavingSource) => {
            Some(t!("Notes.markdown_saving").to_string())
        }
        Some(MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)) => None,
        _ => None,
    }
}
