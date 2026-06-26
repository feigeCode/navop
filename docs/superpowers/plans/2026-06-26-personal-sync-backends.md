# Personal Sync Backends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build production-grade personal sync backends for folder/iCloud/OneDrive-directory and Git-based sync of personal connections and workspaces.

**Architecture:** Add a `cloud_sync::personal` subsystem beside the existing Supabase sync path. The new subsystem stores existing encrypted `CloudSyncData` records through a narrow `PersonalSyncStore`, then layers directory storage, Git wrapping, durable conflict/status repositories, and a single-flight background worker on top.

**Tech Stack:** Rust 2024, `one-core`, `rusqlite`, `serde_json`, `uuid`, `tokio`, `notify`, GPUI settings UI, existing `CloudSyncData`, `SyncTypeHandler`, and `CloudSyncService`.

---

## File Structure

Create focused core modules under `crates/core/src/cloud_sync/personal/`:

- `mod.rs`: public exports and module docs.
- `models.rs`: personal backend settings, status, errors, device ID, manifest, tombstone, conflict metadata.
- `store.rs`: `PersonalSyncStore`, lock guard, and store transaction traits.
- `file_format.rs`: `.onetcli-sync` path layout, manifest parsing, record/tombstone serialization helpers.
- `directory_store.rs`: local folder/iCloud/OneDrive-directory store implementation.
- `git_store.rs`: Git wrapper store and injectable `GitRunner`.
- `planner.rs`: store-agnostic personal sync plan and conflict classification.
- `state.rs`: durable status/conflict/pending retry repository helpers.
- `worker.rs`: debounced single-flight event worker.
- `test_support.rs`: test-only helpers for records, temp folders, fake stores, and fake Git.

Modify existing files:

- `crates/core/src/cloud_sync/mod.rs`: export `personal`.
- `crates/core/src/storage/migration.rs`: register new migration.
- `crates/core/migrations/20260626000001_personal_sync.sql`: add state/conflict/retry tables.
- `crates/core/src/storage/manager.rs`: register personal sync repositories if needed.
- `crates/core/src/settings.rs`: add `PersonalSyncSettings` to `AppSettings`.
- `crates/core/Cargo.toml`: add `notify` dependency and `tempfile` dev-dependency.
- `main/src/setting_tab.rs`: add Sync settings page/group and actions.
- `main/locales/main.yml` and `crates/core/locales/core.yml`: add UI/status strings.

Keep existing Supabase modules unchanged except where shared type exports are needed.

---

### Task 1: Personal Sync Models And File Format

**Files:**
- Create: `crates/core/src/cloud_sync/personal/mod.rs`
- Create: `crates/core/src/cloud_sync/personal/models.rs`
- Create: `crates/core/src/cloud_sync/personal/file_format.rs`
- Create: `crates/core/src/cloud_sync/personal/test_support.rs`
- Modify: `crates/core/src/cloud_sync/mod.rs`
- Modify: `crates/core/Cargo.toml`

- [ ] **Step 1: Add failing file-format tests**

Add tests in `file_format.rs` before production implementation:

```rust
#[test]
fn layout_builds_expected_paths() {
    let layout = SyncPackageLayout::new(PathBuf::from("/sync-root"));

    assert_eq!(Path::new("/sync-root/.onetcli-sync/manifest.json"), layout.manifest_path());
    assert_eq!(
        Path::new("/sync-root/.onetcli-sync/records/connection/record-1.json"),
        layout.record_path("connection", "record-1")
    );
    assert_eq!(
        Path::new("/sync-root/.onetcli-sync/tombstones/record-1.json"),
        layout.tombstone_path("record-1")
    );
}

#[test]
fn manifest_rejects_newer_schema() {
    let manifest = PersonalSyncManifest {
        schema_version: SUPPORTED_SCHEMA_VERSION + 1,
        app: APP_ID.to_string(),
        profile_id: PERSONAL_PROFILE_ID.to_string(),
        created_at: 10,
        updated_at: 20,
    };

    assert_eq!(Err(SyncStoreError::SchemaUnsupported { found: SUPPORTED_SCHEMA_VERSION + 1 }),
        manifest.validate());
}

#[test]
fn tombstone_round_trip_preserves_delete_metadata() {
    let tombstone = SyncTombstone {
        id: "record-1".to_string(),
        data_type: data_type::CONNECTION.to_string(),
        deleted_at: 1000,
        version: 4,
        checksum: "abc".to_string(),
    };

    let json = serde_json::to_string(&tombstone).expect("tombstone serializes");
    let parsed: SyncTombstone = serde_json::from_str(&json).expect("tombstone parses");

    assert_eq!(tombstone, parsed);
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::file_format
```

