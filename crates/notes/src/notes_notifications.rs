use crate::NotesView;
use crate::markdown_file_store::MarkdownSaveOutcome;
use crate::markdown_persistence::CONFLICT_MESSAGE;
use cditor_app::EditorEvent;
use gpui::{AppContext, AsyncApp, Context, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::settings::AppSettings;
use rust_i18n::t;

pub(crate) fn notify_operation_error<T>(
    window: &mut Window,
    cx: &mut Context<T>,
    error: impl std::fmt::Display,
) {
    notify_error_message(
        window,
        cx,
        t!("Notes.operation_failed", error = format!("{error:#}")),
    );
}

pub(crate) fn notify_error_message<T>(
    window: &mut Window,
    cx: &mut Context<T>,
    message: impl Into<String>,
) {
    window.push_notification(Notification::error(message.into()).autohide(false), cx);
}

impl NotesView {
    pub(crate) fn finish_markdown_save(
        &mut self,
        document_id: &str,
        generation: u64,
        result: anyhow::Result<Option<MarkdownSaveOutcome>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let notification = match result {
            Ok(Some(MarkdownSaveOutcome::Saved(_))) => {
                session.state.source_saved(generation);
                None
            }
            Ok(Some(MarkdownSaveOutcome::Conflict(_))) => {
                session.state.conflict();
                Some(Notification::warning(
                    t!("Notes.markdown_external_change").to_string(),
                ))
            }
            Ok(None) => None,
            Err(error) => {
                session
                    .state
                    .source_save_failed(generation, error.to_string());
                Some(Notification::error(
                    t!("Notes.markdown_save_failed", error = error.to_string()).to_string(),
                ))
            }
        };
        if let Some(notification) = notification {
            window.push_notification(notification.autohide(false), cx);
        }
        cx.notify();
    }

    pub(crate) fn observe_editor_events(
        &self,
        events: smol::channel::Receiver<EditorEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let window_handle = window.window_handle();
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            while let Ok(event) = events.recv().await {
                match event {
                    EditorEvent::SaveFailed {
                        document_id,
                        message,
                        ..
                    } => {
                        if message == CONFLICT_MESSAGE {
                            let message = t!("Notes.markdown_external_change").to_string();
                            let weak = weak.clone();
                            let _ = cx.update_window(window_handle, move |_, window, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    if let Some(session) =
                                        view.markdown_sessions.get_mut(&document_id)
                                    {
                                        session.state.conflict();
                                    }
                                    cx.notify();
                                });
                                window.push_notification(
                                    Notification::warning(message).autohide(false),
                                    cx,
                                );
                            });
                        } else {
                            let message =
                                t!("Notes.markdown_save_failed", error = message).to_string();
                            let _ = cx.update_window(window_handle, |_, window, cx| {
                                window.push_notification(
                                    Notification::error(message).autohide(false),
                                    cx,
                                );
                            });
                        }
                    }
                    EditorEvent::AiModelChanged { model } => {
                        let _ = cx.update(|cx| {
                            AppSettings::update_and_save(cx, |settings| {
                                settings.ai_chat.notes_model_id = Some(model.id.clone());
                            });
                        });
                    }
                    _ => {}
                }
            }
        })
        .detach();
    }
}
