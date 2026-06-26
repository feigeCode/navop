# Personal Sync Backends Design

## Summary

This design adds a personal, user-configurable sync backend for OnetCli connection data.
It keeps the existing Supabase cloud sync and team sync flow intact, while allowing users
to sync personal connections and workspaces through a local folder, an iCloud Drive or
OneDrive synced folder, or a local Git repository.

The chosen architecture is a file-backed sync store built on top of the existing
`CloudSyncData` encrypted record model. The application writes one encrypted sync record
per file, watches the sync directory for changes, and runs a background worker that
performs near-real-time sync with debounce, retry, and explicit conflict handling.

## Confirmed Product Decisions

- Version 1 targets personal sync only.
- Team data remains managed by the existing Supabase team sync flow.
- First-class v1 backends are:
  - folder sync, including user-selected iCloud Drive and OneDrive desktop sync folders.
  - Git sync through a user-selected local Git repository.
- Version 1 syncs only personal connections and personal workspaces.
- Application settings, login state, license state, MCP settings, team data, and team
  keys are out of scope.
- Automatic sync is enabled in near-real-time through a background worker.
- Record conflicts pause the affected record and require user choice before continuing.

## Current Implementation Context

The repository already has a cloud sync subsystem under `crates/core/src/cloud_sync`:

- `CloudSyncData` is the shared encrypted cloud record format.
- `CloudSyncService` encrypts and decrypts connection and workspace payloads.
- `SyncEngine` coordinates local storage, sync planning, conflict handling, and
  sync-type handlers.
- `CloudApiClient` currently mixes sync data operations with Supabase auth,
  subscription, team, and AI model responsibilities.
- `SupabaseClient` implements `CloudApiClient` through Supabase REST endpoints.

The new personal backend should not force folder, Git, WebDAV, or future OneDrive Graph
stores to implement Supabase-specific capabilities. The main structural change is to
extract a smaller sync-store interface that only knows how to store encrypted
`CloudSyncData` records.

## Goals

1. Let users choose their own personal sync location without depending on OnetCli's
   hosted Supabase service.
2. Reuse existing encrypted sync data, conflict detection, and connection/workspace
   handlers wherever possible.
3. Keep team sync and hosted account features stable.
4. Make folder sync, Git sync, and future WebDAV or OneDrive Graph sync share the same
   backend interface.
5. Keep conflicts isolated to individual records instead of one large shared file.
6. Avoid storing Git credentials, cloud provider tokens, or sensitive connection
   details in plaintext.

## Non-Goals

- Replacing Supabase auth, subscription, team, or AI model APIs.
- Supporting team sync through user-provided backends in v1.
- Implementing OneDrive OAuth, iCloud CloudKit, or WebDAV in v1.
- Syncing application settings, keybindings, MCP permissions, logs, license state, or
  local auth tokens.
- Creating a full collaborative permission model for file-backed sync stores.
- Guaranteeing that iCloud Drive or OneDrive has uploaded a local file to the cloud; v1
  can only guarantee that OnetCli wrote the local sync package.

## Architecture

Add a personal sync layer beside the existing hosted cloud sync flow:

```text
Local storage
  connections / workspaces
        |
        v
Existing sync type handlers
        |
        v
CloudSyncData encrypted records
        |
        +--> Existing Supabase CloudApiClient for hosted/team sync
        |
        +--> New PersonalSyncStore for personal folder/Git sync
```

The personal path introduces these core components:

- `PersonalSyncStore`: minimal backend interface for encrypted sync records.
- `DirectorySyncStore`: stores records in a user-selected directory.
- `GitSyncStore`: wraps a directory store and runs Git pull/commit/push around sync.
- `PersonalSyncWorker`: background event processor for local changes, remote file
  changes, startup scans, retry, and conflict pauses.
- `PersonalSyncSettings`: persisted application setting for backend type, path, Git
  options, and automatic sync enablement.
- `PersonalSyncStatus`: runtime state for UI display.
- `PersonalSyncConflictRepository`: local persistence for paused record conflicts.

Team records are filtered out by `team_id != None`. A personal backend never writes team
records, team keys, or team membership metadata.

## Sync Store Interface

The store interface should be narrower than `CloudApiClient`:

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

Interface semantics:

- `probe` validates configuration and returns user-facing status.
- `list_records` may ignore `since` and return a full listing.
- `upsert_record` uses `expected_version` for optimistic concurrency when available.
- `tombstone_record` records deletion without immediately removing history.
- `acquire_lock` is a best-effort lock, not the only source of consistency.
- Version, checksum, updated timestamp, and tombstones remain the real conflict boundary.

## File Layout

The user selects a folder or Git repository root. OnetCli creates a hidden sync package:

```text
<user-selected-dir>/
  .onetcli-sync/
    manifest.json
    records/
      workspace/
        <cloud_id>.json
      connection/
        <cloud_id>.json
    tombstones/
      <cloud_id>.json
    state/
      device.json
      last-sync.json
    lock
```

