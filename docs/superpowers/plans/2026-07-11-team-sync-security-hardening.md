# Team Sync Security Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden team encryption, cache scoping, membership reconciliation, permission checks, key-state reporting, and cloud deletion confirmation without changing the deferred Web URL token handoff.

**Architecture:** Introduce a versioned Argon2id envelope that wraps a random team data key while retaining a legacy reader. Move team cache persistence and reconciliation into focused modules keyed by cloud environment and user, then expose separate persistent-cache and runtime-load statuses to UI consumers. Keep the existing transactional rotation RPC and AES-GCM record format, but require confirmed affected rows for cloud soft deletion.

**Tech Stack:** Rust 2024, Argon2id, AES-256-GCM, serde/JSON/Base64, rusqlite migrations, Supabase/PostgREST, GPUI, Tokio, cargo test/check/clippy.

---

## 2026-07-12 approved scope amendment: V2 only

The product has not shipped. This amendment overrides every legacy-compatibility or legacy-upgrade instruction below:

- Delete `TeamKeyScheme` and all legacy detection/fallback behavior.
- `unlock_team_key` accepts only `TEAMKEY2:` envelopes; non-V2 input returns `InvalidKeyOrEnvelope`.
- Use `TeamKeyCacheStatus::{Missing, Cached, VersionMismatch, Invalid}`.
- Use `TeamKeyLoadStatus::{Unlocked, Missing, VersionMismatch, Invalid}`.
- Initialization and every rotation require the 12-character password policy.
- Rotation always unwraps a V2 envelope and generates a fresh envelope plus random data key. Identical old/new passphrases are allowed but do not reuse the data key.
- Remove legacy UI badges, upgrade actions, compatibility warnings, tests, and locale keys. Invalid/missing verification is reinitialized by owner/admin; members see it as unavailable.
- Keep legacy handling only where unrelated to team encryption, such as personal master-key verification or old connection payload deserialization.

Before Task 4, add a TDD cleanup checkpoint:

1. Replace envelope tests with non-V2 rejection tests and verify RED against the existing fallback.
2. Remove `TeamKeyScheme`, legacy unlock, `LegacyNeedsUpgrade`, and `LegacyUnlocked`.
3. Update manager/engine matches and tests to strict V2 or `Invalid`.
4. Run `rtk cargo test -p one-core team_key --lib -- --test-threads=1` and require all tests to pass.
5. Commit as `refactor(sync): require v2 team key envelopes`.

All later tasks must follow this amendment even where older examples below mention legacy behavior.

## File map

- Create `crates/core/src/cloud_sync/team_key_envelope.rs`: versioned envelope creation, parsing, legacy unlock, password policy.
- Create `crates/core/src/cloud_sync/team_key_envelope_tests.rs`: cryptographic behavior tests with low-cost injected KDF parameters.
- Create `crates/core/src/cloud_sync/team_scope.rs`: `CloudAccountScope` normalization and current-account helpers.
- Create `crates/core/src/storage/team_key_cache.rs`: scoped cache model and repository extracted from `repository.rs`.
- Create `crates/core/src/storage/team_key_cache_tests.rs`: migration, isolation, and CRUD tests.
- Create `crates/core/src/cloud_sync/team_cache.rs`: `SyncEngine` team refresh/reconcile helpers extracted from `engine.rs`.
- Create `crates/core/src/cloud_sync/team_cache_tests.rs`: membership failure, removal, role, and stale-version tests.
- Create `crates/core/src/cloud_sync/team_key_manager_tests.rs`: move and extend manager tests outside the production file.
- Create `main/src/settings/team_keys.rs`: team-key settings UI and pure UI decision helpers extracted from `setting_tab.rs`.
- Create `crates/core/migrations/20260711000001_scoped_team_key_cache.sql`: destructive migration of unsafe unscoped key cache only.
- Modify core cloud-sync/service/client/Supabase modules and team selector consumers.

