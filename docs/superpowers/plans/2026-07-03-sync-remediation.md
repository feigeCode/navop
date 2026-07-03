# Sync Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复个人同步与 OnetCloud 同步审查中发现的高风险问题，确保冲突可见、自动同步不丢事件、个人后端并发版本有效、OnetCloud sync_data 版本语义可验证，并明确 `onetcli cloud` CLI 边界。

**Architecture:** 个人同步继续复用 `cloud_sync::personal` store/worker/local source 分层，但补上 SQLite 持久化状态接入、单飞事件补跑、watcher 初始化和版本推进。OnetCloud 同步保留现有 `SyncEngine`/`CloudApiClient` 架构，补齐 Supabase `sync_data` version trigger 或 RPC，并对 workspace 冲突语义作出明确实现。CLI 只在确认需要 `onetcli cloud` 命令时扩展 parser 和 runtime host。

**Tech Stack:** Rust 2024, GPUI, Tokio, rusqlite, notify, Supabase PostgREST/RPC, existing `one-core` storage and cloud sync modules.

---

## Scope And Severity

### P0: 必须修复

1. 个人同步冲突被 `NoopConflictSink` 静默吞掉。
2. 个人自动同步在 `Syncing` 状态下直接丢事件。
3. 个人 watcher 对全新目录可能未监听任何路径。
4. 个人 folder/Git 后端成功写入不推进 `CloudSyncData.version`。
5. OnetCloud 普通 `sync_data` update 的 version 推进缺少仓库内证据。

### P1: 需要产品语义确认后修复

1. OnetCloud workspace 目前是 last-writer-wins，没有连接同步那样的冲突模型。
2. `onetcli cloud` CLI 子命令当前不存在；如果这是用户可见需求，需要补 parser 和 runtime 执行入口。
3. OnetCloud soft delete 是否必须使用乐观版本锁。若要求严格并发保护，需要扩展 API 和本地 cloud version 记录。

---

## File Structure

- Modify: `crates/core/src/cloud_sync/personal/state.rs`
  - 将 personal sync conflict/status repository 改为可注册到 `StorageManager` 的 `SqliteConnection` 仓储。
- Modify: `crates/core/src/storage/repository.rs`
  - 注册 personal sync conflict/status repository。
- Modify: `crates/core/src/cloud_sync/personal/worker.rs`
  - 输出同步结果中的 conflict count，确保 conflict 不再静默成功。
- Modify: `main/src/personal_sync_runtime.rs`
  - 接入 SQLite conflict sink/status sink；修复 `Syncing` 时事件丢失；统一 runtime worker 的 manual/auto drain。
- Modify: `crates/core/src/cloud_sync/personal/runtime.rs`
  - watcher 启动时确保 `.onetcli-sync/records` 和 `.onetcli-sync/tombstones` 目录存在。
- Modify: `crates/core/src/cloud_sync/personal/directory_store.rs`
  - upsert/tombstone 成功时递增 version 并刷新 updated_at。
- Modify: `crates/core/src/cloud_sync/personal/git_store.rs`
  - 继承 directory store version 行为，保持 flush 流程不变。
- Modify: `crates/core/src/cloud_sync/supabase.rs`
  - 补充普通 update version 推进验证；若选择 RPC，则改 update/delete 调用。
- Create: `docs/superpowers/specs/2026-07-03-sync-data-version-trigger.sql`
  - Supabase `sync_data` version trigger 或 RPC SQL。
- Optional Modify: `crates/core/src/storage/models.rs`
  - 若 workspace 需要冲突检测，新增 `last_synced_at` 和可选 `cloud_version`。
- Optional Modify: `crates/core/migrations/20260703000001_workspace_sync_metadata.sql`
  - 若 workspace 需要冲突检测，新增本地字段。
- Optional Modify: `crates/onetcli_cli/src/lib.rs`, `crates/onetcli_cli/src/tests.rs`
  - 若需要 `onetcli cloud` CLI，新增 parser。
- Optional Modify: `crates/onetcli_runtime/src/cli_host.rs`
  - 若需要 `onetcli cloud` CLI，接入执行入口。

---

## Task 1: 接入个人同步冲突和状态持久化

**Files:**
- Modify: `crates/core/src/cloud_sync/personal/state.rs`
- Modify: `crates/core/src/storage/repository.rs`
- Modify: `main/src/personal_sync_runtime.rs`
- Test: `crates/core/src/cloud_sync/personal/state.rs`
- Test: `main/src/personal_sync_runtime_tests.rs`