`manifest.json` stores protocol metadata:

```json
{
  "schema_version": 1,
  "app": "onetcli",
  "profile_id": "personal",
  "created_at": 1790000000000,
  "updated_at": 1790000000000
}
```

Each record file stores one `CloudSyncData`:

```json
{
  "id": "uuid",
  "owner_id": "local-personal-user",
  "team_id": null,
  "data_type": "connection",
  "encrypted_data": "base64(...)",
  "key_version": 1,
  "checksum": "sha256...",
  "version": 3,
  "updated_at": 1790000000000,
  "deleted_at": null
}
```

Deletion writes a tombstone instead of immediately removing all state. Tombstones contain
at least:

- `id`
- `data_type`
- `deleted_at`
- `version`
- `checksum`

Version 1 does not automatically prune tombstones. Retention can be added later after
older-client behavior is understood.

## Directory Backend

`DirectorySyncStore` supports any local folder:

- local filesystem directory.
- iCloud Drive folder.
- OneDrive folder.
- Dropbox or similar desktop sync folder.

Behavior:

- If `.onetcli-sync` is missing, first enablement initializes it.
- If `schema_version` is newer than the app supports, sync is blocked with
  `SchemaUnsupported`.
- Record writes use temp-file plus rename to reduce partial-write risk.
- Directory watcher observes `records/` and `tombstones/`.
- Watcher events are debounced and fed into `PersonalSyncWorker`.

The application does not infer whether iCloud Drive or OneDrive has completed remote
upload. UI should state local write status, not cloud-provider completion status.

## Git Backend

`GitSyncStore` treats a local Git repository as the sync folder.

Required behavior:

- User selects an existing Git repository root.
- `.onetcli-sync` is stored inside that repository.
- Sync starts with `git pull --rebase`.
- After record changes, the backend runs:
  - `git add .onetcli-sync`
  - `git commit -m "onetcli sync: update personal records"`
  - `git push` when automatic push is enabled.
- If automatic push is disabled, the backend commits locally and reports a pending push
  state.
- The app does not save Git passwords, tokens, SSH keys, or credential-helper secrets.
- Git author identity comes from the user's existing Git configuration.

Error classification:

- no repository: `NotConfigured` or `DirectoryUnavailable`.
- no remote: sync can commit locally, but push status is disabled or warning.
- authentication failure: `GitAuthRequired`.
- merge conflict: `GitMergeConflict`; pause the whole Git backend until the repo is
  clean again.
- dirty working tree outside `.onetcli-sync`: warn but do not modify unrelated files.

The Git runner should be injectable so tests can cover pull, commit, push, auth failure,
and merge conflict without depending on real network remotes.

## Near-Real-Time Worker

`PersonalSyncWorker` serializes all automatic sync work. It should never run two sync
transactions at the same time.

Event sources:

```text
Local connection/workspace save or delete
  -> LocalChanged(data_type, local_id)

.onetcli-sync record or tombstone file change
  -> RemoteChanged(path)

Application startup, backend switch, retry, explicit sync
  -> FullScan
```

Scheduling:

- Events go into one queue.
- Debounce is 500-1000 ms.
- While a sync transaction is running, new events mark the worker dirty.
- When the current transaction finishes, the worker runs another pass if dirty.
- Queue entries for the same record can be coalesced.
- Failed store operations use exponential backoff.
- After a configured failure threshold, the backend is paused until the user resumes it.

Sync transaction:

```text
1. acquire best-effort store lock
2. Git backend runs pull --rebase
3. load local personal workspaces and connections
4. load remote records and tombstones
5. filter out team records
6. calculate upload, download, update, delete, and conflict actions
7. write store records/tombstones or update local repositories
8. Git backend commits and optionally pushes
9. update local sync state and status
10. release lock
```

Loop prevention:

- Local changes written by the worker carry a `source = personal_sync_worker` marker so
  local-change listeners can ignore them.
- Store writes remember touched paths during a short time window, allowing file watcher
  callbacks to ignore self-written events.
- Even if a self-written event is not filtered, checksum and version comparison should
  make the second pass a no-op.

## Conflict Handling

Record conflicts pause only the affected record.

Conflict detection:

- Local modified since last sync and remote modified since last sync with different
  checksum.
- Local delete and remote modify.
- Local modify and remote tombstone.
- Matching checksum with different timestamps is not a conflict; update sync metadata.

Conflict persistence should live in local storage, for example a
`personal_sync_conflicts` table:

```text
record_id TEXT NOT NULL
data_type TEXT NOT NULL
backend_profile_id TEXT NOT NULL
conflict_type TEXT NOT NULL
local_snapshot TEXT
remote_snapshot TEXT
detected_at INTEGER NOT NULL
PRIMARY KEY (backend_profile_id, record_id)
```

User resolution options:

- use local version.
- use remote version.
- keep both.

Resolution behavior:

- Use local version: upsert the local encrypted record to the personal store.
- Use remote version: decrypt and update the local record.
- Keep both: keep the remote record and create a local copy with a new local ID and no
  previous cloud ID, then sync the copy as a new record.