### Task 1: Add the Argon2id team-key envelope

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/core/src/cloud_sync/mod.rs`
- Create: `crates/core/src/cloud_sync/team_key_envelope.rs`
- Create: `crates/core/src/cloud_sync/team_key_envelope_tests.rs`

- [ ] **Step 1: Add failing envelope tests**

Create tests for V2 detection, random envelope salt/data keys, correct unlock, wrong password, tampering, legacy unlock, and minimum password length. Use the wished-for API:

```rust
let params = TeamKeyKdfParams::for_tests();
let first = create_team_key_envelope("correct horse battery staple", params).unwrap();
let second = create_team_key_envelope("correct horse battery staple", params).unwrap();
assert_ne!(first.verification, second.verification);
assert_ne!(first.data_key, second.data_key);
assert_eq!(
    first.data_key,
    unlock_team_key(&first.verification, "correct horse battery staple").unwrap().data_key
);
assert_eq!(TeamKeyScheme::EnvelopeV2, detect_team_key_scheme(&first.verification));
```

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core team_key_envelope --lib`

Expected: compilation fails because `TeamKeyKdfParams`, `create_team_key_envelope`, `unlock_team_key`, and `TeamKeyScheme` do not exist.

- [ ] **Step 3: Add dependencies and implement the envelope**

Add `argon2 = "0.5.3"` to workspace dependencies and `argon2.workspace = true` to `one-core`. Implement these exact public types:

```rust
pub const TEAM_KEY_ENVELOPE_PREFIX: &str = "TEAMKEY2:";
pub const MIN_NEW_TEAM_KEY_CHARS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamKeyScheme { Legacy, EnvelopeV2 }

#[derive(Debug, Clone, Copy)]
pub struct TeamKeyKdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTeamKeyEnvelope {
    pub verification: String,
    pub data_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockedTeamKey {
    pub scheme: TeamKeyScheme,
    pub data_key: String,
}
```

Production parameters are 65,536 KiB, 3 iterations, parallelism 1. Serialize a private `TeamKeyEnvelopeV2` as Base64 JSON after `TEAMKEY2:`. Generate independent 16-byte salt, 12-byte nonce, and 32-byte data key; derive the wrapping key with `Argon2::hash_password_into`; wrap `b"NAVOP_TEAM_KEY_V2\0" + data_key` with AES-GCM. Legacy unlock calls `crypto::verify_master_key` and returns the passphrase as `data_key`.

- [ ] **Step 4: Verify GREEN and tamper handling**

Run: `rtk cargo test -p one-core team_key_envelope --lib`

Expected: all envelope tests pass; wrong password and any modified salt/nonce/ciphertext return `TeamKeyEnvelopeError::InvalidKeyOrEnvelope` without leaking cryptographic details.

- [ ] **Step 5: Commit**

```bash
rtk git add Cargo.toml Cargo.lock crates/core/Cargo.toml crates/core/src/cloud_sync/mod.rs crates/core/src/cloud_sync/team_key_envelope.rs crates/core/src/cloud_sync/team_key_envelope_tests.rs
rtk git commit -m "feat(sync): add versioned team key envelope"
```

### Task 2: Scope and migrate the local team-key cache

**Files:**
- Create: `crates/core/src/cloud_sync/team_scope.rs`
- Create: `crates/core/migrations/20260711000001_scoped_team_key_cache.sql`
- Modify: `crates/core/src/storage/migration.rs`
- Modify: `crates/core/src/storage/mod.rs`
- Modify: `crates/core/src/storage/repository.rs`
- Create: `crates/core/src/storage/team_key_cache.rs`
- Create: `crates/core/src/storage/team_key_cache_tests.rs`

- [ ] **Step 1: Write failing scope/repository tests**

Define the required API in tests:

```rust
let alice_prod = CloudAccountScope::new("https://project.supabase.co/", "alice");
let bob_prod = CloudAccountScope::new("https://project.supabase.co", "bob");
let alice_stage = CloudAccountScope::new("https://stage.supabase.co", "alice");
repo.upsert(&cache(&alice_prod, "team-1", 4, Some(4))).unwrap();
repo.upsert(&cache(&bob_prod, "team-1", 4, Some(4))).unwrap();
assert_eq!(1, repo.list(&alice_prod).unwrap().len());
assert_eq!(1, repo.list(&bob_prod).unwrap().len());
assert!(repo.list(&alice_stage).unwrap().is_empty());
```

Add a migration test that seeds the old unscoped table, runs migrations, and proves the unsafe old secret is gone while the new composite primary key accepts duplicate `team_id` values in different scopes.

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core team_key_cache --lib`

Expected: compilation fails because scoped repository APIs and the migration are absent.

- [ ] **Step 3: Implement scope and migration**

Implement:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloudAccountScope {
    pub environment: String,
    pub user_id: String,
}

impl CloudAccountScope {
    pub fn new(environment: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            environment: environment.into().trim().trim_end_matches('/').to_string(),
            user_id: user_id.into(),
        }
    }
}
```

The new table uses `PRIMARY KEY (cloud_environment, user_id, team_id)` and adds nullable `cached_key_version`. Rename the old table, create the new table, and drop the old table without copying secrets. Move `TeamKeyCache` and `TeamKeyCacheRepository` into `storage/team_key_cache.rs`. Make `get/list/delete` require `&CloudAccountScope`; `upsert` reads scope from `TeamKeyCache`.

- [ ] **Step 4: Verify GREEN**

Run: `rtk cargo test -p one-core team_key_cache --lib`

Expected: scoped CRUD and destructive legacy-cache migration tests pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/migrations/20260711000001_scoped_team_key_cache.sql crates/core/src/cloud_sync/team_scope.rs crates/core/src/storage
rtk git commit -m "feat(sync): scope team key cache by cloud account"
```

### Task 3: Separate cache state from runtime unlock state

**Files:**
- Modify: `crates/core/src/cloud_sync/team_key_manager.rs`
- Create: `crates/core/src/cloud_sync/team_key_manager_tests.rs`
- Modify: `crates/core/src/cloud_sync/service.rs`
- Modify: `crates/core/src/cloud_sync/mod.rs`

- [ ] **Step 1: Move existing tests and add failing state tests**

Move the current inline manager tests unchanged to `team_key_manager_tests.rs`, then add:

```rust
assert_eq!(
    TeamKeyCacheStatus::VersionMismatch,
    cache_status(&cache_with_versions(4, Some(3)), TeamKeyScheme::EnvelopeV2)
);
assert_eq!(
    TeamKeyCacheStatus::LegacyNeedsUpgrade,
    cache_status(&cache_with_versions(4, Some(4)), TeamKeyScheme::Legacy)
);
assert_eq!(
    TeamKeyLoadStatus::LegacyUnlocked,
    manager.load_cached_team_key(&legacy_team, &scope, personal_key).unwrap()
);
```

Also assert that loading V2 stores the unwrapped random data key in `CloudSyncService`, while the local cache stores only the personal-key-encrypted passphrase.

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core team_key_manager --lib`

Expected: compilation fails because the two status enums, scope parameters, and V2 unlock path are absent.

- [ ] **Step 3: Implement the manager contract**

Replace `TeamKeyStatus` with:

```rust
pub enum TeamKeyCacheStatus { Missing, Cached, VersionMismatch, LegacyNeedsUpgrade }
pub enum TeamKeyLoadStatus { Unlocked, LegacyUnlocked, Missing, VersionMismatch }
```

All manager methods accept `&CloudAccountScope`. `save_verified_team_key` unlocks verification via `unlock_team_key`, encrypts the entered passphrase with the personal key, records `cached_key_version = Some(team.key_version)`, and stores `unlocked.data_key` in `CloudSyncService`. `load_cached_team_key` checks cached vs remote version before decrypting the passphrase. `forget_team_key` clears the secret and verification timestamp but preserves metadata and scope.