- [ ] **Step 1: 将 personal sync repositories 改为 `SqliteConnection`**

在 `crates/core/src/cloud_sync/personal/state.rs` 中把 repository 持有字段从 `rusqlite::Connection` 改为 `crate::storage::connection::SqliteConnection`。

核心代码形态：

```rust
use crate::storage::connection::SqliteConnection;

#[derive(Clone)]
pub struct PersonalSyncConflictRepository {
    conn: SqliteConnection,
}

impl PersonalSyncConflictRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn upsert(&self, conflict: &PersonalSyncConflict) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO personal_sync_conflicts
                 (backend_profile_id, record_id, data_type, conflict_type, local_snapshot, remote_snapshot, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(backend_profile_id, record_id) DO UPDATE SET
                   data_type = excluded.data_type,
                   conflict_type = excluded.conflict_type,
                   local_snapshot = excluded.local_snapshot,
                   remote_snapshot = excluded.remote_snapshot,
                   detected_at = excluded.detected_at",
                params![
                    conflict.backend_profile_id,
                    conflict.record_id,
                    conflict.data_type,
                    conflict.conflict_type.as_str(),
                    conflict.local_snapshot,
                    conflict.remote_snapshot,
                    conflict.detected_at,
                ],
            )?;
            Ok(())
        })
    }
}
```

同样修改 `PersonalSyncStatusRepository`。

- [ ] **Step 2: 注册 repositories**

在 `crates/core/src/storage/repository.rs::init` 中新增：

```rust
let personal_conflict_repo =
    crate::cloud_sync::personal::PersonalSyncConflictRepository::new(conn.clone());
let personal_status_repo =
    crate::cloud_sync::personal::PersonalSyncStatusRepository::new(conn.clone());

storage.register(personal_conflict_repo);
storage.register(personal_status_repo);
```

- [ ] **Step 3: 添加真实 conflict sink**

在 `main/src/personal_sync_runtime.rs` 中新增一个 runtime 层 adapter：

```rust
#[derive(Clone)]
struct SqlitePersonalSyncConflictSink {
    backend_profile_id: String,
    conflicts: one_core::cloud_sync::personal::PersonalSyncConflictRepository,
}

#[async_trait::async_trait]
impl one_core::cloud_sync::personal::PersonalSyncConflictSink for SqlitePersonalSyncConflictSink {
    async fn paused_record_ids(&self) -> Result<std::collections::HashSet<String>, SyncStoreError> {
        let conflicts = self
            .conflicts
            .list(&self.backend_profile_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))?;
        Ok(conflicts.into_iter().map(|conflict| conflict.record_id).collect())
    }

    async fn pause_record(
        &self,
        conflict: &one_core::cloud_sync::personal::PersonalSyncRecordConflict,
        local: Option<&one_core::cloud_sync::personal::PersonalSyncItemSnapshot>,
        remote: Option<&one_core::cloud_sync::CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        let data_type = remote
            .map(|record| record.data_type.clone())
            .or_else(|| local.map(|item| item.data_type.clone()))
            .unwrap_or_else(|| one_core::cloud_sync::data_type::CONNECTION.to_string());
        let stored = one_core::cloud_sync::personal::PersonalSyncConflict {
            backend_profile_id: self.backend_profile_id.clone(),
            record_id: conflict.cloud_id.clone(),
            data_type,
            conflict_type: conflict.conflict_type,
            local_snapshot: local
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| SyncStoreError::Parse(error.to_string()))?,
            remote_snapshot: remote
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| SyncStoreError::Parse(error.to_string()))?,
            detected_at: current_timestamp(),
        };
        self.conflicts
            .upsert(&stored)
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }
}
```

