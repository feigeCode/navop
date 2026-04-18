# Remote File Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a reusable remote text file editor window for `sftp_view` and `file_manager_sider`, opened by double-clicking remote files and saving changes back through SFTP.

**Architecture:** Introduce a new `remote_file_editor` crate that owns editor window UI, file policy, and remote read/write orchestration. Extend `sftp` with a bounded `read_file` API, then wire both `sftp_view` and `terminal_view` to open the shared window for remote files.

**Tech Stack:** Rust, GPUI, gpui-component `InputState` code editor, one-core popup window helper, russh-sftp

---

### Task 1: Add bounded remote file read support in `sftp`

**Files:**
- Modify: `crates/sftp/src/lib.rs`
- Modify: `crates/sftp/src/russh_impl.rs`

- [x] Add tests for any new pure helper logic used to enforce size bounds.
- [x] Add `read_file(path, max_bytes)` to the `SftpClient` trait.
- [x] Implement bounded reading in `RusshSftpClient`.
- [x] Run `cargo test -p sftp`.

### Task 2: Create reusable `remote_file_editor` crate

**Files:**
- Create: `crates/remote_file_editor/Cargo.toml`
- Create: `crates/remote_file_editor/src/lib.rs`
- Create: `crates/remote_file_editor/src/file_policy.rs`
- Create: `crates/remote_file_editor/src/language.rs`
- Create: `crates/remote_file_editor/src/editor_window.rs`
- Modify: `Cargo.toml`

- [x] Add failing tests for file policy and language mapping.
- [x] Implement file size policy, UTF-8 normalization, and language detection.
- [x] Implement popup editor window with toolbar (`Save`, `Search`, `Reload`, `Soft Wrap`) and status display.
- [x] Wire save to `SftpClient::write_file`.
- [x] Run `cargo test -p remote_file_editor`.

### Task 3: Wire `sftp_view` remote file double-click and context menu

**Files:**
- Modify: `crates/sftp_view/Cargo.toml`
- Modify: `crates/sftp_view/src/file_list_panel.rs`
- Modify: `crates/sftp_view/src/lib.rs`
- Modify: `crates/sftp_view/src/context_menu_handler.rs`

- [x] Extend remote file events to carry enough data to distinguish files and directories.
- [x] Open the shared editor window on remote file double-click.
- [x] Add an `Edit` context menu action for remote files.
- [x] Run targeted integration verification for `sftp_view` via `cargo check -p sftp_view -p terminal_view`.

### Task 4: Wire `file_manager_sider` remote file double-click and context menu

**Files:**
- Modify: `crates/terminal_view/Cargo.toml`
- Modify: `crates/terminal_view/src/sidebar/file_manager_panel.rs`

- [x] Replace remote file double-click copy behavior with editor opening.
- [x] Add an `Edit` context menu action for remote files.
- [x] Keep directory navigation behavior unchanged.
- [x] Run targeted integration verification for `terminal_view` via `cargo check -p sftp_view -p terminal_view`.

### Task 5: Verify integrated behavior

**Files:**
- Modify: `docs/superpowers/plans/2026-04-17-remote-file-editor.md`

- [x] Run formatter if needed on touched Rust files.
- [x] Run targeted verification commands for `sftp`, `remote_file_editor`, `sftp_view`, and `terminal_view`.
- [x] Update this plan checklist to reflect actual work completed.

## Verification Notes

- `cargo test -p sftp -- --nocapture` passed.
- `cargo test -p remote_file_editor -- --nocapture` passed.
- `cargo test -p remote_file_editor close_guard -- --nocapture` passed for unsaved-close guard logic.
- `cargo check -p sftp_view -p terminal_view` passed.
- `cargo test -p sftp_view --lib -- --nocapture` is currently blocked by the local GPUI Metal shader toolchain issue: `cannot execute tool 'metal' due to missing Metal Toolchain; use: xcodebuild -downloadComponent MetalToolchain`.
