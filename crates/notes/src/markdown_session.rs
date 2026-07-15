use crate::MarkdownViewMode;

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
}

pub(crate) struct MarkdownSession {
    pub relative_path: PathBuf,
    pub store: MarkdownFileStore,
    pub source_editor: Entity<InputState>,
    pub preview: EditorHandle,
    pub fingerprint: Option<FileFingerprint>,
    pub save_generation: Arc<AtomicU64>,
    pub state: MarkdownSessionState,
    pub _subscription: Subscription,
}

impl Default for MarkdownSessionState {
    fn default() -> Self {
        Self {
            mode: MarkdownViewMode::Source,
            source_revision: 0,
            projected_revision: 0,
            persisted_revision: 0,
            generation: 0,
            sync_state: MarkdownSyncState::Clean,
        }
    }
}

impl MarkdownSessionState {
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
        self.sync_state = MarkdownSyncState::SavingSource;
        true
    }

    pub(crate) fn source_saved(&mut self, generation: u64) {
        if generation != self.generation {
            return;
        }
        self.persisted_revision = self.source_revision;
        self.sync_state = MarkdownSyncState::Clean;
    }

    pub(crate) fn source_save_failed(&mut self, generation: u64, message: String) {
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
        self.sync_state = MarkdownSyncState::Conflict;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_source_save_does_not_clean_newer_edit() {
        let mut state = MarkdownSessionState::default();
        let first = state.source_changed();
        assert!(state.begin_source_save(first));
        let second = state.source_changed();
        state.source_saved(first);
        assert_eq!(second, state.generation);
        assert_eq!(MarkdownSyncState::SourceDirty, state.sync_state);
    }

    #[test]
    fn switching_projects_latest_source_revision() {
        let mut state = MarkdownSessionState::default();
        state.source_changed();
        assert!(state.begin_switch());
        state.switch_to_wysiwyg();
        assert_eq!(state.source_revision, state.projected_revision);
        assert_eq!(MarkdownViewMode::Wysiwyg, state.mode);
    }

    #[test]
    fn failed_save_blocks_mode_switch() {
        let mut state = MarkdownSessionState::default();
        let generation = state.source_changed();
        assert!(state.begin_source_save(generation));
        state.source_save_failed(generation, "disk full".to_owned());
        assert!(!state.begin_switch());
        assert_eq!(MarkdownViewMode::Source, state.mode);
    }
}
use crate::markdown_file_store::{FileFingerprint, MarkdownFileStore};
use cditor_app::EditorHandle;
use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