如果 `PersonalSyncItemSnapshot` 没有 `Serialize`，给它补：

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersonalSyncItemSnapshot { ... }
```

- [ ] **Step 4: worker 不再静默吞 conflict**

在 `crates/core/src/cloud_sync/personal/worker.rs` 中让 `apply_conflicts` 保存完所有冲突后返回 conflict error：

```rust
async fn apply_conflicts(
    &self,
    plan: &PersonalSyncPlan,
    items: &[PersonalSyncItemSnapshot],
    records: &[CloudSyncData],
) -> Result<(), SyncStoreError> {
    for conflict in &plan.conflicts {
        let local = find_local_by_id(items, &conflict.local_id);
        let remote = find_remote_by_id(records, &conflict.cloud_id);
        self.conflicts.pause_record(conflict, local, remote).await?;
    }
    if !plan.conflicts.is_empty() {
        return Err(SyncStoreError::Conflict(format!(
            "{} personal sync conflict(s) paused",
            plan.conflicts.len()
        )));
    }
    Ok(())
}
```

- [ ] **Step 5: runtime 使用真实 sink 创建 worker**

在 `start_running_runtime` 和 `run_sync` 里改用 `PersonalSyncWorker::with_conflict_sink(...)`。`run_sync` 需要接收 sink 或 storage，而不是只接收 `_service`。

核心代码形态：

```rust
let sink = build_conflict_sink(cx)?;
let worker = PersonalSyncWorker::with_conflict_sink(
    store.clone(),
    source,
    sink,
    one_core::cloud_sync::personal::WorkerConfig {
        backend_profile_id: "personal".to_string(),
        device_id: SyncDeviceId(device_id()),
    },
);
```

`device_id()` 先可用稳定本机标识占位：

```rust
fn device_id() -> String {
    format!("local-{}", whoami::hostname())
}
```

如果不想新增依赖，保留 `"local-device"`，但要在后续任务中替换为设置持久化的设备 ID。

- [ ] **Step 6: 更新测试**

修改 `worker_pauses_conflicting_record`：原来期望 `drain_once()` 成功，改为期望返回 `SyncStoreError::Conflict(_)` 且 conflict sink 有记录。

```rust
let error = worker.drain_once().await.expect_err("conflict pauses sync pass");
assert!(matches!(error, SyncStoreError::Conflict(_)));
assert_eq!(vec!["cloud-1"], conflicts.paused_record_ids());
```

新增 `main/src/personal_sync_runtime_tests.rs` 测试：配置 personal sync 后，本地 conflict sink 存在时不再使用 noop sink。该测试可以只覆盖 builder helper，例如把 `build_conflict_sink` 设为 `pub(crate)` 并验证配置缺仓库时返回 `None`、有仓库时返回 `Some(_)`。

- [ ] **Step 7: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
```

Expected: 所有 personal sync 测试通过；冲突测试从“成功但 noop”变为“持久化后返回 conflict”。

---

## Task 2: 修复个人自动同步 `Syncing` 时丢事件

**Files:**
- Modify: `main/src/personal_sync_runtime.rs`
- Test: `main/src/personal_sync_runtime_tests.rs`

- [ ] **Step 1: 给 runtime state 增加补跑标记**

在 `GlobalPersonalSyncRuntime` 增加字段：

```rust
pending_auto_drain: bool,
```

初始化为 `false`。

- [ ] **Step 2: 重构 enqueue 顺序**

把 `enqueue_auto_sync_event` 改为先 enqueue，再判断是否已经在同步中：

```rust
fn enqueue_auto_sync_event(event: PersonalSyncEvent, cx: &mut App) {
    let settings = AppSettings::global(cx);
    if settings.sync_provider != SyncProvider::Personal || !settings.personal_sync.auto_sync {
        return;
    }
    sync_master_key_and_user(cx);

    let Some(runtime) = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
    else {
        return;
    };

    let worker = runtime.worker.clone();
    worker.enqueue(event);

    if matches!(runtime_status(cx), PersonalSyncRuntimeStatus::Syncing) {
        cx.global_mut::<GlobalPersonalSyncRuntime>().pending_auto_drain = true;
        return;
    }

    start_runtime_drain(cx);
}
```

- [ ] **Step 3: 添加统一 drain helper**

新增：

```rust
fn start_runtime_drain(cx: &mut App) {
    let Some(state) = cx.try_global::<GlobalPersonalSyncRuntime>() else {
        return;
    };
    if matches!(state.status, PersonalSyncRuntimeStatus::Syncing) {
        return;
    }
    let Some(runtime) = state.runtime.as_ref() else {
        return;
    };
    let worker = runtime.worker.clone();
    let store = runtime.store.clone();
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let task = Tokio::spawn(cx, drain_and_flush(worker, store));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = personal_sync_status_from_task(task.await);
        let _ = cx.update(move |cx| finish_operation_and_maybe_drain(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}
```

