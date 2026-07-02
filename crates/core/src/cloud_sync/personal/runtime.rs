use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::settings::{PersonalSyncBackendKind, PersonalSyncSettings};

use super::{PersonalSyncEvent, SyncPackageLayout, SyncStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncRuntimeConfig {
    pub backend: PersonalSyncBackendKind,
    pub root: PathBuf,
    pub auto_sync: bool,
    pub git_auto_push: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonalSyncRuntimeError {
    Disabled,
    NotConfigured,
}

pub fn build_personal_sync_runtime_config(
    settings: &PersonalSyncSettings,
) -> Result<PersonalSyncRuntimeConfig, PersonalSyncRuntimeError> {
    let path = settings.path.trim();
    if path.is_empty() {
        return Err(PersonalSyncRuntimeError::NotConfigured);
    }

    Ok(PersonalSyncRuntimeConfig {
        backend: settings.backend,
        root: PathBuf::from(path),
        auto_sync: settings.auto_sync,
        git_auto_push: settings.git.auto_push,
    })
}

#[derive(Debug, Clone)]
pub struct SelfWriteGuard {
    window: Duration,
    written_at: HashMap<PathBuf, Instant>,
}

impl SelfWriteGuard {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            written_at: HashMap::new(),
        }
    }

    pub fn mark_written(&mut self, path: PathBuf, now: Instant) {
        self.written_at.insert(path, now);
    }

    pub fn should_ignore(&mut self, path: &Path, now: Instant) -> bool {
        self.prune_expired(now);
        self.written_at
            .get(path)
            .is_some_and(|written| now.duration_since(*written) <= self.window)
    }

    fn prune_expired(&mut self, now: Instant) {
        let window = self.window;
        self.written_at
            .retain(|_, written| now.duration_since(*written) <= window);
    }
}

pub struct PersonalSyncWatcher {
    _watcher: RecommendedWatcher,
    guard: Arc<Mutex<SelfWriteGuard>>,
}

impl PersonalSyncWatcher {
    pub fn start(
        root: PathBuf,
        guard_window: Duration,
        on_event: impl Fn(PersonalSyncEvent) + Send + Sync + 'static,
    ) -> Result<Self, SyncStoreError> {
        let layout = SyncPackageLayout::new(root);
        let guard = Arc::new(Mutex::new(SelfWriteGuard::new(guard_window)));
        let callback_guard = Arc::clone(&guard);
        let callback = Arc::new(on_event);
        let mut watcher = notify::recommended_watcher(move |event| {
            handle_watch_event(event, &callback_guard, callback.as_ref());
        })
        .map_err(|error| SyncStoreError::Io(error.to_string()))?;

        watch_if_exists(&mut watcher, &layout.records_dir())?;
        watch_if_exists(&mut watcher, &layout.tombstones_dir())?;
        Ok(Self {
            _watcher: watcher,
            guard,
        })
    }

    pub fn mark_written(&self, path: PathBuf, now: Instant) -> Result<(), SyncStoreError> {
        self.guard
            .lock()
            .map_err(|_| SyncStoreError::Io("watch guard lock poisoned".to_string()))?
            .mark_written(path, now);
        Ok(())
    }
}

fn handle_watch_event(
    event: notify::Result<Event>,
    guard: &Arc<Mutex<SelfWriteGuard>>,
    on_event: &(dyn Fn(PersonalSyncEvent) + Send + Sync),
) {
    let Ok(event) = event else {
        return;
    };
    if event.paths.is_empty() {
        return;
    }
    if all_paths_ignored(&event.paths, guard) {
        return;
    }
    on_event(PersonalSyncEvent::RemoteChanged);
}

fn all_paths_ignored(paths: &[PathBuf], guard: &Arc<Mutex<SelfWriteGuard>>) -> bool {
    let Ok(mut guard) = guard.lock() else {
        return false;
    };
    let now = Instant::now();
    paths.iter().all(|path| guard.should_ignore(path, now))
}

fn watch_if_exists(watcher: &mut RecommendedWatcher, path: &Path) -> Result<(), SyncStoreError> {
    if path.exists() {
        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|error| SyncStoreError::Io(error.to_string()))?;
    }
    Ok(())
}
