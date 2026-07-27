use crate::{MarkdownSaveMode, MarkdownViewMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownSyncState {
    Clean,
    SourceDirty,
    SavingSource,
    Switching,
    Conflict,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownSessionState {
    pub mode: MarkdownViewMode,
    pub source_revision: u64,
    pub projected_revision: u64,
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
    pub source_editor: Entity<InputState>,
    pub preview: Entity<markdown_editor::MarkdownEditor>,
    pub source_document: Arc<std::sync::Mutex<markdown_source::SourceMarkdownDocument>>,
    pub save_generation: Arc<AtomicU64>,
    pub state: MarkdownSessionState,
    pub _subscriptions: Vec<Subscription>,
    pub _file_watcher: Option<notify::RecommendedWatcher>,
}

impl Default for MarkdownSessionState {
    fn default() -> Self {
        Self {
            mode: MarkdownViewMode::Wysiwyg,
            source_revision: 0,
            projected_revision: 0,
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
    pub(crate) fn with_mode(mode: MarkdownViewMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    pub(crate) fn source_changed(&mut self) -> u64 {
        self.source_revision = self.source_revision.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
        self.sync_state = MarkdownSyncState::SourceDirty;
        self.generation
    }

    pub(crate) fn begin_source_save(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.sync_state != MarkdownSyncState::SourceDirty {
            return false;
        }
        self.auto_save_scheduled = false;
        self.saving_generation = Some(generation);
        self.sync_state = MarkdownSyncState::SavingSource;
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
    pub(crate) fn begin_scheduled_source_save(&mut self, epoch: u64) -> Option<u64> {
        if !self.auto_save_scheduled || epoch != self.save_schedule_epoch {
            return None;
        }
        self.auto_save_scheduled = false;
        self.begin_exclusive_source_save()
    }

    /// Immediately save the latest dirty generation. Any pending automatic
    /// timer is invalidated so it cannot perform a duplicate write later.
    pub(crate) fn begin_manual_source_save(&mut self) -> Option<u64> {
        self.cancel_scheduled_automatic_save();
        self.begin_exclusive_source_save()
    }

    pub(crate) fn source_saved(&mut self, generation: u64) {
        if self.saving_generation != Some(generation) {
            return;
        }
        self.saving_generation = None;
        if generation == self.generation {
            self.persisted_revision = self.source_revision;
            self.sync_state = MarkdownSyncState::Clean;
        } else if !matches!(self.sync_state, MarkdownSyncState::Conflict) {
            self.sync_state = MarkdownSyncState::SourceDirty;
        }
    }

    pub(crate) fn source_save_failed(&mut self, generation: u64, message: String) {
        if self.saving_generation == Some(generation) {
            self.saving_generation = None;
        } else if self.saving_generation.is_some() {
            return;
        }
        if generation == self.generation {
            self.sync_state = MarkdownSyncState::Failed(message);
        }
    }

    pub(crate) fn begin_switch(&mut self) -> bool {
        if matches!(
            self.sync_state,
            MarkdownSyncState::SavingSource
                | MarkdownSyncState::Switching
                | MarkdownSyncState::Conflict
                | MarkdownSyncState::Failed(_)
        ) {
            return false;
        }
        self.sync_state = MarkdownSyncState::Switching;
        true
    }

    pub(crate) fn switch_to_wysiwyg(&mut self) {
        self.mode = MarkdownViewMode::Wysiwyg;
        self.projected_revision = self.source_revision;
        self.sync_state = MarkdownSyncState::Clean;
    }

    pub(crate) fn switch_to_source(&mut self) {
        self.mode = MarkdownViewMode::Source;
        self.sync_state = MarkdownSyncState::Clean;
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
        self.persisted_revision = self.source_revision;
        self.sync_state = MarkdownSyncState::Clean;
    }

    /// Reset the session after the document was reloaded from disk,
    /// discarding local changes.
    pub(crate) fn external_reloaded(&mut self) {
        let mode = self.mode;
        *self = MarkdownSessionState {
            mode,
            ..MarkdownSessionState::default()
        };
    }

    fn schedule_automatic_save(&mut self) -> Option<u64> {
        if self.auto_save_scheduled
            || self.saving_generation.is_some()
            || self.sync_state != MarkdownSyncState::SourceDirty
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

    fn begin_exclusive_source_save(&mut self) -> Option<u64> {
        if self.saving_generation.is_some()
            || !matches!(
                self.sync_state,
                MarkdownSyncState::SourceDirty | MarkdownSyncState::Failed(_)
            )
        {
            return None;
        }
        let generation = self.generation;
        self.saving_generation = Some(generation);
        self.sync_state = MarkdownSyncState::SavingSource;
        Some(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarkdownSaveMode;

    #[test]
    fn automatic_save_schedules_only_one_timer_and_uses_latest_generation() {
        let mut state = MarkdownSessionState::default();
        let first = state.source_changed();
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .expect("first edit should schedule one auto-save timer");

        let second = state.source_changed();
        assert_eq!(
            None,
            state.save_mode_changed(MarkdownSaveMode::Automatic),
            "typing again in the same throttle window must not schedule another timer"
        );
        assert_ne!(first, second);
        assert_eq!(
            Some(second),
            state.begin_scheduled_source_save(epoch),
            "the timer must save the latest generation, not the generation that scheduled it"
        );
    }

    #[test]
    fn stale_source_save_does_not_clean_newer_edit() {
        let mut state = MarkdownSessionState::default();
        state.source_changed();
        let generation = state
            .begin_manual_source_save()
            .expect("dirty source should start saving");
        let second = state.source_changed();
        state.source_saved(generation);

        assert_eq!(second, state.generation);
        assert_eq!(MarkdownSyncState::SourceDirty, state.sync_state);
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
        state.source_changed();

        assert_eq!(None, state.save_mode_changed(MarkdownSaveMode::Manual));
        assert_eq!(MarkdownSyncState::SourceDirty, state.sync_state);
        assert!(state.begin_manual_source_save().is_some());
    }

    #[test]
    fn manual_save_cancels_a_pending_timer_and_saves_latest_generation() {
        let mut state = MarkdownSessionState::default();
        state.source_changed();
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .unwrap();
        let latest = state.source_changed();

        assert_eq!(Some(latest), state.begin_manual_source_save());
        assert_eq!(None, state.begin_scheduled_source_save(epoch));
    }

    #[test]
    fn switching_to_manual_mode_invalidates_pending_automatic_save() {
        let mut state = MarkdownSessionState::default();
        state.source_changed();
        let epoch = state
            .save_mode_changed(MarkdownSaveMode::Automatic)
            .unwrap();

        assert_eq!(None, state.save_mode_changed(MarkdownSaveMode::Manual));
        assert_eq!(None, state.begin_scheduled_source_save(epoch));
        assert_eq!(MarkdownSyncState::SourceDirty, state.sync_state);
    }

    #[test]
    fn edit_during_save_never_starts_a_concurrent_save() {
        let mut state = MarkdownSessionState::default();
        state.source_changed();
        let saving = state.begin_manual_source_save().unwrap();
        state.source_changed();

        assert_eq!(
            None,
            state.save_mode_changed(MarkdownSaveMode::Automatic),
            "an in-flight write must finish before another timer is scheduled"
        );
        assert_eq!(None, state.begin_manual_source_save());

        state.source_saved(saving);
        assert!(
            state
                .save_mode_changed(MarkdownSaveMode::Automatic)
                .is_some()
        );
    }

    #[test]
    fn switching_projects_latest_source_revision() {
        let mut state = MarkdownSessionState::default();
        state.mode = MarkdownViewMode::Source;
        state.source_changed();
        assert!(state.begin_switch());
        state.switch_to_wysiwyg();
        assert_eq!(state.source_revision, state.projected_revision);
        assert_eq!(MarkdownViewMode::Wysiwyg, state.mode);
    }

    #[test]
    fn failed_save_blocks_mode_switch() {
        let mut state = MarkdownSessionState::default();
        state.mode = MarkdownViewMode::Source;
        let generation = state.source_changed();
        assert!(state.begin_source_save(generation));
        state.source_save_failed(generation, "disk full".to_owned());
        assert!(!state.begin_switch());
        assert_eq!(MarkdownViewMode::Source, state.mode);
    }

    #[test]
    fn default_mode_is_wysiwyg() {
        assert_eq!(
            MarkdownViewMode::Wysiwyg,
            MarkdownSessionState::default().mode
        );
    }
}
use crate::markdown_file_store::MarkdownFileStore;
use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