- After resolution, remove the paused status and enqueue a full scan.

Git merge conflicts are backend-level conflicts, not record conflicts. The worker pauses
the whole Git backend and asks the user to make the repository clean.

## Settings And UI

Add a Sync page or Sync group in existing settings. Keep it visually separate from
Supabase login/team sync.

Controls:

- enable personal sync switch.
- backend segmented control:
  - Folder
  - Git
- folder or repository path input with picker button.
- automatic sync switch, default enabled.
- test connection button.
- sync now button.
- latest sync status.
- conflict list entry shown only when conflicts exist.

Git-only controls:

- repository status.
- remote URL read-only display.
- automatic push switch, default enabled.
- pending local commits or push failure status.

Displayed states:

- `NotConfigured`
- `DirectoryUnavailable`
- `SchemaUnsupported`
- `LockTimeout`
- `GitAuthRequired`
- `GitMergeConflict`
- `RecordConflict`
- `PausedAfterRepeatedFailures`
- `Synced`
- `Syncing`
- `PendingRetry`

The UI should avoid claiming that iCloud Drive or OneDrive remote upload is complete.
For folder sync, the success state means OnetCli wrote the local sync package.

## Security

- Connection and workspace payloads remain encrypted using the existing
  `CloudSyncService` model.
- Git credentials are never stored by OnetCli.
- Future WebDAV or OneDrive OAuth credentials must use an isolated credential store, not
  `settings.json` and not `.onetcli-sync`.
- Visible metadata includes file names, data type, timestamps, schema version, and
  deletion markers.
- Sensitive connection details such as hosts, usernames, passwords, private keys, and
  database names must not appear in plaintext sync files.
- The UI should warn users before using a public Git repository.
- Team records and team keys are skipped by the personal backend.

## Persistence

Settings can live in `AppSettings`:

```text
personal_sync.enabled
personal_sync.backend
personal_sync.path
personal_sync.auto_sync
personal_sync.git.auto_push
```

Runtime and durable sync state should not be stored only in memory:

- paused conflicts need a local table.
- retry state should survive restarts when practical.
- last successful sync status should be persisted for settings UI display.
- per-record sync metadata can reuse existing `cloud_id`, `last_synced_at`, and
  checksum/version fields where possible, but personal backend state must not corrupt
  Supabase team sync semantics.

If existing fields are too coupled to hosted cloud sync, add a separate
`personal_sync_state` table keyed by local entity ID and backend profile ID.

## Future Extensions

The same `PersonalSyncStore` can later support:

- WebDAV or Nextcloud.
- OneDrive Graph OAuth.
- iCloud CloudKit, if a native provider is added.
- multiple personal profiles.
- setting sync as a new `CloudSyncData.data_type`.
- tombstone pruning after compatibility rules are defined.

Future network stores should not require changes to connection/workspace sync handlers.
They should only implement the store interface and credential configuration UI.

## Test Strategy

Unit tests:

- serialize and deserialize `CloudSyncData` record files.
- initialize and validate `manifest.json`.
- reject unsupported schema versions.
- write records atomically through `DirectorySyncStore`.
- classify tombstones correctly.
- detect checksum/version conflicts.
- skip team records in personal sync.

Integration tests:

- simulate two devices with temporary directories.
- device A writes a record and device B imports it by full scan.
- both devices modify the same record and produce a paused conflict.
- local delete versus remote modify produces a conflict.
- worker writes do not trigger infinite sync loops.
- Git runner mock covers pull, commit, push, auth failure, no remote, and merge conflict.

UI/state tests:

- settings page renders unconfigured, syncing, synced, retry, paused, and conflict states.
- conflict count appears only when conflicts exist.
- Git automatic push toggle changes backend behavior.

Expected verification commands for implementation:

```bash
cargo test -p one-core cloud_sync
cargo check -p main
```

If UI code changes, add focused GPUI state tests or view tests where feasible.

## Rollout Plan

1. Add personal sync settings and local status models.
2. Extract `PersonalSyncStore` and implement `DirectorySyncStore`.
3. Add file format tests for manifest, records, tombstones, and schema checks.
4. Add `PersonalSyncWorker` with debounce, single-flight execution, retry, and conflict
   pause semantics.
5. Wire local connection/workspace changes into the worker.
6. Add settings UI and status display.
7. Add `GitSyncStore` with an injectable Git runner.
8. Add conflict list and resolution actions.
9. Run targeted core tests and main crate checks.

## Open Risks

- Existing `CloudApiClient` mixes hosted service features with sync data access. The
  implementation should avoid extending that trait for personal stores.
- Existing sync state fields may be too Supabase-oriented for multiple personal
  backends. A separate state table may be cleaner than overloading current columns.
- iCloud Drive and OneDrive file synchronization timing is outside the app's control.
- Git automatic sync must be conservative around dirty repos and merge conflicts.
- Near-real-time sync increases the need for deterministic worker tests.
