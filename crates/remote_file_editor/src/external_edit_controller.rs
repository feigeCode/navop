use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::hash_map::DefaultHasher, hash::Hasher as _};

use anyhow::{Result, anyhow};
use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, Context, Entity, PromptLevel, Task, WeakEntity, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use notify::RecommendedWatcher;
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
use smol::Timer;
use tokio::sync::Mutex;

use crate::external_session::snapshot_from_metadata;
use crate::{
    MAX_EDITABLE_FILE_SIZE, RemoteFileSnapshot, RemoteMutationCallback, UploadDecision,
    decide_upload,
};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(750);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RELOAD_SUPPRESSION: Duration = Duration::from_secs(2);
const MAX_ACTIVE_EXTERNAL_EDIT_SESSIONS: usize = 32;

enum SyncCheck {
    Unchanged,
    Changed {
        current: Option<RemoteFileSnapshot>,
        bytes: Vec<u8>,
        local_hash: u64,
    },
}

struct ConflictPrompt {
    bytes: Vec<u8>,
    local_hash: u64,
    decision: UploadDecision,
}

struct SnapshotCompletion {
    task: gpui::Task<Result<(RemoteFileSnapshot, u64)>>,
    success_message: String,
    remote_changed: bool,
}

pub(crate) struct ExternalEditController {
    client: Arc<Mutex<RusshSftpClient>>,
    remote_path: String,
    local_path: PathBuf,
    snapshot: RemoteFileSnapshot,
    last_local_hash: u64,
    check_conflict: bool,
    syncing: bool,
    pending_sync: bool,
    suppress_until: Option<std::time::Instant>,
    _watcher: RecommendedWatcher,
    on_remote_changed: RemoteMutationCallback,
}

pub(crate) struct ExternalEditControllerConfig {
    pub(crate) client: Arc<Mutex<RusshSftpClient>>,
    pub(crate) remote_path: String,
    pub(crate) local_path: PathBuf,
    pub(crate) snapshot: RemoteFileSnapshot,
    pub(crate) initial_local_hash: u64,
    pub(crate) check_conflict: bool,
    pub(crate) on_remote_changed: RemoteMutationCallback,
}

impl ExternalEditController {
    pub(crate) fn new(config: ExternalEditControllerConfig, watcher: RecommendedWatcher) -> Self {
        Self {
            client: config.client,
            remote_path: config.remote_path,
            local_path: config.local_path,
            snapshot: config.snapshot,
            last_local_hash: config.initial_local_hash,
            check_conflict: config.check_conflict,
            syncing: false,
            pending_sync: false,
            suppress_until: None,
            _watcher: watcher,
            on_remote_changed: config.on_remote_changed,
        }
    }