把原 `finish_operation` 改成：

```rust
fn finish_operation_and_maybe_drain(
    cx: &mut App,
    generation: u64,
    status: PersonalSyncRuntimeStatus,
) {
    let should_drain = {
        let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
        if state.generation == generation {
            state.status = status;
        }
        let pending = state.pending_auto_drain;
        state.pending_auto_drain = false;
        pending
    };
    if should_drain {
        start_runtime_drain(cx);
    }
}
```

- [ ] **Step 4: 手动 sync 也使用 runtime worker**

`sync_now` 优先使用已启动 runtime：

```rust
pub fn sync_now(cx: &mut App) {
    sync_master_key_and_user(cx);
    if let Some(runtime) = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
    {
        runtime.worker.enqueue(PersonalSyncEvent::FullScan);
        start_runtime_drain(cx);
        return;
    }

    run_temporary_full_scan(cx);
}
```

保留 `run_temporary_full_scan` 作为没有 active runtime 时的兜底。

- [ ] **Step 5: 添加测试**

在 `main/src/personal_sync_runtime_tests.rs` 增加纯函数或状态 helper 测试，避免 GPUI 异步不稳定：

```rust
#[test]
fn personal_sync_enqueue_while_syncing_marks_pending_drain() {
    let mut state = TestPersonalSyncRuntimeState::syncing();
    state.enqueue_event(PersonalSyncEvent::FullScan);

    assert!(state.worker_has_pending_event(PersonalSyncEvent::FullScan));
    assert!(state.pending_auto_drain);
}
```

如果不引入 test-only state helper，就把 `enqueue_auto_sync_event` 拆成可测试的纯函数：

```rust
fn should_start_drain_after_enqueue(status: &PersonalSyncRuntimeStatus) -> bool {
    !matches!(status, PersonalSyncRuntimeStatus::Syncing)
}
```

并测试 `Syncing` 返回 `false`，但事件已经在调用方 enqueue。

- [ ] **Step 6: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker
```

Expected: personal runtime tests pass；worker coalesce/dirty 行为仍通过。

---

## Task 3: 初始化 watcher 监听目录

**Files:**
- Modify: `crates/core/src/cloud_sync/personal/runtime.rs`
- Test: `crates/core/src/cloud_sync/personal/runtime_tests.rs`

- [ ] **Step 1: watcher 启动时创建目录**

在 `PersonalSyncWatcher::start` 里创建 `records` 和 `tombstones`：

```rust
std::fs::create_dir_all(layout.records_dir())?;
std::fs::create_dir_all(layout.tombstones_dir())?;

watch_required(&mut watcher, &layout.records_dir())?;
watch_required(&mut watcher, &layout.tombstones_dir())?;
```

替换当前 `watch_if_exists`。

- [ ] **Step 2: 添加 required watch helper**

```rust
fn watch_required(watcher: &mut RecommendedWatcher, path: &Path) -> Result<(), SyncStoreError> {
    watcher
        .watch(path, RecursiveMode::Recursive)
        .map_err(|error| SyncStoreError::Io(error.to_string()))
}
```

删除或仅保留不再使用的 `watch_if_exists`。

- [ ] **Step 3: 添加测试**

在 `runtime_tests.rs` 添加：

```rust
#[test]
fn watcher_start_creates_missing_record_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let watcher = PersonalSyncWatcher::start(
        temp.path().to_path_buf(),
        Duration::from_secs(2),
        |_| {},
    )
    .expect("watcher starts");

    assert!(temp.path().join(".onetcli-sync/records").is_dir());
    assert!(temp.path().join(".onetcli-sync/tombstones").is_dir());
    drop(watcher);
}
```

- [ ] **Step 4: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::runtime
```

Expected: watcher guard 测试和目录初始化测试通过。

---

## Task 4: 推进个人后端 record version

**Files:**
- Modify: `crates/core/src/cloud_sync/personal/directory_store.rs`
- Test: `crates/core/src/cloud_sync/personal/directory_store_tests.rs`
- Test: `crates/core/src/cloud_sync/personal/git_store_tests.rs`

- [ ] **Step 1: 新增 version 推进 helper**

在 `directory_store.rs` 新增：

```rust
fn next_stored_record(
    mut record: CloudSyncData,
    existing: Option<&CloudSyncData>,
) -> CloudSyncData {
    if let Some(existing) = existing {
        record.version = existing.version.saturating_add(1);
    } else {
        record.version = record.version.max(1);
    }
    record.updated_at = now_millis();
    record
}
```