- [ ] **Step 4: Verify GREEN and existing regressions**

Run: `rtk cargo test -p one-core team_key_manager --lib -- --test-threads=1`

Expected: existing save/load/forget/version tests and new legacy/V2 state tests pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/src/cloud_sync/team_key_manager.rs crates/core/src/cloud_sync/team_key_manager_tests.rs crates/core/src/cloud_sync/service.rs crates/core/src/cloud_sync/mod.rs
rtk git commit -m "refactor(sync): separate team key cache and load states"
```

### Task 4: Initialize, upgrade, and rotate with random data keys

**Files:**
- Modify: `crates/core/src/cloud_sync/team_key_manager.rs`
- Modify: `crates/core/src/cloud_sync/team_key_manager_tests.rs`
- Modify: `crates/core/src/cloud_sync/engine.rs`
- Test: `crates/core/src/cloud_sync/team_key_manager_tests.rs`

- [ ] **Step 1: Add failing initialization and rotation tests**

Cover new-team V2 initialization, V2-to-V2 rotation, legacy-to-V2 same-passphrase upgrade, legacy-to-V2 new-passphrase rotation, and password policy:

```rust
let rotation = TeamKeyManager::rotate_team_key_records(
    &legacy_team,
    "short",
    "short",
    &records,
    TeamKeyKdfParams::for_tests(),
).unwrap();
assert_eq!(TeamKeyScheme::EnvelopeV2, detect_team_key_scheme(rotation.team.key_verification.as_deref().unwrap()));
assert!(crypto::decrypt_with_key(&rotation.records[0].encrypted_data, "short").is_err());
assert_eq!("{}", crypto::decrypt_with_key(&rotation.records[0].encrypted_data, &rotation.data_key).unwrap());
```

Assert that a new V2 team rejects fewer than 12 characters, but a same-passphrase legacy upgrade accepts the existing short key.

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core rotate_team_key --lib`

Expected: legacy same-key upgrade is rejected by current code and V2 verification/data-key assertions fail.

- [ ] **Step 3: Implement preparation and rotation**

Add `PreparedTeamKey { team, data_key }` and extend `TeamKeyRotation` with `pub data_key: String`. New initialization calls `create_team_key_envelope`; rotation unlocks the old verification to get the old data key, creates a new V2 envelope/data key, and re-encrypts every non-empty record from old data key to new data key. Permit identical old/new passphrases only when the old scheme is legacy. Keep the RPC payload and optimistic versions unchanged.

- [ ] **Step 4: Verify GREEN**

Run: `rtk cargo test -p one-core team_key --lib -- --test-threads=1`

Expected: all team-key tests pass, including atomic record re-encryption and password-policy cases.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/src/cloud_sync/team_key_manager.rs crates/core/src/cloud_sync/team_key_manager_tests.rs crates/core/src/cloud_sync/engine.rs
rtk git commit -m "feat(sync): upgrade team encryption to wrapped data keys"
```

### Task 5: Reconcile teams and invalidate stale keys

**Files:**
- Create: `crates/core/src/cloud_sync/team_cache.rs`
- Create: `crates/core/src/cloud_sync/team_cache_tests.rs`
- Modify: `crates/core/src/cloud_sync/engine.rs`
- Modify: `crates/core/src/cloud_sync/mod.rs`
- Modify: `crates/core/src/cloud_sync/client.rs`
- Modify: `crates/core/src/cloud_sync/supabase.rs`

- [ ] **Step 1: Add failing reconciliation tests**

Add fake-client cases proving: removed teams are deleted only within the current scope; the same team ID in another user/environment remains; remote key-version changes clear secret/verification timestamp while retaining old `cached_key_version`; member-list failure preserves the old row; removed teams are also removed from `CloudSyncService`.

```rust
let result = engine.refresh_team_key_cache().await.unwrap();
assert_eq!(1, result.cached);
assert_eq!(1, result.removed);
assert!(repo.get(&alice_scope, "removed-team").unwrap().is_none());
assert!(repo.get(&bob_scope, "removed-team").unwrap().is_some());
assert!(!service.read().unwrap().is_team_unlocked("removed-team"));
```

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core team_cache --lib`

