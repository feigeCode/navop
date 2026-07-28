use crate::NotesView;
use crate::markdown_session::{MarkdownSessionState, MarkdownSyncState};
use futures::channel::oneshot;
use gpui::{App, AppContext, Context, IntoElement, ParentElement, SharedString, Task, Window};
use gpui_component::{
    Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants},
};
use one_core::tab_container::TabContent;
use rust_i18n::t;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

const CLOSE_SAVE_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkdownClosePrompt {
    Unsaved,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkdownCloseChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseSaveProgress {
    Complete,
    Waiting,
    Failed,
}

type CloseChoiceSender = Arc<Mutex<Option<oneshot::Sender<MarkdownCloseChoice>>>>;

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
        self.prepare_close(window, cx)
    }
}

impl NotesView {
    pub fn has_unsaved_changes(&self, cx: &App) -> bool {
        let _ = cx;
        self.markdown_has_blocking_state()
    }

    pub fn prepare_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let Some(prompt) = markdown_close_prompt(
            self.markdown_sessions
                .values()
                .map(|session| &session.state),
        ) else {
            return Task::ready(true);
        };
        let initially_blocked = self
            .markdown_sessions
            .iter()
            .filter(|(_, session)| {
                matches!(
                    session.state.sync_state,
                    MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        confirm_markdown_close(cx.entity().clone(), prompt, initially_blocked, window, cx)
    }

    fn advance_close_save(
        &mut self,
        initially_blocked: &HashSet<String>,
        attempted_generations: &mut HashMap<String, u64>,
        force_attempted: &mut HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseSaveProgress {
        // This should already be guaranteed by MarkdownSessionState, but repair
        // a stale clean flag rather than allowing a revision that never reached
        // disk to close silently.
        for session in self.markdown_sessions.values_mut() {
            if matches!(session.state.sync_state, MarkdownSyncState::Clean)
                && session.state.source_revision != session.state.persisted_revision
            {
                session.state.sync_state = MarkdownSyncState::SourceDirty;
            }
        }

        let pending = self
            .markdown_sessions
            .iter()
            .filter(|(_, session)| session.state.has_unpersisted_changes())
            .map(|(id, session)| {
                (
                    id.clone(),
                    session.state.sync_state.clone(),
                    session.state.generation,
                )
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return CloseSaveProgress::Complete;
        }

        let mut waiting = false;
        for (document_id, sync_state, generation) in pending {
            match sync_state {
                MarkdownSyncState::Clean => {
                    // A mismatched clean state was normalized above, so a
                    // genuinely clean session cannot be pending.
                    return CloseSaveProgress::Failed;
                }
                MarkdownSyncState::SourceDirty => {
                    if attempted_generations.get(&document_id) == Some(&generation) {
                        return CloseSaveProgress::Failed;
                    }
                    self.save_markdown_document(&document_id, window, cx);
                    let Some(session) = self.markdown_sessions.get(&document_id) else {
                        continue;
                    };
                    if matches!(session.state.sync_state, MarkdownSyncState::SavingSource) {
                        attempted_generations.insert(document_id, generation);
                    }
                    // If an older generation is already being written,
                    // begin_manual_source_save intentionally does nothing. Wait
                    // for it to finish, then save this newer generation.
                    waiting = true;
                }
                MarkdownSyncState::SavingSource => {
                    attempted_generations
                        .entry(document_id)
                        .or_insert(generation);
                    waiting = true;
                }
                MarkdownSyncState::Failed(_) => {
                    if initially_blocked.contains(&document_id) {
                        if !force_attempted.insert(document_id.clone()) {
                            return CloseSaveProgress::Failed;
                        }
                        self.resolve_markdown_conflict_keep_local(&document_id, window, cx);
                        if self
                            .markdown_sessions
                            .get(&document_id)
                            .is_some_and(|session| session.state.has_unpersisted_changes())
                        {
                            return CloseSaveProgress::Failed;
                        }
                    } else {
                        // A save may have failed while the ordinary unsaved
                        // prompt was open. Retry it once without overwriting a
                        // potentially changed external file.
                        if attempted_generations.get(&document_id) == Some(&generation) {
                            return CloseSaveProgress::Failed;
                        }
                        self.save_markdown_document(&document_id, window, cx);
                        let Some(session) = self.markdown_sessions.get(&document_id) else {
                            continue;
                        };
                        if !matches!(session.state.sync_state, MarkdownSyncState::SavingSource) {
                            return CloseSaveProgress::Failed;
                        }
                        attempted_generations.insert(document_id, generation);
                        waiting = true;
                    }
                }
                MarkdownSyncState::Conflict => {
                    // Only overwrite an external version when the dialog the
                    // user accepted explicitly described that behavior. A new
                    // conflict discovered during an ordinary close-save keeps
                    // the tab open for an explicit resolution.
                    if !initially_blocked.contains(&document_id)
                        || !force_attempted.insert(document_id.clone())
                    {
                        return CloseSaveProgress::Failed;
                    }
                    self.resolve_markdown_conflict_keep_local(&document_id, window, cx);
                    if self
                        .markdown_sessions
                        .get(&document_id)
                        .is_some_and(|session| session.state.has_unpersisted_changes())
                    {
                        return CloseSaveProgress::Failed;
                    }
                }
                MarkdownSyncState::Switching => {
                    // Mode switching is synchronous. If this state survives
                    // until try_close runs, closing would risk losing source.
                    return CloseSaveProgress::Failed;
                }
            }
        }

        if !self.markdown_has_blocking_state() {
            CloseSaveProgress::Complete
        } else if waiting {
            CloseSaveProgress::Waiting
        } else {
            CloseSaveProgress::Failed
        }
    }
}

fn markdown_close_prompt<'a>(
    states: impl IntoIterator<Item = &'a MarkdownSessionState>,
) -> Option<MarkdownClosePrompt> {
    let mut has_unsaved = false;
    let mut has_blocked = false;
    for state in states {
        if !state.has_unpersisted_changes() {
            continue;
        }
        has_unsaved = true;
        has_blocked |= matches!(
            state.sync_state,
            MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)
        );
    }
    if has_blocked {
        Some(MarkdownClosePrompt::Blocked)
    } else if has_unsaved {
        Some(MarkdownClosePrompt::Unsaved)
    } else {
        None
    }
}

fn confirm_markdown_close(
    view: gpui::Entity<NotesView>,
    prompt: MarkdownClosePrompt,
    initially_blocked: HashSet<String>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Task<bool> {
    let (tx, rx) = oneshot::channel::<MarkdownCloseChoice>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let title = match prompt {
        MarkdownClosePrompt::Unsaved => t!("Notes.unsaved_markdown_changes_title").to_string(),
        MarkdownClosePrompt::Blocked => t!("Notes.unsaved_markdown_close_title").to_string(),
    };
    let message = match prompt {
        MarkdownClosePrompt::Unsaved => t!("Notes.unsaved_markdown_changes_message").to_string(),
        MarkdownClosePrompt::Blocked => t!("Notes.unsaved_markdown_close_message").to_string(),
    };
    let save_label = match prompt {
        MarkdownClosePrompt::Unsaved => t!("Notes.markdown_save_and_close").to_string(),
        MarkdownClosePrompt::Blocked => t!("Notes.markdown_conflict_keep_local").to_string(),
    };
    let discard_label = match prompt {
        MarkdownClosePrompt::Unsaved => t!("Notes.markdown_discard_and_close").to_string(),
        MarkdownClosePrompt::Blocked => t!("Notes.markdown_conflict_discard").to_string(),
    };

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let tx_cancel = tx.clone();
        let tx_discard = tx.clone();
        let tx_save = tx.clone();
        let save_label = save_label.clone();
        let discard_label = discard_label.clone();

        dialog
            .title(title.clone())
            .overlay_closable(false)
            .close_button(false)
            .footer(move |_ok, _cancel, _window, _cx| {
                let tx_cancel = tx_cancel.clone();
                let tx_discard = tx_discard.clone();
                let tx_save = tx_save.clone();

                vec![
                    Button::new("notes-close-cancel")
                        .label(t!("Notes.markdown_conflict_cancel").to_string())
                        .on_click(move |_, window: &mut Window, cx| {
                            window.close_dialog(cx);
                            send_close_choice(&tx_cancel, MarkdownCloseChoice::Cancel);
                        })
                        .into_any_element(),
                    Button::new("notes-close-discard")
                        .label(discard_label.clone())
                        .on_click(move |_, window: &mut Window, cx| {
                            window.close_dialog(cx);
                            send_close_choice(&tx_discard, MarkdownCloseChoice::Discard);
                        })
                        .into_any_element(),
                    Button::new("notes-close-save")
                        .label(save_label.clone())
                        .primary()
                        .on_click(move |_, window: &mut Window, cx| {
                            window.close_dialog(cx);
                            send_close_choice(&tx_save, MarkdownCloseChoice::Save);
                        })
                        .into_any_element(),
                ]
            })
            .child(message.clone())
    });

    let window_handle = window.window_handle();
    cx.spawn(
        async move |_handle, cx| match rx.await.unwrap_or(MarkdownCloseChoice::Cancel) {
            MarkdownCloseChoice::Cancel => false,
            MarkdownCloseChoice::Discard => true,
            MarkdownCloseChoice::Save => {
                let mut attempted_generations = HashMap::new();
                let mut force_attempted = HashSet::new();
                loop {
                    let progress = cx.update_window(window_handle, |_, window, cx| {
                        view.update(cx, |view, cx| {
                            view.advance_close_save(
                                &initially_blocked,
                                &mut attempted_generations,
                                &mut force_attempted,
                                window,
                                cx,
                            )
                        })
                    });
                    match progress {
                        Ok(CloseSaveProgress::Complete) => return true,
                        Ok(CloseSaveProgress::Failed) | Err(_) => return false,
                        Ok(CloseSaveProgress::Waiting) => {
                            cx.background_executor()
                                .timer(CLOSE_SAVE_POLL_INTERVAL)
                                .await;
                        }
                    }
                }
            }
        },
    )
}

fn send_close_choice(sender: &CloseChoiceSender, choice: MarkdownCloseChoice) {
    if let Ok(mut sender) = sender.lock()
        && let Some(sender) = sender.take()
    {
        let _ = sender.send(choice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_markdown_does_not_require_a_close_prompt() {
        let state = MarkdownSessionState::default();

        assert_eq!(None, markdown_close_prompt([&state]));
    }

    #[test]
    fn dirty_and_saving_markdown_use_the_unsaved_prompt() {
        let mut dirty = MarkdownSessionState::default();
        dirty.source_changed();
        let mut saving = dirty.clone();
        saving.begin_manual_source_save().unwrap();

        assert_eq!(
            Some(MarkdownClosePrompt::Unsaved),
            markdown_close_prompt([&dirty])
        );
        assert_eq!(
            Some(MarkdownClosePrompt::Unsaved),
            markdown_close_prompt([&saving])
        );
    }

    #[test]
    fn failed_or_conflicted_markdown_uses_the_blocked_prompt() {
        let mut failed = MarkdownSessionState::default();
        failed.sync_state = MarkdownSyncState::Failed("read only".to_owned());
        let mut conflicted = MarkdownSessionState::default();
        conflicted.sync_state = MarkdownSyncState::Conflict;

        assert_eq!(
            Some(MarkdownClosePrompt::Blocked),
            markdown_close_prompt([&failed])
        );
        assert_eq!(
            Some(MarkdownClosePrompt::Blocked),
            markdown_close_prompt([&conflicted])
        );
    }

    #[test]
    fn revision_mismatch_requires_a_prompt_even_with_a_stale_clean_flag() {
        let mut state = MarkdownSessionState::default();
        state.source_revision = 1;

        assert_eq!(
            Some(MarkdownClosePrompt::Unsaved),
            markdown_close_prompt([&state])
        );
    }

    #[test]
    fn a_blocked_document_takes_precedence_over_an_ordinary_dirty_document() {
        let mut dirty = MarkdownSessionState::default();
        dirty.source_changed();
        let mut conflicted = MarkdownSessionState::default();
        conflicted.sync_state = MarkdownSyncState::Conflict;

        assert_eq!(
            Some(MarkdownClosePrompt::Blocked),
            markdown_close_prompt([&dirty, &conflicted])
        );
    }
}