- [ ] **Step 2: 修改 upsert**

```rust
async fn upsert_record(
    &self,
    record: &CloudSyncData,
    expected_version: Option<u32>,
) -> Result<CloudSyncData, SyncStoreError> {
    self.initialize_package()?;
    let existing = self.existing_record(&record.id)?;
    self.ensure_expected_version(&record.id, expected_version)?;
    let stored = next_stored_record(record.clone(), existing.as_ref());
    write_json_atomically(
        &self.layout.record_path(&stored.data_type, &stored.id),
        &stored,
    )?;
    Ok(stored)
}
```

- [ ] **Step 3: 修改 tombstone**

```rust
record.deleted_at = Some(now_millis());
record.updated_at = now_millis();
record.version = record.version.saturating_add(1);
write_json_atomically(&self.layout.record_path(&record.data_type, id), &record)?;
write_json_atomically(&self.layout.tombstone_path(id), &tombstone_from(&record))?;
```

- [ ] **Step 4: 添加测试**

在 `directory_store_tests.rs` 添加：

```rust
#[tokio::test]
async fn upsert_advances_version_after_expected_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let first = store.upsert_record(&record, None).await.expect("seed succeeds");

    let mut changed = first.clone();
    changed.checksum = "checksum-2".to_string();
    let second = store
        .upsert_record(&changed, Some(first.version))
        .await
        .expect("expected write succeeds");

    assert_eq!(first.version + 1, second.version);
    assert!(second.updated_at >= first.updated_at);
}

#[tokio::test]
async fn tombstone_advances_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let stored = store.upsert_record(&record, None).await.expect("seed succeeds");

    store
        .tombstone_record("connection-1", Some(stored.version))
        .await
        .expect("tombstone succeeds");
    let records = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("list succeeds");

    assert_eq!(stored.version + 1, records[0].version);
    assert!(records[0].deleted_at.is_some());
}
```

- [ ] **Step 5: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::directory_store
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::git_store
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal::worker
```

Expected: directory store version tests pass；Git store flush 仍通过；worker mark_synced 使用新的 `updated_at / 1000`。

---

## Task 5: 固化 OnetCloud `sync_data` version 推进

**Files:**
- Create: `docs/superpowers/specs/2026-07-03-sync-data-version-trigger.sql`
- Modify: `crates/core/src/cloud_sync/supabase.rs`
- Test: `crates/core/src/cloud_sync/supabase.rs`

- [ ] **Step 1: 添加 Supabase trigger SQL 文档**

创建 `docs/superpowers/specs/2026-07-03-sync-data-version-trigger.sql`：

```sql
create or replace function public.bump_sync_data_version()
returns trigger
language plpgsql
as $$
begin
    if tg_op = 'UPDATE' then
        if old.encrypted_data is distinct from new.encrypted_data
           or old.key_version is distinct from new.key_version
           or old.checksum is distinct from new.checksum
           or old.deleted_at is distinct from new.deleted_at then
            new.version = old.version + 1;
            new.updated_at = now();
        end if;
    end if;
    return new;
end;
$$;

drop trigger if exists trg_bump_sync_data_version on public.sync_data;