Expected: compilation fails because refresh returns only `usize`, no reconcile module exists, and repository calls are unscoped.

- [ ] **Step 3: Implement account/environment identity and reconciliation**

Add object-safe `fn environment_id(&self) -> &str` to `CloudApiClient`; Supabase returns normalized `project_url`, test fakes use `"test://cloud"`. Add `SyncEngine::account_scope()` from client environment plus logged-in user ID. Move refresh helpers into `team_cache.rs` and return:

```rust
pub struct TeamCacheRefreshResult { pub cached: usize, pub removed: usize }
```

Only delete `local - remote` after `list_teams` succeeds. A listed team whose member query fails keeps its prior row. `team_key_cache_for_cloud_team` clears the secret when `cached_key_version != team.key_version` and records the new remote metadata.

- [ ] **Step 4: Verify GREEN**

Run: `rtk cargo test -p one-core team_cache --lib -- --test-threads=1`

Expected: removal, isolation, member-failure, stale-version, and runtime-key tests pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/src/cloud_sync/team_cache.rs crates/core/src/cloud_sync/team_cache_tests.rs crates/core/src/cloud_sync/engine.rs crates/core/src/cloud_sync/mod.rs crates/core/src/cloud_sync/client.rs crates/core/src/cloud_sync/supabase.rs
rtk git commit -m "fix(sync): reconcile scoped team membership and stale keys"
```

### Task 6: Enforce scoped permissions and selectors

**Files:**
- Modify: `crates/core/src/cloud_sync/mod.rs`
- Modify: `crates/core/src/settings.rs`
- Modify: `crates/core/src/config.rs`
- Modify: `main/src/home_tab.rs`
- Modify: `main/src/new_connection/form_page.rs`
- Modify: `main/src/home/connection_import_window/editor.rs`
- Modify team selector consumers under `crates/db_view`, `crates/redis_view`, `crates/mongodb_view`, `crates/terminal_view`, `crates/remote_desktop_view`, and `crates/port_forwarding_view`.

- [ ] **Step 1: Add failing global-scope and permission tests**

Test `GlobalCloudUser::get_scope`, `get_cached_team_options` filtering, and the owner bypass regression:

```rust
assert!(!can_edit_team_connection_for_scope(
    &connection_owned_by_alice,
    &bob_scope,
    Some(&alice_admin_cache),
));
assert!(can_edit_team_connection_for_scope(
    &connection_owned_by_alice,
    &alice_scope,
    Some(&alice_member_cache),
));
```

The second assertion remains true because the creator may edit only after the current scope has confirmed team access.

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core cloud_sync::tests --lib`

Expected: tests fail because globals and permission helpers do not carry `CloudAccountScope`.

- [ ] **Step 3: Implement scoped globals and UI helpers**

Preserve `GlobalCloudUser::get_user`, add `get_scope`, and derive scope during `GlobalCurrentUser::set_user` from `SupabaseConfig::get().project_url` and user ID. `get_cached_team_options`, save/forget/ensure helpers, and `can_edit_connection` require the current scope. Check accessible team cache before owner/admin rules. Replace selector matches with `TeamKeyCacheStatus`; `LegacyNeedsUpgrade` remains selectable like `Cached`, while `Missing` and `VersionMismatch` show unavailable status.

- [ ] **Step 4: Verify GREEN and compile all consumers**

Run:

```bash
rtk cargo test -p one-core cloud_sync::tests --lib
rtk cargo check -p main
```