Expected: fails because `cloud_sync::personal` and the model/layout types do not exist.

- [ ] **Step 3: Implement minimal models and layout**

Implement:

```rust
pub const APP_ID: &str = "onetcli";
pub const PERSONAL_PROFILE_ID: &str = "personal";
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSyncManifest {
    pub schema_version: u32,
    pub app: String,
    pub profile_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub id: String,
    pub data_type: String,
    pub deleted_at: i64,
    pub version: u32,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStoreError {
    NotConfigured,
    DirectoryUnavailable(String),
    SchemaUnsupported { found: u32 },
    Conflict(String),
    LockTimeout,
    GitAuthRequired,
    GitMergeConflict,
    Io(String),
    Parse(String),
}
```

Implement `SyncPackageLayout` with `manifest_path`, `record_path`, `tombstone_path`,
`records_dir`, `tombstones_dir`, `state_dir`, and `lock_path`.

- [ ] **Step 4: Run GREEN test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::file_format
```

Expected: file-format tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/Cargo.toml crates/core/src/cloud_sync/mod.rs crates/core/src/cloud_sync/personal
git commit -m "feat(sync): add personal sync file format models"
```

---

### Task 2: DirectorySyncStore

**Files:**
- Create: `crates/core/src/cloud_sync/personal/store.rs`
- Create: `crates/core/src/cloud_sync/personal/directory_store.rs`
- Modify: `crates/core/src/cloud_sync/personal/mod.rs`
- Test: `crates/core/src/cloud_sync/personal/directory_store.rs`

- [ ] **Step 1: Add failing directory store tests**

Add tests:

```rust
#[tokio::test]
async fn probe_initializes_missing_sync_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());

    let status = store.probe().await.expect("probe succeeds");

    assert_eq!(SyncStoreHealth::Ready, status.health);
    assert!(temp.path().join(".onetcli-sync/manifest.json").exists());
}

#[tokio::test]
async fn upsert_record_writes_and_lists_by_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");

    let stored = store.upsert_record(&record, None).await.expect("upsert succeeds");
    let records = store.list_records(Some(data_type::CONNECTION), None).await.expect("list succeeds");

    assert_eq!(record.id, stored.id);
    assert_eq!(vec![stored], records);
}

#[tokio::test]
async fn upsert_rejects_stale_expected_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let record = test_record("connection-1", data_type::CONNECTION, 3, "checksum-1");
    store.upsert_record(&record, None).await.expect("seed succeeds");

    let err = store.upsert_record(&record, Some(2)).await.expect_err("stale write conflicts");

    assert!(matches!(err, SyncStoreError::Conflict(_)));
}

#[tokio::test]
async fn tombstone_hides_active_record_but_keeps_delete_marker() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    store.upsert_record(&record, None).await.expect("upsert succeeds");

    store.tombstone_record("connection-1", Some(1)).await.expect("tombstone succeeds");
    let records = store.list_records(Some(data_type::CONNECTION), None).await.expect("list succeeds");

    assert_eq!(1, records.len());
    assert_eq!(Some(true), records.first().map(|record| record.deleted_at.is_some()));
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::directory_store
```

Expected: fails because `DirectorySyncStore` and `PersonalSyncStore` do not exist.

- [ ] **Step 3: Implement store trait and directory store**

Implement `PersonalSyncStore`:

```rust
#[async_trait]
pub trait PersonalSyncStore: Send + Sync {
    fn backend_id(&self) -> &'static str;
    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError>;
    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError>;
    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError>;
    async fn tombstone_record(
        &self,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError>;
    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError>;
}
```

Implement `DirectorySyncStore` with atomic writes:

```rust
fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<(), SyncStoreError> {
    let parent = path.parent().ok_or_else(|| SyncStoreError::Io("missing parent".to_string()))?;
    fs::create_dir_all(parent)?;
    let temp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&temp_path, bytes)?;
    fs::rename(&temp_path, path)?;
    Ok(())
}
```

Convert `std::io::Error` and `serde_json::Error` into `SyncStoreError`.

- [ ] **Step 4: Run GREEN test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::directory_store
```

Expected: directory store tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/cloud_sync/personal crates/core/Cargo.toml
git commit -m "feat(sync): add directory personal sync store"
```

---

### Task 3: Personal Sync Durable State

**Files:**
- Create: `crates/core/migrations/20260626000001_personal_sync.sql`
- Create: `crates/core/src/cloud_sync/personal/state.rs`
- Modify: `crates/core/src/storage/migration.rs`
- Modify: `crates/core/src/storage/manager.rs`

- [ ] **Step 1: Add failing repository tests**

Add tests in `state.rs`:

```rust
#[test]
fn conflict_repository_round_trips_paused_conflict() {
    let conn = test_connection();
    run_migrations(&conn).expect("migrations run");
    let repo = PersonalSyncConflictRepository::new(conn.clone());
    let conflict = PersonalSyncConflict {
        backend_profile_id: "personal".to_string(),
        record_id: "record-1".to_string(),
        data_type: data_type::CONNECTION.to_string(),
        conflict_type: PersonalConflictType::BothModified,
        local_snapshot: Some("local".to_string()),
        remote_snapshot: Some("remote".to_string()),
        detected_at: 100,
    };

    repo.upsert(&conflict).expect("conflict stored");
    let loaded = repo.list("personal").expect("conflicts list");

    assert_eq!(vec![conflict], loaded);
}

#[test]
fn status_repository_persists_last_success_and_pause_reason() {
    let conn = test_connection();
    run_migrations(&conn).expect("migrations run");
    let repo = PersonalSyncStatusRepository::new(conn.clone());
    let status = PersonalSyncStoredStatus {
        backend_profile_id: "personal".to_string(),
        health: SyncStoreHealth::PausedAfterRepeatedFailures,
        last_success_at: Some(120),
        last_error: Some("git auth required".to_string()),
        updated_at: 130,
    };

    repo.save(&status).expect("status stored");
    let loaded = repo.get("personal").expect("status loads").expect("status exists");

    assert_eq!(status, loaded);
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::state
```

Expected: fails because migration and repositories do not exist.

- [ ] **Step 3: Add migration and repositories**

Migration:

```sql
CREATE TABLE IF NOT EXISTS personal_sync_conflicts (
    backend_profile_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    data_type TEXT NOT NULL,
    conflict_type TEXT NOT NULL,
    local_snapshot TEXT,
    remote_snapshot TEXT,
    detected_at INTEGER NOT NULL,
    PRIMARY KEY (backend_profile_id, record_id)
);

CREATE TABLE IF NOT EXISTS personal_sync_status (
    backend_profile_id TEXT PRIMARY KEY,
    health TEXT NOT NULL,
    last_success_at INTEGER,
    last_error TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS personal_sync_retry_queue (
    backend_profile_id TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    retry_count INTEGER NOT NULL,
    next_retry_at INTEGER,
    last_error TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (backend_profile_id, operation_key)
);
```

Register version `20260626000001` in `MIGRATIONS`.

- [ ] **Step 4: Run GREEN test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::state
```

Expected: state repository tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/migrations/20260626000001_personal_sync.sql crates/core/src/storage/migration.rs crates/core/src/storage/manager.rs crates/core/src/cloud_sync/personal/state.rs
git commit -m "feat(sync): persist personal sync state"
```