create trigger trg_bump_sync_data_version
before update on public.sync_data
for each row
execute function public.bump_sync_data_version();
```

- [ ] **Step 2: 给 Supabase client 增加 HTTP 层回归测试**

在 `supabase.rs` 测试模块添加 fake HTTP client，覆盖：

```rust
#[tokio::test]
async fn update_sync_data_uses_id_and_version_filter() {
    let http = RecordingHttpClient::respond_json(
        200,
        r#"[{"id":"cloud-1","owner_id":"user-1","team_id":null,"data_type":"connection","encrypted_data":"enc","key_version":1,"checksum":"b","version":8,"updated_at":"2026-07-03T00:00:00Z","deleted_at":null}]"#,
    );
    let client = SupabaseClient::new(test_config(), Arc::new(http.clone()));
    client.set_auth("access".to_string(), "refresh".to_string(), "user-1".to_string());

    let mut data = test_cloud_sync_data();
    data.id = "cloud-1".to_string();
    data.version = 7;
    data.checksum = "b".to_string();
    let updated = client.update_sync_data(&data).await.expect("update succeeds");

    assert_eq!(8, updated.version);
    assert!(http.last_url().contains("id=eq.cloud-1"));
    assert!(http.last_url().contains("version=eq.7"));
}
```

如果现有 fake HTTP helper 不存在，新增最小 `RecordingHttpClient`，只实现 `send` 并记录 URL/method/body。

- [ ] **Step 3: 空响应必须继续识别为 conflict**

添加：

```rust
#[tokio::test]
async fn update_sync_data_empty_patch_response_is_conflict() {
    let http = RecordingHttpClient::respond_json(200, "[]");
    let client = SupabaseClient::new(test_config(), Arc::new(http));
    client.set_auth("access".to_string(), "refresh".to_string(), "user-1".to_string());

    let error = client
        .update_sync_data(&test_cloud_sync_data())
        .await
        .expect_err("empty response means version filter matched no rows");

    assert!(matches!(error, CloudApiError::Conflict(_)));
}
```

- [ ] **Step 4: 部署前验证规则**

在 release checklist 或部署说明中加入：

```sql
select tgname
from pg_trigger
where tgname = 'trg_bump_sync_data_version';
```

Expected: 返回 1 行。

- [ ] **Step 5: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync::supabase
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync
```

Expected: Supabase client tests pass；cloud sync 全量定向测试 pass。

---

## Task 6: 明确 OnetCloud soft delete 并发语义

**Files:**
- Modify: `crates/core/src/cloud_sync/client.rs`
- Modify: `crates/core/src/cloud_sync/supabase.rs`
- Modify: `crates/core/src/cloud_sync/connection_sync.rs`
- Modify: `crates/core/src/cloud_sync/generic_sync.rs`
- Optional Modify: `crates/core/src/storage/models.rs`
- Optional Modify: `crates/core/migrations/20260703000002_pending_deletion_version.sql`

### Option A: delete-wins，保持现状但写入文档

适用条件：产品接受“用户显式删除优先于并发更新”。  
修改方式：在 `CloudApiClient::delete_sync_data` 注释、`delete_connection` 和 `process_pending_deletions` 注释中明确 delete-wins，不增加本地 cloud version 字段。

需要新增测试：

```rust
#[test]
fn cloud_delete_policy_is_delete_wins() {
    assert_eq!("delete_wins", one_core::cloud_sync::delete_policy());
}
```

### Option B: strict optimistic delete，新增 versioned delete

适用条件：产品要求删除也不能覆盖并发更新。

- [ ] **Step 1: 扩展 CloudApiClient**

```rust
async fn delete_sync_data(
    &self,
    id: &str,
    expected_version: Option<u32>,
) -> Result<(), CloudApiError>;
```

所有调用点改为传 `None` 或 `Some(version)`。

- [ ] **Step 2: Supabase delete 使用 version filter**

```rust
let mut url = format!("{}?id=eq.{}", self.rest_url("sync_data"), id);
if let Some(version) = expected_version {
    url.push_str(&format!("&version=eq.{}", version));
}
let extra_headers = vec![("Prefer", "return=representation".to_string())];
let (status, result) = self
    .patch_json_with_retry::<Vec<SyncDataRow>, _>(&url, extra_headers, &payload)
    .await?;
if status.is_success() {
    let rows = result.map_err(CloudApiError::ParseError)?;
    if expected_version.is_some() && rows.is_empty() {
        return Err(CloudApiError::Conflict("删除版本冲突".to_string()));
    }
    return Ok(());
}
```

- [ ] **Step 3: pending deletion 存 version**

Migration:

```sql
ALTER TABLE pending_cloud_deletions ADD COLUMN cloud_version INTEGER;
```

Repository:

```rust
pub fn add_with_version(
    &self,
    cloud_id: &str,
    entity_type: &str,
    cloud_version: Option<u32>,
) -> Result<()> { ... }
```

