use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use std::fmt;

use crate::settings::{PersonalSyncBackendKind, PersonalSyncSettings};

use super::{PersonalSyncEvent, SyncPackageLayout, SyncStoreError};

/// WebDAV 后端在运行时真正使用的连接参数。
///
/// `password` 是解密后的明文，只存在于内存中；为了不让它在日志里出现，
/// 这个结构体手写了 `Debug`。
#[derive(Clone, PartialEq, Eq)]
pub struct PersonalWebDavRuntimeSettings {
    pub url: String,
    pub username: String,
    password: String,
}

impl PersonalWebDavRuntimeSettings {
    pub fn new(url: String, username: String, password: String) -> Self {
        Self {
            url,
            username,
            password,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }
}

impl fmt::Debug for PersonalWebDavRuntimeSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonalWebDavRuntimeSettings")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncRuntimeConfig {
    pub backend: PersonalSyncBackendKind,
    pub root: PathBuf,
    pub auto_sync: bool,
    pub git_auto_push: bool,
    /// WebDAV 后端的连接参数；其他后端为 `None`。
    pub webdav: Option<PersonalWebDavRuntimeSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonalSyncRuntimeError {
    Disabled,
    NotConfigured,
}

pub fn build_personal_sync_runtime_config(
    settings: &PersonalSyncSettings,
    webdav_password: Option<&str>,
) -> Result<PersonalSyncRuntimeConfig, PersonalSyncRuntimeError> {
    let common = |root: Option<PathBuf>, webdav: Option<PersonalWebDavRuntimeSettings>| PersonalSyncRuntimeConfig {
        backend: settings.backend,
        root: root.unwrap_or_default(),
        auto_sync: settings.auto_sync,
        git_auto_push: settings.git.auto_push,
        webdav,
    };

    if settings.backend == PersonalSyncBackendKind::Webdav {
        let webdav = &settings.webdav;
        if !webdav.is_complete() {
            return Err(PersonalSyncRuntimeError::NotConfigured);
        }
        let Some(password) = webdav_password.map(str::trim).filter(|value| !value.is_empty())
        else {
            // 配置里存着密文，但没有可用主密钥去解开它。
            return Err(PersonalSyncRuntimeError::NotConfigured);
        };
        return Ok(common(None, Some(PersonalWebDavRuntimeSettings::new(
            webdav.url.trim().to_string(),
            webdav.username.trim().to_string(),
            password.to_string(),
        ))));
    }

    let path = settings.path.trim();
    if path.is_empty() {
        return Err(PersonalSyncRuntimeError::NotConfigured);
    }

    Ok(common(Some(PathBuf::from(path)), None))
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

        fs::create_dir_all(layout.records_dir())?;
        fs::create_dir_all(layout.tombstones_dir())?;
        watch_required(&mut watcher, &layout.records_dir())?;
        watch_required(&mut watcher, &layout.tombstones_dir())?;
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

fn watch_required(watcher: &mut RecommendedWatcher, path: &Path) -> Result<(), SyncStoreError> {
    watcher
        .watch(path, RecursiveMode::Recursive)
        .map_err(|error| SyncStoreError::Io(error.to_string()))
}
