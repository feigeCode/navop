use crate::NotesView;
use crate::markdown_session::MarkdownSyncState;
use crate::notes_notifications::notify_operation_error;
use cditor_app::{EditorHandle, EditorSaveState};
use futures::channel::oneshot;
use gpui::{App, Context, IntoElement, ParentElement, SharedString, Task, Window};
use gpui_component::{
    Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants},
};
use one_core::tab_container::TabContent;
use rust_i18n::t;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CLOSE_POLL_ATTEMPTS: usize = 100;

impl TabContent for NotesView {
    fn content_key(&self) -> &'static str {
        "Notes"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.notebook_name.clone()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        let icon = if self.standalone_markdown {
            IconName::MarkdownColor
        } else {
            IconName::NotesColor
        };
        Some(icon.color().with_size(Size::Medium))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let dirty = dirty_editors(self, cx);
        if markdown_close_blocked(self) {
            return confirm_blocked_close(cx.entity().clone(), window, cx);
        }
        if dirty.is_empty() && !self.markdown_has_blocking_state() {
            return Task::ready(true);
        }
        if let Err(error) = save_rich_editors(&dirty, cx) {
            notify_operation_error(window, cx, error);
            return Task::ready(false);
        }
        poll_until_saved(dirty, cx)
    }
}

fn dirty_editors(view: &NotesView, cx: &App) -> Vec<EditorHandle> {
    view.editors
        .values()
        .filter(|cached| cached.handle.is_dirty(cx))
        .map(|cached| cached.handle.clone())
        .chain(
            view.markdown_sessions
                .values()
                .filter(|session| session.preview.is_dirty(cx))
                .map(|session| session.preview.clone()),
        )
        .collect()
}

fn markdown_close_blocked(view: &NotesView) -> bool {
    view.markdown_sessions.values().any(|session| {
        matches!(
            session.state.sync_state,
            MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)
        )
    })
}

/// Ask how to resolve unsaved (conflicted or failed) Markdown changes:
/// keep local changes (overwrite the external file), discard them, or cancel.
fn confirm_blocked_close(
    view: gpui::Entity<NotesView>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Task<bool> {
    let (tx, rx) = oneshot::channel::<bool>();
    let tx = Arc::new(Mutex::new(Some(tx)));

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let tx_cancel = tx.clone();
        let tx_discard = tx.clone();
        let tx_keep = tx.clone();
        let view_discard = view.clone();
        let view_keep = view.clone();

        dialog
            .title(t!("Notes.unsaved_markdown_close_title").to_string())
            .overlay_closable(false)
            .close_button(false)
            .footer(move |_ok, _cancel, _window, _cx| {
                let tx_cancel = tx_cancel.clone();
                let tx_discard = tx_discard.clone();
                let tx_keep = tx_keep.clone();
                let view_discard = view_discard.clone();
                let view_keep = view_keep.clone();

                vec![
                    Button::new("cancel")
                        .label(t!("Notes.markdown_conflict_cancel").to_string())
                        .on_click(move |_, window: &mut Window, cx| {
                            window.close_dialog(cx);
                            if let Some(tx) = tx_cancel.lock().ok().and_then(|mut g| g.take()) {
                                let _ = tx.send(false);
                            }
                        })
                        .into_any_element(),
                    Button::new("discard")
                        .label(t!("Notes.markdown_conflict_discard").to_string())
                        .on_click(move |_, window: &mut Window, cx| {
                            window.close_dialog(cx);
                            view_discard.update(cx, |view, cx| {
                                view.discard_blocked_markdown_sessions(cx);
                            });
                            if let Some(tx) = tx_discard.lock().ok().and_then(|mut g| g.take()) {
                                let _ = tx.send(true);
                            }
                        })
                        .into_any_element(),
                    Button::new("keep-local")
                        .label(t!("Notes.markdown_conflict_keep_local").to_string())
                        .primary()
                        .on_click(move |_, window: &mut Window, cx| {
                            let still_blocked = view_keep.update(cx, |view, cx| {
                                for id in view.blocked_markdown_document_ids() {
                                    view.resolve_markdown_conflict_keep_local(&id, window, cx);
                                }
                                view.markdown_sessions.values().any(|session| {
                                    matches!(
                                        session.state.sync_state,
                                        MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)
                                    )
                                })
                            });
                            if still_blocked {
                                // Save failed again; keep the dialog open result as cancel.
                                window.close_dialog(cx);
                                if let Some(tx) = tx_keep.lock().ok().and_then(|mut g| g.take()) {
                                    let _ = tx.send(false);
                                }
                            } else {
                                window.close_dialog(cx);
                                if let Some(tx) = tx_keep.lock().ok().and_then(|mut g| g.take()) {
                                    let _ = tx.send(true);
                                }
                            }
                        })
                        .into_any_element(),
                ]
            })
            .child(t!("Notes.unsaved_markdown_close_message").to_string())
    });

    cx.spawn(async move |_handle, _cx| rx.await.unwrap_or(false))
}

fn save_rich_editors(
    dirty: &[EditorHandle],
    cx: &mut Context<NotesView>,
) -> Result<(), cditor_app::EditorError> {
    for handle in dirty {
        handle.save(cx)?;
    }
    Ok(())
}

fn poll_until_saved(dirty: Vec<EditorHandle>, cx: &mut Context<NotesView>) -> Task<bool> {
    let executor = cx.background_executor().clone();
    cx.spawn(async move |view, cx| {
        for _ in 0..CLOSE_POLL_ATTEMPTS {
            executor.timer(CLOSE_POLL_INTERVAL).await;
            let Ok((rich, markdown_clean)) =
                view.update(cx, |view, cx| close_states(view, &dirty, cx))
            else {
                return false;
            };
            if rich.iter().any(rich_save_failed) {
                return false;
            }
            if markdown_clean && rich.iter().all(rich_save_clean) {
                return true;
            }
        }
        false
    })
}

fn close_states(
    view: &NotesView,
    dirty: &[EditorHandle],
    cx: &App,
) -> (Vec<EditorSaveState>, bool) {
    let rich = dirty.iter().map(|handle| handle.save_state(cx)).collect();
    let markdown_clean = view
        .markdown_sessions
        .values()
        .all(|session| session.state.sync_state == MarkdownSyncState::Clean);
    (rich, markdown_clean)
}

fn rich_save_failed(state: &EditorSaveState) -> bool {
    matches!(state, EditorSaveState::SaveFailed { .. })
}

fn rich_save_clean(state: &EditorSaveState) -> bool {
    matches!(state, EditorSaveState::Clean | EditorSaveState::Disabled)
}
