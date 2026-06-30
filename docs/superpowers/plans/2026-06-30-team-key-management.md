# Team Key Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each team member enter, verify, cache, forget, and reuse their own local team key so team-owned connections are encrypted with the team key without storing the key on the team record.

**Architecture:** Add a core team-key manager around existing `CloudSyncService`, `TeamKeyCacheRepository`, and `crypto` helpers. UI forms use shared status/guard helpers so selecting a team with no valid local key prompts the user before saving.

**Tech Stack:** Rust, GPUI, SQLite repositories, existing AES-GCM crypto helpers, existing cloud sync models.

---

### Task 1: Core Team Key Manager

**Files:**
- Create: `crates/core/src/cloud_sync/team_key_manager.rs`
- Modify: `crates/core/src/cloud_sync/mod.rs`

- [ ] Add tests for saving, loading, validating, version mismatch, and forgetting cached team keys.
- [ ] Implement a small manager API using `TeamKeyCacheRepository`, `CloudSyncService`, and `crypto::{encrypt_with_key, decrypt_with_key, verify_master_key}`.
- [ ] Export status and manager types from `cloud_sync`.
- [ ] Run `cargo test -p one-core team_key_manager`.

### Task 2: Team Option Status

**Files:**
- Modify: `crates/core/src/cloud_sync/mod.rs`

- [ ] Extend cached team options with key status metadata while preserving current id/name use sites.
- [ ] Add tests for missing, cached, unlocked, and version mismatch status mapping.
- [ ] Run `cargo test -p one-core cloud_sync::tests`.

### Task 3: Shared UI Guard

**Files:**
- Create: `main/src/team_key_ui.rs`
- Modify: `main/src/main.rs`
- Modify: `main/locales/main.yml`

- [ ] Add helper functions to render a team-key dialog and to validate selected-team save readiness.
- [ ] Keep plaintext team keys inside input state and pass them directly to core manager on save.
- [ ] Add localized labels, errors, and tooltip text.

### Task 4: Form Integration

**Files:**
- Modify: `crates/db_view/src/common/db_connection_form.rs`
- Modify: `crates/terminal_view/src/ssh_form_window.rs`
- Modify: `crates/redis_view/src/redis_form_window.rs`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form/view.rs`
- Modify: remote desktop persistence files as needed

- [ ] Show team key state next to team selectors.
- [ ] Add a set-key action for selected teams.
- [ ] Block saving a team connection when the selected team key is missing or stale.

### Task 5: Verification

**Files:**
- All touched files.

- [ ] Run focused core tests.
- [ ] Run focused form tests where available.
- [ ] Run `cargo check` for affected crates.
- [ ] Request code review and address findings.