    fn request_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .suppress_until
            .is_some_and(|deadline| std::time::Instant::now() < deadline)
        {
            return;
        }
        if self.syncing {
            self.pending_sync = true;
            return;
        }
        self.syncing = true;
        let task = self.build_sync_check(cx);
        let entity = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(check) => {
                    let _ = entity.update_in(cx, |this, window, cx| {
                        this.handle_sync_check(check, window, cx);
                    });
                }
                Err(error) => finish_with_error(&entity, error.to_string(), cx),
            })
            .detach();
    }

    fn build_sync_check(&self, cx: &Context<Self>) -> gpui::Task<Result<SyncCheck>> {
        let client = self.client.clone();
        let remote_path = self.remote_path.clone();
        let local_path = self.local_path.clone();
        let last_local_hash = self.last_local_hash;
        Tokio::spawn_result(cx, async move {
            let bytes = tokio::fs::read(local_path).await?;
            let local_hash = local_content_hash(&bytes);
            if local_hash == last_local_hash {
                return Ok(SyncCheck::Unchanged);
            }
            let current = client
                .lock()
                .await
                .stat(&remote_path)
                .await?
                .as_ref()
                .map(snapshot_from_metadata);
            Ok(SyncCheck::Changed {
                current,
                bytes,
                local_hash,
            })
        })
    }

    fn handle_sync_check(&mut self, check: SyncCheck, window: &mut Window, cx: &mut Context<Self>) {
        let SyncCheck::Changed {
            current,
            bytes,
            local_hash,
        } = check
        else {
            self.finish_sync(window, cx);
            return;
        };
        let decision = if self.check_conflict {
            decide_upload(self.snapshot, current)
        } else {
            UploadDecision::Upload
        };
        match decision {
            UploadDecision::Upload => self.upload(bytes, local_hash, window, cx),
            UploadDecision::Conflict | UploadDecision::RemoteMissing => self.prompt_conflict(
                ConflictPrompt {
                    bytes,
                    local_hash,
                    decision,
                },
                window,
                cx,
            ),
        }
    }

    fn prompt_conflict(
        &mut self,
        conflict: ConflictPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = match conflict.decision {
            UploadDecision::Conflict => t!("RemoteFileEditor.prompt.remote_changed").to_string(),
            UploadDecision::RemoteMissing => {
                t!("RemoteFileEditor.prompt.remote_missing").to_string()
            }
            UploadDecision::Upload => return,
        };
        let overwrite = t!("RemoteFileEditor.action.overwrite_remote").to_string();
        let reload = t!("RemoteFileEditor.action.reload_remote").to_string();
        let cancel = t!("RemoteFileEditor.action.cancel").to_string();
        let answer = window.prompt(
            PromptLevel::Warning,
            &t!("RemoteFileEditor.prompt.conflict_title"),
            Some(&message),
            &[overwrite.as_str(), reload.as_str(), cancel.as_str()],
            cx,
        );
        let entity = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let selection = answer.await.ok();
                let _ = entity.update_in(cx, |this, window, cx| match selection {
                    Some(0) => this.upload(conflict.bytes, conflict.local_hash, window, cx),
                    Some(1) => this.reload(window, cx),
                    _ => this.finish_sync(window, cx),
                });
            })
            .detach();
    }

    fn upload(
        &mut self,
        bytes: Vec<u8>,
        local_hash: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let client = self.client.clone();
        let remote_path = self.remote_path.clone();
        let task = Tokio::spawn_result(cx, async move {
            let mut client = client.lock().await;
            client.write_file(&remote_path, &bytes).await?;
            let metadata = client
                .stat(&remote_path)
                .await?
                .ok_or_else(|| anyhow!("Remote file disappeared after upload"))?;
            Ok((snapshot_from_metadata(&metadata), local_hash))
        });
        self.await_snapshot_task(
            SnapshotCompletion {
                task,
                success_message: t!("RemoteFileEditor.notification.external_uploaded").to_string(),
                remote_changed: true,
            },
            window,
            cx,
        );
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        let remote_path = self.remote_path.clone();
        let local_path = self.local_path.clone();
        let task = Tokio::spawn_result(cx, async move {
            let mut client = client.lock().await;
            let metadata = client
                .stat(&remote_path)
                .await?
                .ok_or_else(|| anyhow!("Remote file no longer exists"))?;
            let bytes = client
                .read_file(&remote_path, MAX_EDITABLE_FILE_SIZE)
                .await?;
            let local_hash = local_content_hash(&bytes);
            tokio::fs::write(local_path, bytes).await?;
            Ok((snapshot_from_metadata(&metadata), local_hash))
        });
        self.suppress_until = Some(std::time::Instant::now() + RELOAD_SUPPRESSION);
        self.await_snapshot_task(
            SnapshotCompletion {
                task,
                success_message: t!("RemoteFileEditor.notification.external_reloaded").to_string(),
                remote_changed: false,
            },
            window,
            cx,
        );
    }

    fn await_snapshot_task(
        &mut self,
        completion: SnapshotCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| match completion.task.await {
                Ok((snapshot, local_hash)) => {
                    let _ = entity.update_in(cx, |this, window, cx| {
                        this.snapshot = snapshot;
                        this.last_local_hash = local_hash;
                        if completion.remote_changed {
                            this.on_remote_changed.notify(cx);
                        }
                        window.push_notification(
                            Notification::success(completion.success_message),
                            cx,
                        );
                        this.finish_sync(window, cx);
                    });
                }
                Err(error) => finish_with_error(&entity, error.to_string(), cx),
            })
            .detach();
    }

    fn finish_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = false;
        if std::mem::take(&mut self.pending_sync) {
            self.request_sync(window, cx);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExternalEditSessionKey {
    client_id: usize,
    remote_path: String,
}

impl ExternalEditSessionKey {
    pub(crate) fn new(
        client: &Arc<Mutex<RusshSftpClient>>,
        remote_path: impl Into<String>,
    ) -> Self {
        Self {
            client_id: Arc::as_ptr(client) as usize,
            remote_path: remote_path.into(),
        }
    }
}

struct BoundedSessionRegistry<K, V> {
    capacity: usize,
    entries: VecDeque<(K, V)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionEvictionReason {
    Replaced,
    Capacity,
}

impl<K: Eq, V> BoundedSessionRegistry<K, V> {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "session registry capacity must be positive");
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    fn insert(&mut self, key: K, value: V) -> Option<(SessionEvictionReason, V)> {
        let removed = self
            .entries
            .iter()
            .position(|(existing_key, _)| existing_key == &key)
            .and_then(|index| {
                self.entries
                    .remove(index)
                    .map(|(_, value)| (SessionEvictionReason::Replaced, value))
            })
            .or_else(|| {
                (self.entries.len() >= self.capacity)
                    .then(|| {
                        self.entries
                            .pop_front()
                            .map(|(_, value)| (SessionEvictionReason::Capacity, value))
                    })
                    .flatten()
            });
        self.entries.push_back((key, value));
        removed
    }
}

struct ActiveExternalEditSession {
    // GPUI tasks cancel when dropped. Keep them before the controller so the
    // watch loops stop before the watcher and its sender are released.
    _polling_task: Task<()>,
    _event_task: Task<()>,
    _controller: Entity<ExternalEditController>,
}

struct ExternalEditSessionRegistry {
    sessions: BoundedSessionRegistry<ExternalEditSessionKey, ActiveExternalEditSession>,
}

impl Default for ExternalEditSessionRegistry {
    fn default() -> Self {
        Self {
            sessions: BoundedSessionRegistry::new(MAX_ACTIVE_EXTERNAL_EDIT_SESSIONS),
        }
    }
}

impl gpui::Global for ExternalEditSessionRegistry {}

pub(crate) struct ExternalEditWatchLoop {
    controller: Entity<ExternalEditController>,
    receiver: mpsc::UnboundedReceiver<()>,
}

impl ExternalEditWatchLoop {
    pub(crate) fn new(
        controller: Entity<ExternalEditController>,
        receiver: mpsc::UnboundedReceiver<()>,
    ) -> Self {
        Self {
            controller,
            receiver,
        }
    }

    pub(crate) fn run(self, key: ExternalEditSessionKey, window: &mut Window, cx: &mut App) {
        let Self {
            controller,
            mut receiver,
        } = self;
        let polling_controller = controller.downgrade();
        let polling_task = window.spawn(cx, async move |cx| {
            loop {
                Timer::after(POLL_INTERVAL).await;
                Timer::after(SAVE_DEBOUNCE).await;
                if polling_controller
                    .update_in(cx, |this, window, cx| {
                        this.request_sync(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let event_controller = controller.downgrade();
        let event_task = window.spawn(cx, async move |cx| {
            while receiver.next().await.is_some() {
                Timer::after(SAVE_DEBOUNCE).await;
                while receiver.try_recv().is_ok() {}
                if event_controller
                    .update_in(cx, |this, window, cx| {
                        this.request_sync(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let session = ActiveExternalEditSession {
            _polling_task: polling_task,
            _event_task: event_task,
            _controller: controller,
        };
        let evicted = cx
            .default_global::<ExternalEditSessionRegistry>()
            .sessions
            .insert(key, session);
        if let Some((reason, session)) = evicted {
            drop(session);
            let message = match reason {
                SessionEvictionReason::Replaced => {
                    t!("RemoteFileEditor.notification.external_session_replaced")
                }
                SessionEvictionReason::Capacity => {
                    t!("RemoteFileEditor.notification.external_session_evicted")
                }
            };
            window.push_notification(Notification::warning(message), cx);
        }
    }
}

pub(crate) fn local_content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn finish_with_error(
    entity: &WeakEntity<ExternalEditController>,
    message: String,
    cx: &mut gpui::AsyncWindowContext,
) {
    let _ = entity.update_in(cx, |this, window, cx| {
        window.push_notification(Notification::error(message), cx);
        this.finish_sync(window, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::{BoundedSessionRegistry, SessionEvictionReason, local_content_hash};

    #[test]
    fn local_content_hash_changes_with_file_content() {
        assert_eq!(local_content_hash(b"same"), local_content_hash(b"same"));
        assert_ne!(local_content_hash(b"before"), local_content_hash(b"after"));
    }

    #[test]
    fn bounded_session_registry_replaces_existing_key_without_growing() {
        let mut registry = BoundedSessionRegistry::new(2);

        assert_eq!(None, registry.insert("same", 1));
        assert_eq!(
            Some((SessionEvictionReason::Replaced, 1)),
            registry.insert("same", 2)
        );
        assert_eq!(1, registry.entries.len());
        assert_eq!(Some(&("same", 2)), registry.entries.front());
    }

    #[test]
    fn bounded_session_registry_evicts_oldest_key() {
        let mut registry = BoundedSessionRegistry::new(2);

        assert_eq!(None, registry.insert("first", 1));
        assert_eq!(None, registry.insert("second", 2));
        assert_eq!(
            Some((SessionEvictionReason::Capacity, 1)),
            registry.insert("third", 3)
        );
        assert_eq!(
            vec![("second", 2), ("third", 3)],
            registry.entries.into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn replacing_key_refreshes_its_eviction_order() {
        let mut registry = BoundedSessionRegistry::new(2);

        registry.insert("first", 1);
        registry.insert("second", 2);
        assert_eq!(
            Some((SessionEvictionReason::Replaced, 1)),
            registry.insert("first", 3)
        );
        assert_eq!(
            Some((SessionEvictionReason::Capacity, 2)),
            registry.insert("third", 4)
        );
        assert_eq!(
            vec![("first", 3), ("third", 4)],
            registry.entries.into_iter().collect::<Vec<_>>()
        );
    }
}
