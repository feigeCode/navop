use crate::markdown_file_store::MarkdownFileStore;
use crate::{MarkdownSaveMode, MarkdownViewMode};
use gpui::{Entity, Subscription};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownSyncState {
    Clean,
    Dirty,
    Saving,
    Conflict,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownSessionState {
    pub mode: MarkdownViewMode,
    pub editor_revision: u64,
    pub persisted_revision: u64,
    pub generation: u64,
    pub sync_state: MarkdownSyncState,
    auto_save_scheduled: bool,
    save_schedule_epoch: u64,
    saving_generation: Option<u64>,
}

pub(crate) struct MarkdownSession {
    pub relative_path: PathBuf,
    pub store: MarkdownFileStore,
    /// The single Velotype editor that owns the Markdown document, history,
    /// selection, block tree, rendered/source modes, and focus.
    pub editor: Entity<markdown_editor::MarkdownEditor>,
    /// Split 模式下的只读预览编辑器；镜像主编辑器内容，用户在预览侧的
    /// 瞬时编辑会被主编辑器内容立即覆盖。
    pub preview: Option<MarkdownPreview>,
    pub state: MarkdownSessionState,
    pub _subscriptions: Vec<Subscription>,
    pub _file_watcher: Option<notify::RecommendedWatcher>,
}

/// 只读预览视图及其事件订阅；离开 Split 模式时整体丢弃。
pub(crate) struct MarkdownPreview {
    pub editor: Entity<markdown_editor::MarkdownEditor>,
    pub _subscription: Subscription,
}

impl Default for MarkdownSessionState {
    fn default() -> Self {
        Self {
            mode: MarkdownViewMode::Wysiwyg,
            editor_revision: 0,
            persisted_revision: 0,
            generation: 0,
            sync_state: MarkdownSyncState::Clean,
            auto_save_scheduled: false,
            save_schedule_epoch: 0,
            saving_generation: None,
        }
    }
}

impl MarkdownSessionState {
    pub(crate) fn with_mode_and_revision(mode: MarkdownViewMode, editor_revision: u64) -> Self {
        Self {
            mode,
            editor_revision,
            persisted_revision: editor_revision,
            ..Self::default()
        }
    }

    pub(crate) fn has_unpersisted_changes(&self) -> bool {
        self.editor_revision != self.persisted_revision
            || !matches!(self.sync_state, MarkdownSyncState::Clean)
    }

    /// Accept a document event only once. The editor revision is the
    /// authoritative identity of a mutation, so delayed events from an
    /// externally reloaded document cannot dirty the clean replacement.
    pub(crate) fn document_changed(&mut self, revision: u64) -> bool {
        if revision <= self.editor_revision {
            return false;
        }

        self.editor_revision = revision;
        self.generation = self.generation.saturating_add(1);
        self.sync_state = MarkdownSyncState::Dirty;
        true
    }

    /// Apply a user save-mode choice and, for automatic mode, return the epoch
    /// for the single timer that should cover the current throttle window.
    pub(crate) fn save_mode_changed(&mut self, mode: MarkdownSaveMode) -> Option<u64> {
        match mode {
            MarkdownSaveMode::Automatic => self.schedule_automatic_save(),
            MarkdownSaveMode::Manual => {
                self.cancel_scheduled_automatic_save();
                None
            }
        }
    }

    /// Consume a scheduled throttle window and begin writing the latest
    /// generation. Repeated edits do not create another timer, so the
    /// generation is intentionally read here rather than when it was queued.
    pub(crate) fn begin_scheduled_save(&mut self, epoch: u64) -> Option<u64> {
        if !self.auto_save_scheduled || epoch != self.save_schedule_epoch {
            return None;
        }
        self.auto_save_scheduled = false;
        self.begin_exclusive_save()
    }

    /// Immediately save the latest dirty generation. Any pending automatic
    /// timer is invalidated so it cannot perform a duplicate write later.
    pub(crate) fn begin_manual_save(&mut self) -> Option<u64> {
        self.cancel_scheduled_automatic_save();
        self.begin_exclusive_save()
    }

    pub(crate) fn document_saved(&mut self, generation: u64) {
        if self.saving_generation != Some(generation) {
            return;
        }
        self.saving_generation = None;
        if generation == self.generation {
            self.persisted_revision = self.editor_revision;
            self.sync_state = MarkdownSyncState::Clean;
        } else if !matches!(self.sync_state, MarkdownSyncState::Conflict) {
            self.sync_state = MarkdownSyncState::Dirty;
        }
    }

    pub(crate) fn document_save_failed(&mut self, generation: u64, message: String) {
        if self.saving_generation != Some(generation) {
            return;
        }

        self.saving_generation = None;
        if generation == self.generation {
            self.sync_state = MarkdownSyncState::Failed(message);
        } else if !matches!(self.sync_state, MarkdownSyncState::Conflict) {
            self.sync_state = MarkdownSyncState::Dirty;
        }
    }

    pub(crate) fn set_mode(&mut self, mode: MarkdownViewMode) {
        self.mode = mode;
    }

    pub(crate) fn conflict(&mut self) {
        self.cancel_scheduled_automatic_save();
        self.saving_generation = None;
        self.sync_state = MarkdownSyncState::Conflict;
    }

    /// Mark the conflict as resolved by keeping the local changes,
    /// which were just force-written to disk.
    pub(crate) fn conflict_resolved(&mut self) {
        self.cancel_scheduled_automatic_save();
        self.saving_generation = None;
        self.persisted_revision = self.editor_revision;
        self.sync_state = MarkdownSyncState::Clean;
    }

    /// Reset persistence bookkeeping after the host replaced the document from
    /// disk. Generations and timer epochs stay monotonic so an old timer or save
    /// completion can never collide with work scheduled after the reload.
    pub(crate) fn external_reloaded(&mut self, editor_revision: u64) {
        self.cancel_scheduled_automatic_save();
        self.saving_generation = None;
        self.generation = self.generation.saturating_add(1);
        self.editor_revision = editor_revision;
        self.persisted_revision = editor_revision;
        self.sync_state = MarkdownSyncState::Clean;
    }

    fn schedule_automatic_save(&mut self) -> Option<u64> {
        if self.auto_save_scheduled
            || self.saving_generation.is_some()
            || self.sync_state != MarkdownSyncState::Dirty
        {
            return None;
        }
        self.save_schedule_epoch = self.save_schedule_epoch.saturating_add(1);
        self.auto_save_scheduled = true;
        Some(self.save_schedule_epoch)
    }

    fn cancel_scheduled_automatic_save(&mut self) {
        if self.auto_save_scheduled {
            self.auto_save_scheduled = false;
            self.save_schedule_epoch = self.save_schedule_epoch.saturating_add(1);
        }
    }

    fn begin_exclusive_save(&mut self) -> Option<u64> {
        if self.saving_generation.is_some()
            || !matches!(
                self.sync_state,
                MarkdownSyncState::Dirty | MarkdownSyncState::Failed(_)
            )
        {
            return None;
        }
        let generation = self.generation;
        self.saving_generation = Some(generation);
        self.sync_state = MarkdownSyncState::Saving;
        Some(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_save_schedules_only_one_timer_and_uses_latest_generation() {
        let mut state = MarkdownSessionState::default();
        assert!(state.document_changed(1));
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .expect("first edit should schedule one auto-save timer");

        assert!(state.document_changed(2));
        assert_eq!(
            None,
            state.save_mode_changed(MarkdownSaveMode::Automatic),
            "typing again in the same throttle window must not schedule another timer"
        );
        assert_eq!(
            Some(state.generation),
            state.begin_scheduled_save(epoch),
            "the timer must save the latest generation, not the generation that scheduled it"
        );
    }

    #[test]
    fn duplicate_or_delayed_editor_revision_is_ignored() {
        let mut state = MarkdownSessionState::default();

        assert!(state.document_changed(1));
        let generation = state.generation;
        assert!(!state.document_changed(1));
        assert!(!state.document_changed(0));
        assert_eq!(generation, state.generation);
    }

    #[test]
    fn stale_save_does_not_clean_newer_edit() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let generation = state
            .begin_manual_save()
            .expect("dirty document should start saving");
        state.document_changed(2);
        state.document_saved(generation);

        assert_eq!(MarkdownSyncState::Dirty, state.sync_state);
        assert!(
            state
                .save_mode_changed(MarkdownSaveMode::Automatic)
                .is_some(),
            "a newer edit must be eligible for the next throttle window"
        );
    }

    #[test]
    fn manual_mode_never_schedules_an_automatic_save() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);

        assert_eq!(None, state.save_mode_changed(MarkdownSaveMode::Manual));
        assert_eq!(MarkdownSyncState::Dirty, state.sync_state);
        assert!(state.begin_manual_save().is_some());
    }

    #[test]
    fn manual_save_cancels_a_pending_timer_and_saves_latest_generation() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .unwrap();
        state.document_changed(2);
        let latest = state.generation;

        assert_eq!(Some(latest), state.begin_manual_save());
        assert_eq!(None, state.begin_scheduled_save(epoch));
    }

    #[test]
    fn switching_to_manual_mode_invalidates_pending_automatic_save() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .unwrap();

        assert_eq!(None, state.save_mode_changed(MarkdownSaveMode::Manual));
        assert_eq!(None, state.begin_scheduled_save(epoch));
        assert_eq!(MarkdownSyncState::Dirty, state.sync_state);
    }

    #[test]
    fn edit_during_save_never_starts_a_concurrent_save() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let saving = state.begin_manual_save().unwrap();
        state.document_changed(2);

        assert_eq!(
            None,
            state.save_mode_changed(MarkdownSaveMode::Automatic),
            "an in-flight write must finish before another timer is scheduled"
        );
        assert_eq!(None, state.begin_manual_save());

        state.document_saved(saving);
        assert!(
            state
                .save_mode_changed(MarkdownSaveMode::Automatic)
                .is_some()
        );
    }

    #[test]
    fn mode_switch_does_not_change_document_or_persistence_state() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let generation = state.generation;

        state.set_mode(MarkdownViewMode::Source);
        state.set_mode(MarkdownViewMode::Wysiwyg);

        assert_eq!(generation, state.generation);
        assert_eq!(MarkdownSyncState::Dirty, state.sync_state);
        assert!(state.has_unpersisted_changes());
    }

    #[test]
    fn external_reload_invalidates_old_events_timers_and_generations() {
        let mut state = MarkdownSessionState::default();
        state.document_changed(1);
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .unwrap();
        let old_generation = state.generation;

        state.external_reloaded(2);

        assert_eq!(MarkdownSyncState::Clean, state.sync_state);
        assert_eq!(state.editor_revision, state.persisted_revision);
        assert!(!state.has_unpersisted_changes());
        assert_eq!(None, state.begin_scheduled_save(epoch));
        assert!(!state.document_changed(2));
        assert!(state.generation > old_generation);
        assert!(state.document_changed(3));
    }

    #[test]
    fn default_mode_is_wysiwyg() {
        assert_eq!(
            MarkdownViewMode::Wysiwyg,
            MarkdownSessionState::default().mode
        );
    }
}