Expected: permission tests pass and all form crates compile through `main`.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/src/cloud_sync/mod.rs crates/core/src/settings.rs crates/core/src/config.rs main/src/home_tab.rs main/src/new_connection main/src/home/connection_import_window crates/db_view crates/redis_view crates/mongodb_view crates/terminal_view crates/remote_desktop_view crates/port_forwarding_view
rtk git commit -m "fix(sync): isolate team permissions and selectors by account"
```

### Task 7: Confirm affected rows for cloud soft deletion

**Files:**
- Modify: `crates/core/src/cloud_sync/supabase.rs`
- Modify: `crates/core/src/cloud_sync/connection_sync.rs`
- Modify: `crates/core/src/cloud_sync/generic_sync.rs`

- [ ] **Step 1: Add failing fake-HTTP tests**

Extend `RecordingHttpClient` to capture headers. Add a concrete helper and response-cardinality tests:

```rust
fn delete_client(body: &'static str) -> (SupabaseClient, Arc<RecordingHttpClient>) {
    let http = Arc::new(RecordingHttpClient::respond_json(200, body));
    let client = SupabaseClient::new(test_config(), http.clone());
    client.set_auth("access".into(), "refresh".into(), "user-1".into());
    (client, http)
}

let one_row = r#"[{"id":"cloud-1","owner_id":"user-1","team_id":null,"data_type":"connection","encrypted_data":"enc","key_version":1,"checksum":"a","version":7,"updated_at":"2026-07-03T00:00:00Z","deleted_at":"2026-07-11T00:00:00Z"}]"#;
let two_rows = r#"[{"id":"cloud-1","owner_id":"user-1","team_id":null,"data_type":"connection","encrypted_data":"enc","key_version":1,"checksum":"a","version":7,"updated_at":"2026-07-03T00:00:00Z","deleted_at":"2026-07-11T00:00:00Z"},{"id":"cloud-2","owner_id":"user-1","team_id":null,"data_type":"connection","encrypted_data":"enc","key_version":1,"checksum":"b","version":7,"updated_at":"2026-07-03T00:00:00Z","deleted_at":"2026-07-11T00:00:00Z"}]"#;
let (client, http) = delete_client(one_row);
assert!(client.delete_sync_data("cloud-1").await.is_ok());
assert!(matches!(
    delete_client("[]").0.delete_sync_data("cloud-1").await,
    Err(CloudApiError::Conflict(_))
));
assert!(matches!(
    delete_client(two_rows).0.delete_sync_data("cloud-1").await,
    Err(CloudApiError::ParseError(_))
));
assert_eq!(Some("return=representation"), http.last_header("Prefer"));
```

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p one-core delete_sync_data --lib`

Expected: current delete parses arbitrary JSON, sends no Prefer header, and accepts an empty response.

- [ ] **Step 3: Implement strict delete response handling**

Patch with `Prefer: return=representation`, deserialize `Vec<SyncDataRow>`, accept exactly one row, map zero rows to `CloudApiError::Conflict("删除目标不存在或当前账号无权限".into())`, and map multiple rows to `ParseError`. Do not remove pending deletion records on these errors; existing connection/generic sync retry behavior remains active.

- [ ] **Step 4: Verify GREEN**

Run: `rtk cargo test -p one-core delete_sync_data --lib`

Expected: all three response cardinality cases and pending-deletion retention pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/core/src/cloud_sync/supabase.rs crates/core/src/cloud_sync/connection_sync.rs crates/core/src/cloud_sync/generic_sync.rs
rtk git commit -m "fix(sync): require confirmed cloud soft deletion"
```

### Task 8: Extract and update team-key settings UI

**Files:**
- Create: `main/src/settings/team_keys.rs`
- Modify: `main/src/settings/mod.rs`
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`

- [ ] **Step 1: Add failing pure UI decision tests**

Define and test:

```rust
assert_eq!(TeamKeyActionLabel::Upgrade, action_label(TeamKeyCacheStatus::LegacyNeedsUpgrade));
assert!(same_passphrase_allowed(TeamKeyScheme::Legacy, "short", "short"));
assert!(!same_passphrase_allowed(TeamKeyScheme::EnvelopeV2, "new secure key", "new secure key"));
assert!(!can_manage_remote_key(Some("member")));
assert!(can_manage_remote_key(Some("admin")));
```