- [ ] **Step 4: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
```

Expected: 所有 `delete_sync_data` 调用点编译通过；versioned delete conflict 有测试覆盖。

Recommendation: 先选 Option A 并文档化；只有在真实协作场景需要保护“删除 vs 更新”时再做 Option B，因为 Option B 会扩展本地 schema 和 API surface。

---

## Task 7: 明确并修复 workspace 冲突语义

**Files:**
- Modify: `crates/core/src/storage/models.rs`
- Modify: `crates/core/src/storage/repository.rs`
- Create: `crates/core/migrations/20260703000001_workspace_sync_metadata.sql`
- Modify: `crates/core/src/cloud_sync/generic_sync.rs`
- Modify: `crates/core/src/cloud_sync/workspace_sync.rs`
- Test: `crates/core/src/cloud_sync/generic_sync.rs`

### Option A: 接受 last-writer-wins

适用条件：workspace 只包含 name/color/icon，冲突成本低，产品接受较晚更新覆盖较早更新。  
修改方式：保留现状，在 `generic_sync.rs` 注释和用户文档中明确 workspace 是 last-writer-wins，不进入 conflict dialog。

### Option B: 给 workspace 增加连接同级冲突检测

适用条件：用户会多端频繁编辑 workspace，不能接受静默覆盖。

- [ ] **Step 1: 新增本地字段**

Migration:

```sql
ALTER TABLE workspaces ADD COLUMN last_synced_at INTEGER;
```

Model:

```rust
pub struct Workspace {
    ...
    pub last_synced_at: Option<i64>,
}

impl SyncableItem for Workspace {
    fn last_synced_at(&self) -> Option<i64> {
        self.last_synced_at
    }
}
```

- [ ] **Step 2: 更新 repository row mapping**

所有 workspace select 增加 `last_synced_at`：

```sql
SELECT id, name, color, icon, created_at, updated_at, cloud_id, last_synced_at FROM workspaces
```

`update_cloud_id` 改为：

```rust
pub fn update_sync_status(
    &self,
    local_id: i64,
    cloud_id: Option<String>,
    last_synced_at: Option<i64>,
) -> Result<()> {
    self.conn.with_connection(|conn| {
        conn.execute(
            "UPDATE workspaces SET cloud_id = ?1, last_synced_at = ?2 WHERE id = ?3",
            params![cloud_id, last_synced_at, local_id],
        )?;
        Ok(())
    })
}
```

- [ ] **Step 3: generic plan 增加 conflict**

新增 generic conflict model：

```rust
pub struct GenericSyncConflict<T: SyncableItem> {
    pub local: T,
    pub cloud: CloudSyncData,
    pub cloud_name: String,
}

pub struct GenericSyncPlan<T: SyncableItem> {
    ...
    pub conflicts: Vec<GenericSyncConflict<T>>,
}
```

在 `calculate_sync_plan` 中用 `last_synced_at` 判断：

```rust
let last_synced = local_item.last_synced_at().unwrap_or(0);
let local_changed = local_updated > last_synced;
let cloud_changed = cloud_updated > last_synced;
match (local_changed, cloud_changed) {
    (true, true) => plan.conflicts.push(GenericSyncConflict {
        local: local_item.clone(),
        cloud: (*cloud_data).clone(),
        cloud_name,
    }),
    (true, false) => plan.to_update_cloud.push((local_item.clone(), (*cloud_data).clone())),
    (false, true) => plan.to_update_local.push(((*cloud_data).clone(), local_item.clone())),
    (false, false) => {}
}
```

- [ ] **Step 4: 冲突处理策略**

先用保守策略：generic conflicts 不自动覆盖，加入 `result.errors` 并跳过该 record。

```rust
if !plan.conflicts.is_empty() {
    result.errors.extend(plan.conflicts.iter().map(|conflict| {
        format!(
            "{} '{}' 本地和云端均已修改，请先手动处理",
            type_name, conflict.cloud_name
        )
    }));
}
```

后续如需 UI conflict dialog，再把 workspace conflict 映射到可展示模型。

- [ ] **Step 5: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync
```

Expected: workspace both-modified 测试产生 conflict/error，不再静默覆盖。

Recommendation: 如果要快速降低风险，先做 Option A 文档化；如果 workspace 已经是强协作对象，做 Option B。

---

## Task 8: 确认并实现 `onetcli cloud` CLI

**Files:**
- Modify: `crates/onetcli_cli/src/lib.rs`
- Modify: `crates/onetcli_cli/src/tests.rs`
- Modify: `crates/onetcli_runtime/src/cli_host.rs`
- Optional Modify: `crates/onetcli_runtime/src/cli_host/domain.rs`

### Decision

如果“onetcli cloud同步”指 GUI 里的 OnetCloud hosted sync，不需要本任务。  
如果用户希望命令行执行 `onetcli cloud sync/status`，执行本任务。

