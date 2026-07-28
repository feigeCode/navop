use crate::markdown_session::MarkdownSyncState;
use crate::{MarkdownSaveMode, MarkdownViewMode, NotesView};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder,
};
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::Button,
    h_flex,
    input::{Input, LocalInputStyle},
    switch::Switch,
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
            MarkdownViewMode::Source => {
                let theme = self.resolved_editor_theme(cx);
                div()
                    .key_context(crate::markdown_source::SOURCE_CONTEXT)
                    .on_action(cx.listener(
                        |view, _: &crate::markdown_source::UndoSourceMode, window, cx| {
                            view.apply_source_mode_history(true, window, cx);
                        },
                    ))
                    .on_action(cx.listener(
                        |view, _: &crate::markdown_source::RedoSourceMode, window, cx| {
                            view.apply_source_mode_history(false, window, cx);
                        },
                    ))
                    .on_action(cx.listener(
                        |view, _: &crate::markdown_source::SaveMarkdown, window, cx| {
                            view.save_active_markdown(window, cx);
                        },
                    ))
                    .size_full()
                    .child(
                        Input::new(&session.source_editor)
                            .size_full()
                            .local_style(LocalInputStyle {
                                background: theme.background,
                                foreground: theme.foreground,
                                muted_foreground: theme.muted_foreground,
                                border: theme.border,
                            })
                            .highlight_theme(theme.highlight_theme)
                            .caret_color(theme.primary)
                            .indent_guide_color(theme.border.opacity(0.7)),
                    )
                    .into_any_element()
            }
            MarkdownViewMode::Wysiwyg => div()
                .id("markdown-wysiwyg-editor")
                .debug_selector(|| "markdown-wysiwyg-editor".to_owned())
                .size_full()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(session.preview.clone())
                .into_any_element(),
        };
        v_flex()
            .key_context(crate::markdown_source::MARKDOWN_CONTEXT)
            .on_action(cx.listener(
                |view, _: &crate::markdown_source::SaveMarkdown, window, cx| {
                    view.save_active_markdown(window, cx);
                },
            ))
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
        let theme = self.resolved_editor_theme(cx);
        let keep_id = document_id.to_owned();
        let external_id = document_id.to_owned();
        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.warning.opacity(0.12))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(theme.warning),
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
        let theme = self.resolved_editor_theme(cx);
        let session = self.markdown_sessions.get(document_id);
        let status = markdown_status(session, self.tree.markdown_save_mode);
        let save_disabled = session.is_none_or(|session| {
            !matches!(
                session.state.sync_state,
                MarkdownSyncState::SourceDirty | MarkdownSyncState::Failed(_)
            )
        });
        h_flex()
            .id("markdown-mode-toolbar")
            .debug_selector(|| "markdown-mode-toolbar".to_owned())
            .h_9()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(self.render_source_toggle(document_id, mode, cx))
            .child(div().flex_1().when_some(status, |status_view, status| {
                status_view
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(status)
            }))
            .child(
                div()
                    .debug_selector(|| "markdown-auto-save".to_owned())
                    .child(
                        Switch::new("markdown-auto-save-switch")
                            .small()
                            .checked(self.tree.markdown_save_mode == MarkdownSaveMode::Automatic)
                            .label(t!("Notes.markdown_auto_save").to_string())
                            .tooltip(t!("Notes.markdown_auto_save_tooltip").to_string())
                            .on_click(cx.listener(|view, checked: &bool, window, cx| {
                                view.set_markdown_save_mode(
                                    if *checked {
                                        MarkdownSaveMode::Automatic
                                    } else {
                                        MarkdownSaveMode::Manual
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                Button::new("markdown-save-now")
                    .debug_selector(|| "markdown-save-now".to_owned())
                    .label(t!("Notes.markdown_save_now").to_string())
                    .tooltip(t!("Notes.markdown_save_now_tooltip").to_string())
                    .small()
                    .disabled(save_disabled)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.save_active_markdown(window, cx);
                    })),
            )
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
            .debug_selector(|| "markdown-source-mode".to_owned())
            .icon(if mode == MarkdownViewMode::Source {
                IconName::Eye
            } else {
                IconName::Edit
            })
            .label(if mode == MarkdownViewMode::Source {
                t!("Notes.markdown_preview").to_string()
            } else {
                t!("Notes.markdown_source").to_string()
            })
            .tooltip(if mode == MarkdownViewMode::Source {
                t!("Notes.markdown_preview_tooltip").to_string()
            } else {
                t!("Notes.markdown_source_tooltip").to_string()
            })
            .small()
            .disabled(disabled)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.toggle_markdown_mode(id.clone(), window, cx)
            }))
    }
}

fn markdown_status(
    session: Option<&crate::markdown_session::MarkdownSession>,
    save_mode: MarkdownSaveMode,
) -> Option<String> {
    match session.map(|session| &session.state.sync_state) {
        Some(MarkdownSyncState::Clean) => Some(t!("Notes.markdown_saved").to_string()),
        Some(MarkdownSyncState::SourceDirty) if save_mode == MarkdownSaveMode::Automatic => {
            Some(t!("Notes.markdown_waiting_autosave").to_string())
        }
        Some(MarkdownSyncState::SourceDirty) => Some(t!("Notes.markdown_unsaved").to_string()),
        Some(MarkdownSyncState::SavingSource) => Some(t!("Notes.markdown_saving").to_string()),
        Some(MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)) => None,
        _ => None,
    }
}