- [ ] **Step 2: Verify RED**

Run: `rtk cargo test -p main team_key -- --nocapture`

Expected: helpers and the extracted settings module do not exist.

- [ ] **Step 3: Extract UI and implement legacy upgrade behavior**

Move team-key rendering/dialog/async helpers from `setting_tab.rs` to `settings/team_keys.rs`. Implement the tested decisions exactly:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamKeyActionLabel { Rotate, Upgrade }

fn action_label(status: TeamKeyCacheStatus) -> TeamKeyActionLabel {
    if status == TeamKeyCacheStatus::LegacyNeedsUpgrade {
        TeamKeyActionLabel::Upgrade
    } else {
        TeamKeyActionLabel::Rotate
    }
}

fn same_passphrase_allowed(scheme: TeamKeyScheme, old: &str, new: &str) -> bool {
    scheme == TeamKeyScheme::Legacy && old == new
}

fn can_manage_remote_key(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}
```

Add `LegacyNeedsUpgrade` badge and an upgrade label/help/compatibility warning. Permit identical old/new text only for legacy upgrade. Keep new-team/new-password 12-character validation in core and display its returned error. Refresh notifications show both cached and removed counts.

- [ ] **Step 4: Add locale keys and verify GREEN**

Add English, Simplified Chinese, and Traditional Chinese strings for `status_legacy`, `upgrade_key`, `upgrade_help`, `upgrade_compat_warning`, and `key_too_short`.

Run:

```bash
rtk cargo test -p main team_key
rtk cargo check -p main
```

Expected: UI decision tests pass, translations resolve, and the application compiles.

- [ ] **Step 5: Commit**

```bash
rtk git add main/src/settings/team_keys.rs main/src/settings/mod.rs main/src/setting_tab.rs main/locales/main.yml
rtk git commit -m "feat(sync): expose legacy team encryption upgrade"
```

### Task 9: Integration verification and review

**Files:**
- No planned production-file changes. Any verified review failure returns to the task that owns that behavior and repeats its RED/GREEN cycle.

- [ ] **Step 1: Run focused core suites**

```bash
rtk cargo test -p one-core team_key --lib -- --test-threads=1
rtk cargo test -p one-core team_cache --lib -- --test-threads=1
rtk cargo test -p one-core delete_sync_data --lib
```

Expected: all focused tests pass with zero failures.

- [ ] **Step 2: Run affected application/form suites**

```bash
rtk cargo test -p main team_key
rtk cargo test -p db_view
rtk cargo test -p redis_view
rtk cargo test -p mongodb_view
rtk cargo test -p terminal_view
rtk cargo test -p remote_desktop_view
rtk cargo test -p port_forwarding_view
```

Expected: all affected consumer suites pass; ignored tests are reported separately.

- [ ] **Step 3: Run compile, clippy, formatting, and diff gates**

```bash
rtk cargo check -p main
rtk cargo clippy -p one-core -p main --all-targets -- -D warnings
rtk cargo fmt --all -- --check
rtk git diff --check
```

Expected: check/clippy/diff pass. If full formatting reports only documented pre-existing `db_view` baselines, do not edit unrelated files; report the baseline and verify every changed Rust file with `rustfmt --check`.

- [ ] **Step 4: Run review skills**

Use `superpowers:requesting-code-review`; classify and fix every Critical/Important finding. If feedback requires behavioral changes, use `superpowers:receiving-code-review` and add a failing regression test before production edits.

- [ ] **Step 5: Run completion verification**

Use `superpowers:verification-before-completion`, rerun the exact final commands, inspect `rtk git status --short --branch` and `rtk git diff sync/onetcli-20260711-2...HEAD --stat`, and audit all eight requirements in the approved spec against code and test evidence.