- [ ] **Step 1: parser 增加 CloudCommand**

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum OnetCliCommand {
    Tool(ToolCommand),
    Connection(ConnectionCommand),
    Db(DbCommand),
    Ssh(SshCommand),
    Sftp(SftpCommand),
    Cloud(CloudCommand),
}

#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum CloudCommand {
    /// Print cloud sync status from local state.
    Status {
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
    /// Request a cloud sync.
    Sync {
        #[arg(long, default_value = "json")]
        format: OutputFormat,
    },
}
```

`CliCommand` 增加：

```rust
Cloud {
    #[command(subcommand)]
    command: CloudCommand,
},
```

`parse_from` match 增加：

```rust
CliCommand::Cloud { command } => OnetCliCommand::Cloud(command),
```

- [ ] **Step 2: parser tests**

```rust
#[test]
fn parses_cloud_status_command() {
    let parsed = parse_from(["onetcli", "cloud", "status", "--format", "json"]).unwrap();

    assert_eq!(
        Some(OnetCliCommand::Cloud(CloudCommand::Status {
            format: OutputFormat::Json,
        })),
        parsed
    );
}

#[test]
fn parses_cloud_sync_command() {
    let parsed = parse_from(["onetcli", "cloud", "sync", "--format", "json"]).unwrap();

    assert_eq!(
        Some(OnetCliCommand::Cloud(CloudCommand::Sync {
            format: OutputFormat::Json,
        })),
        parsed
    );
}
```

- [ ] **Step 3: runtime host 接入口**

在 `crates/onetcli_runtime/src/cli_host.rs`：

```rust
match command {
    ...
    onetcli_cli::OnetCliCommand::Cloud(command) => run_cloud_command(command, registry()?),
}
```

新增：

```rust
fn run_cloud_command(
    command: onetcli_cli::CloudCommand,
    registry: ToolRegistry,
) -> anyhow::Result<String> {
    match command {
        onetcli_cli::CloudCommand::Status { format } => {
            run_function_tool("onetcli.cloud.status", json!({}), false, registry, format)
        }
        onetcli_cli::CloudCommand::Sync { format } => {
            run_function_tool("onetcli.cloud.sync", json!({}), true, registry, format)
        }
    }
}
```

- [ ] **Step 4: 工具注册**

如果 `onetcli.cloud.status` / `onetcli.cloud.sync` 不存在，需要在 `main/src/public_mcp_runtime/tool_registry.rs` 或现有 registry builder 中注册。`status` 只能读取本地状态；`sync` 需要明确 headless 环境如何获取登录 token 和 master key。若不能安全获取，`sync` 返回结构化错误：

```json
{
  "ok": false,
  "error": "cloud sync requires an unlocked desktop session"
}
```

- [ ] **Step 5: 运行验证**

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p onetcli_cli
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p onetcli_runtime cloud
```

Expected: parser tests pass；runtime host dispatch tests pass；headless sync 不会误报成功。

---

## Final Verification Gate

完成所有选定任务后运行：

```bash
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core personal
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p one-core cloud_sync
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p main personal_sync
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p onetcli_cli
rtk env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo test -p onetcli_runtime
```

如果修改了 Supabase SQL，还必须在目标 Supabase 项目执行：

```sql
select tgname
from pg_trigger
where tgname = 'trg_bump_sync_data_version';
```

Expected: trigger 存在；普通 `sync_data` update 后 `version` 递增；version filter 不匹配时返回空 representation 并被客户端识别为 conflict。

---

## Acceptance Criteria

- 个人同步冲突不会静默成功；冲突会持久化，runtime 状态能体现失败或暂停。
- 同步进行中发生的个人同步事件不会丢失；当前 pass 完成后会自动补跑。
- 新个人同步目录启动 auto sync 后会创建并监听 `.onetcli-sync/records` 和 `.onetcli-sync/tombstones`。
- folder/Git personal backend 每次成功写入和 tombstone 都会推进 `version`。
- OnetCloud 普通 update 的 version 推进有 SQL 或 RPC 证据，且客户端测试覆盖 version filter。
- workspace 同步语义被明确：要么文档化 last-writer-wins，要么实现冲突检测。
- 若需要 `onetcli cloud` CLI，parser 和 runtime host 均有测试；若不需要，文档明确该名称指 GUI OnetCloud sync。

