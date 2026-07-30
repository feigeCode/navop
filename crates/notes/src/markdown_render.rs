use crate::markdown_session::MarkdownSyncState;
use crate::{MarkdownSaveMode, MarkdownViewMode, NotesView};
use gpui::{
    AnyElement, Context, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
};
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle, Paste},
    popover::Popover,
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
                .on_action(cx.listener(|view, _: &Paste, window, cx| {
                    if !view.paste_markdown_images(window, cx) {
                        cx.propagate();
                    }
                }))
                .drag_over::<ExternalPaths>(|element, _, _, _| element.bg(gpui::rgba(0x3b82f614)))
                .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
                    view.drop_markdown_images(paths, window, cx);
                }))
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
            .on_action(cx.listener(
                |view, _: &crate::markdown_source::OpenMarkdownSearch, window, cx| {
                    view.open_markdown_search(window, cx);
                },
            ))
            .on_action(cx.listener(
                |view, _: &crate::markdown_source::ToggleMarkdownOutline, _window, cx| {
                    view.toggle_markdown_outline(cx);
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
        let statistics =
            session.map(|session| document_statistics(session.preview.read(cx).projected_text()));
        let toolbar_status = statistics.map(|statistics| {
            let metrics = t!(
                "Notes.markdown_statistics",
                words = statistics.words,
                characters = statistics.characters,
                lines = statistics.lines
            )
            .to_string();
            status.map_or(metrics.clone(), |status| format!("{status} · {metrics}"))
        });
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
            .child(self.render_outline(document_id, cx))
            .child(
                Button::new("markdown-search")
                    .debug_selector(|| "markdown-search".to_owned())
                    .icon(IconName::Search)
                    .tooltip(t!("Notes.markdown_search_tooltip").to_string())
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_markdown_search(window, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .when_some(toolbar_status, |status_view, status| {
                        status_view
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(status)
                    }),
            )
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

    fn render_outline(&self, document_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.markdown_sessions.get(document_id) else {
            return div().into_any_element();
        };
        let headings = session.preview.read(cx).headings();
        let preview = session.preview.clone();
        let view = cx.entity();
        Popover::new("markdown-outline")
            .open(self.markdown_outline_open && !headings.is_empty())
            .on_open_change(move |open, _, cx| {
                view.update(cx, |view, cx| {
                    if view.markdown_outline_open != *open {
                        view.markdown_outline_open = *open;
                        cx.notify();
                    }
                });
            })
            .trigger(
                Button::new("markdown-outline-trigger")
                    .debug_selector(|| "markdown-outline-trigger".to_owned())
                    .icon(IconName::Menu)
                    .tooltip(t!("Notes.markdown_outline_tooltip").to_string())
                    .ghost()
                    .small()
                    .disabled(headings.is_empty()),
            )
            .content(move |_state, _window, cx| {
                let popover = cx.entity();
                v_flex()
                    .id("markdown-outline-content")
                    .debug_selector(|| "markdown-outline-content".to_owned())
                    .min_w_48()
                    .max_w_96()
                    .max_h_96()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_1()
                    .children(headings.clone().into_iter().map(|heading| {
                        let preview = preview.clone();
                        let popover = popover.clone();
                        Button::new(("markdown-outline-heading", heading.block_id.0))
                            .label(format!(
                                "{} {}",
                                "#".repeat(heading.level as usize),
                                heading.title
                            ))
                            .ghost()
                            .small()
                            .on_click(move |_, window, cx| {
                                preview.update(cx, |editor, cx| {
                                    editor.activate_block(heading.block_id, window, cx);
                                });
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                    }))
                    .into_any_element()
            })
            .into_any_element()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkdownDocumentStatistics {
    words: usize,
    characters: usize,
    lines: usize,
}

fn document_statistics(source: &str) -> MarkdownDocumentStatistics {
    let mut words = 0;
    let mut in_latin_word = false;
    for character in source.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            if !in_latin_word {
                words += 1;
                in_latin_word = true;
            }
        } else {
            in_latin_word = false;
            if is_cjk_character(character) {
                words += 1;
            }
        }
    }
    MarkdownDocumentStatistics {
        words,
        characters: source
            .chars()
            .filter(|character| !character.is_whitespace())
            .count(),
        lines: (!source.is_empty())
            .then(|| source.lines().count())
            .unwrap_or(0),
    }
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
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

#[cfg(test)]
mod tests {
    use super::{MarkdownDocumentStatistics, document_statistics};

    #[test]
    fn statistics_count_latin_runs_and_cjk_characters() {
        assert_eq!(
            MarkdownDocumentStatistics {
                words: 4,
                characters: 11,
                lines: 2,
            },
            document_statistics("hello 世界\nRust")
        );
        assert_eq!(
            MarkdownDocumentStatistics {
                words: 0,
                characters: 0,
                lines: 0,
            },
            document_statistics("")
        );
    }
}
