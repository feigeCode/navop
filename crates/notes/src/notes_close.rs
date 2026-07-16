use crate::NotesView;
use crate::markdown_session::MarkdownSyncState;
use crate::notes_notifications::{notify_error_message, notify_operation_error};
use cditor_app::{EditorHandle, EditorSaveState};
use gpui::{App, Context, SharedString, Task, Window};
use gpui_component::{Icon, IconName, Sizable, Size};
use one_core::tab_container::TabContent;
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
        Some(IconName::NotesColor.color().with_size(Size::Medium))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let dirty = dirty_editors(self, cx);
        if markdown_close_blocked(self) {
            notify_error_message(
                window,
                cx,
                rust_i18n::t!("Notes.unsaved_markdown_close").to_string(),
            );
            return Task::ready(false);
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