---

### Task 4: Personal Sync Settings

**Files:**
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/src/settings.rs`

- [ ] **Step 1: Add failing settings tests**

Add tests:

```rust
#[test]
fn app_settings_default_disables_personal_sync() {
    let settings = AppSettings::default();

    assert!(!settings.personal_sync.enabled);
    assert_eq!(PersonalSyncBackendKind::Folder, settings.personal_sync.backend);
    assert!(settings.personal_sync.path.is_empty());
    assert!(settings.personal_sync.auto_sync);
    assert!(settings.personal_sync.git.auto_push);
}

#[test]
fn app_settings_deserializes_personal_sync_defaults_from_legacy_json() {
    let settings: AppSettings = serde_json::from_value(serde_json::json!({
        "locale": "en",
        "theme_mode": "dark"
    }))
    .expect("legacy settings should load");

    assert!(!settings.personal_sync.enabled);
    assert!(settings.personal_sync.auto_sync);
}

#[test]
fn app_settings_round_trip_preserves_personal_sync() {
    let mut settings = AppSettings::default();
    settings.personal_sync.enabled = true;
    settings.personal_sync.backend = PersonalSyncBackendKind::Git;
    settings.personal_sync.path = "/tmp/repo".to_string();
    settings.personal_sync.git.auto_push = false;

    let json = serde_json::to_string(&settings).expect("settings serialize");
    let loaded: AppSettings = serde_json::from_str(&json).expect("settings deserialize");

    assert_eq!(settings.personal_sync, loaded.personal_sync);
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core settings::tests::app_settings_default_disables_personal_sync
```

Expected: fails because `personal_sync` settings do not exist.

- [ ] **Step 3: Implement settings structs**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalSyncBackendKind {
    Folder,
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalGitSyncSettings {
    #[serde(default = "default_true")]
    pub auto_push: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSyncSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub backend: PersonalSyncBackendKind,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_true")]
    pub auto_sync: bool,
    #[serde(default)]
    pub git: PersonalGitSyncSettings,
}
```

Add `personal_sync: PersonalSyncSettings` to `AppSettings` with default and legacy-safe
serde defaults.

- [ ] **Step 4: Run GREEN settings tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core settings::tests::app_settings
```

Expected: settings tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/settings.rs
git commit -m "feat(sync): add personal sync settings"
```

---

### Task 5: Personal Sync Planner

**Files:**
- Create: `crates/core/src/cloud_sync/personal/planner.rs`
- Modify: `crates/core/src/cloud_sync/personal/mod.rs`

- [ ] **Step 1: Add failing planner tests**

Add tests:

```rust
#[test]
fn planner_uploads_new_local_personal_record() {
    let local = vec![local_item("local-1", None, 200, None)];
    let remote = Vec::new();

    let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

    assert_eq!(vec!["local-1"], plan.to_upload_local_ids());
    assert!(plan.conflicts.is_empty());
}

#[test]
fn planner_skips_team_records() {
    let local = vec![local_item("local-1", None, 200, Some("team-1"))];
    let remote = Vec::new();

    let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

    assert!(plan.is_empty());
}

#[test]
fn planner_conflicts_when_both_sides_modified_with_different_checksum() {
    let local = vec![local_item_with_sync("local-1", "cloud-1", 300, 100, "local-checksum")];
    let remote = vec![test_record("cloud-1", data_type::CONNECTION, 2, "remote-checksum")
        .with_updated_at(300_000)];

    let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

    assert_eq!(PersonalConflictType::BothModified, plan.conflicts[0].conflict_type);
}

#[test]
fn planner_treats_matching_checksum_as_synced_even_with_different_timestamps() {
    let local = vec![local_item_with_sync("local-1", "cloud-1", 300, 100, "same")];
    let remote = vec![test_record("cloud-1", data_type::CONNECTION, 2, "same")
        .with_updated_at(500_000)];

    let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

    assert!(plan.conflicts.is_empty());
    assert_eq!(vec!["cloud-1"], plan.to_mark_synced_cloud_ids());
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::planner
```

Expected: fails because planner does not exist.

- [ ] **Step 3: Implement planner**

Implement planner over a small `PersonalSyncItem` trait rather than directly coupling to
connections. Include:

- upload new local item with no cloud ID.
- download remote item with no local cloud ID match.
- update cloud when local updated since last sync and remote unchanged.
- update local when remote updated since last sync and local unchanged.
- conflict when both changed with different checksum.
- conflict for local delete vs remote modified and local modified vs remote tombstone.
- skip local items with `team_id`.
- skip paused record IDs.

- [ ] **Step 4: Run GREEN planner tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::planner
```

Expected: planner tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/cloud_sync/personal/planner.rs crates/core/src/cloud_sync/personal/mod.rs
git commit -m "feat(sync): plan personal record sync"
```

---

### Task 6: Personal Sync Worker Core

**Files:**
- Create: `crates/core/src/cloud_sync/personal/worker.rs`
- Modify: `crates/core/src/cloud_sync/personal/mod.rs`

- [ ] **Step 1: Add failing worker tests**

Add tests with fake store and fake local source:

```rust
#[tokio::test]
async fn worker_coalesces_events_and_runs_single_sync_pass() {
    let store = FakePersonalSyncStore::default();
    let local = FakePersonalSyncLocalSource::default();
    let worker = PersonalSyncWorker::new(store.clone(), local.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::LocalChanged { data_type: data_type::CONNECTION.to_string(), local_id: 1 });
    worker.enqueue(PersonalSyncEvent::LocalChanged { data_type: data_type::CONNECTION.to_string(), local_id: 1 });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(1, store.list_calls());
}

#[tokio::test]
async fn worker_pauses_conflicting_record() {
    let store = FakePersonalSyncStore::with_records(vec![remote_record_conflicting()]);
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record_conflicting()]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(store, local, conflicts.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::FullScan);
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(vec!["cloud-1"], conflicts.paused_record_ids());
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker
```

Expected: fails because worker does not exist.

- [ ] **Step 3: Implement worker core**

Implement:

- `PersonalSyncEvent`.
- queue with coalescing.
- `dirty` flag during active sync.
- `drain_once` test hook.
- `PersonalSyncLocalSource` trait to decouple local repositories.
- conflict sink interface for persisted conflicts.
- no filesystem watcher yet.

- [ ] **Step 4: Run GREEN worker tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker
```

Expected: worker core tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/cloud_sync/personal/worker.rs crates/core/src/cloud_sync/personal/test_support.rs crates/core/src/cloud_sync/personal/mod.rs
git commit -m "feat(sync): add personal sync worker core"
```

---

### Task 7: GitSyncStore

**Files:**
- Create: `crates/core/src/cloud_sync/personal/git_store.rs`
- Modify: `crates/core/src/cloud_sync/personal/mod.rs`

- [ ] **Step 1: Add failing Git store tests**

Add fake-runner tests:

```rust
#[tokio::test]
async fn git_store_pulls_before_probe_and_pushes_after_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new()
        .with_success("pull")
        .with_success("add")
        .with_success("commit")
        .with_success("push");
    let store = GitSyncStore::new(temp.path().to_path_buf(), runner.clone(), GitSyncOptions { auto_push: true });
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");

    store.probe().await.expect("probe succeeds");
    store.upsert_record(&record, None).await.expect("upsert succeeds");
    store.flush().await.expect("flush succeeds");

    assert_eq!(vec!["pull --rebase", "add .onetcli-sync", "commit onetcli sync: update personal records", "push"],
        runner.commands());
}

#[tokio::test]
async fn git_store_maps_auth_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new().with_error("pull", GitRunnerError::AuthRequired);
    let store = GitSyncStore::new(temp.path().to_path_buf(), runner, GitSyncOptions { auto_push: true });

    let err = store.probe().await.expect_err("auth failure maps");

    assert_eq!(SyncStoreError::GitAuthRequired, err);
}

#[tokio::test]
async fn git_store_maps_merge_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new().with_error("pull", GitRunnerError::MergeConflict);
    let store = GitSyncStore::new(temp.path().to_path_buf(), runner, GitSyncOptions { auto_push: true });

    let err = store.probe().await.expect_err("merge failure maps");

    assert_eq!(SyncStoreError::GitMergeConflict, err);
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::git_store
```

Expected: fails because `GitSyncStore` does not exist.

- [ ] **Step 3: Implement Git wrapper**

Implement:

- `GitRunner` trait with `pull_rebase`, `add_sync_package`, `commit_sync_package`,
  `push`, `remote_url`, `is_repo`, and `is_clean_for_sync`.
- `CommandGitRunner` using `std::process::Command`.
- `GitSyncStore` wraps `DirectorySyncStore`.
- `flush` performs add/commit/push only when `.onetcli-sync` changed.
- no credential persistence.

- [ ] **Step 4: Run GREEN Git tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::git_store
```

Expected: Git store tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/cloud_sync/personal/git_store.rs crates/core/src/cloud_sync/personal/mod.rs
git commit -m "feat(sync): add git personal sync store"
```

---

### Task 8: Settings UI

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`
- Test: `main/src/setting_tab.rs`

- [ ] **Step 1: Add focused UI/settings tests**

Add pure helper tests rather than brittle full rendering first:

```rust
#[test]
fn personal_sync_backend_options_include_folder_and_git() {
    let options = personal_sync_backend_options();

    assert_eq!(
        vec![("folder".into(), t!("Settings.Sync.Backend.folder").into()),
             ("git".into(), t!("Settings.Sync.Backend.git").into())],
        options
    );
}

#[test]
fn personal_sync_status_label_maps_git_auth_required() {
    assert_eq!(
        t!("Settings.Sync.Status.git_auth_required").to_string(),
        personal_sync_status_label(&SyncStoreHealth::GitAuthRequired)
    );
}
```

- [ ] **Step 2: Run RED test**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
```

Expected: fails because UI helpers do not exist.

- [ ] **Step 3: Implement UI group**

Add a `SettingPage::new(t!("Settings.Sync.title"))` or a dedicated group with:

- enable switch.
- backend dropdown/segmented control.
- path field.
- auto sync switch.
- Git auto push switch shown for Git backend.
- test connection button.
- sync now button.
- status text.

Use `AppSettings::update_and_save` for persisted settings. Do not start the worker from UI
until Task 9 wires runtime service lifecycle.

- [ ] **Step 4: Run GREEN UI helper tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
```

Expected: UI helper tests pass.

- [ ] **Step 5: Commit**

```bash
git add main/src/setting_tab.rs main/locales/main.yml
git commit -m "feat(sync): add personal sync settings UI"
```

---

### Task 9: Runtime Wiring And Watcher

**Files:**
- Modify: `main/src/app_init.rs`
- Modify: `main/src/onetcli_app.rs`
- Modify: `crates/core/src/cloud_sync/personal/worker.rs`
- Modify: `crates/core/src/cloud_sync/personal/directory_store.rs`

- [ ] **Step 1: Add runtime lifecycle tests**

Add tests around pure lifecycle helpers:

```rust
#[test]
fn personal_sync_runtime_is_disabled_without_path() {
    let settings = PersonalSyncSettings {
        enabled: true,
        path: String::new(),
        ..PersonalSyncSettings::default()
    };

    assert_eq!(Err(PersonalSyncRuntimeError::NotConfigured), build_personal_sync_runtime_config(&settings));
}

#[test]
fn watcher_ignores_self_written_path_within_window() {
    let mut guard = SelfWriteGuard::new(Duration::from_secs(2));
    let path = PathBuf::from("/sync/.onetcli-sync/records/connection/a.json");

    guard.mark_written(path.clone(), Instant::now());

    assert!(guard.should_ignore(&path, Instant::now() + Duration::from_millis(500)));
    assert!(!guard.should_ignore(&path, Instant::now() + Duration::from_secs(3)));
}
```

- [ ] **Step 2: Run RED runtime tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker::tests::watcher_ignores_self_written_path_within_window
```

Expected: fails because runtime helpers do not exist.

- [ ] **Step 3: Implement runtime lifecycle**

Implement:

- worker start/stop service from app initialization when `personal_sync.enabled`.
- file watcher with `notify` observing `records` and `tombstones`.
- `SelfWriteGuard` to suppress self-triggered events.
- app setting change restarts worker when backend/path changes.
- manual "sync now" sends `FullScan`.

- [ ] **Step 4: Run GREEN runtime tests**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p main
```

Expected: worker tests and main check pass.

- [ ] **Step 5: Commit**

```bash
git add main/src/app_init.rs main/src/onetcli_app.rs crates/core/src/cloud_sync/personal
git commit -m "feat(sync): wire personal sync runtime"
```

---

### Task 10: End-To-End Verification And Hardening

**Files:**
- Verify: `crates/core/src/cloud_sync/personal/*.rs`
- Verify: `crates/core/src/settings.rs`
- Verify: `crates/core/src/storage/migration.rs`
- Verify: `main/src/setting_tab.rs`
- Verify: `main/locales/main.yml`
- Update: `docs/superpowers/specs/2026-06-26-personal-sync-backends-design.md` only when the implementation deliberately changes a documented contract.

- [ ] **Step 1: Run targeted test suite**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p main
```

Expected:

- all personal sync tests pass.
- existing 25 cloud sync tests pass.
- main personal sync helper tests pass.
- main crate compiles.

- [ ] **Step 2: Run full relevant build**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo build
```

Expected: workspace build passes.

- [ ] **Step 3: Manual smoke check**

Run app from worktree if build succeeds:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo run -p main
```

Manual checks:

- Settings has Sync page/group.
- Folder backend can choose a temp folder and initialize `.onetcli-sync`.
- "Sync now" writes encrypted record files after a personal connection/workspace exists.
- Git backend reports clear status for non-repo folder.
- Git backend does not ask OnetCli to store credentials.

- [ ] **Step 4: Final status audit**

Run:

```bash
git status --short
git log --oneline --decorate -10
```

Expected:

- only intentional committed work is present.
- no generated temp files or accidental sync packages are tracked.

- [ ] **Step 5: Commit final fixes**

```bash
git add crates/core/src/cloud_sync/personal \
  crates/core/src/settings.rs \
  crates/core/src/storage/migration.rs \
  crates/core/migrations/20260626000001_personal_sync.sql \
  main/src/setting_tab.rs \
  main/locales/main.yml \
  docs/superpowers/specs/2026-06-26-personal-sync-backends-design.md
git commit -m "test(sync): verify personal sync backends"
```

Create this commit only when Step 1 or Step 2 required fixes after Task 9. If no files
changed during verification, skip this commit and record the clean verification output.

---

## Self-Review

Spec coverage:

- Personal-only scope: Tasks 5, 6, and 9 filter team data and avoid team sync changes.
- Folder/iCloud/OneDrive-directory backend: Tasks 1 and 2 implement file format and directory store.
- Git backend: Task 7 implements Git wrapper with credential-free command runner.
- Near-real-time automatic sync: Tasks 6 and 9 implement worker, debounce, dirty reruns, watcher, and retry pause.
- Conflict pause and manual resolution: Tasks 3, 5, 6, and 8 persist conflicts, pause records, list conflicts in UI, and expose use-local/use-remote/keep-both resolution actions before the feature can be considered complete.
- Security: Tasks 1, 2, and 7 store encrypted records only and avoid credential persistence.
- Verification: Task 10 covers targeted and build checks.

Known implementation risk:

- The spec's conflict list and resolution UI is broad. If Task 8 cannot fit a polished full resolver, ship a production-safe "conflicts paused and listed with sync disabled for that record" core state first, then add resolver controls before claiming full feature completion.
