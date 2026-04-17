# SSH Session Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure each SSH terminal tab reuses one underlying SSH session for shell, history loading, file manager, server monitor, and `cd` directory completion.

**Architecture:** Add a tab-scoped `SshSessionManager` in the `ssh` crate that lazily owns one shared `RusshClient`, reconnects on demand, and hands out session channels to consumers. Then migrate terminal shell setup, SSH history loading, SFTP-backed features, and server monitor commands to request channels or SFTP clients from that manager instead of calling `RusshClient::connect` or `RusshSftpClient::connect` independently.

**Tech Stack:** Rust, ssh, terminal, sftp, terminal_view

---

### Task 1: Add failing session-manager tests

**Files:**
- Create: `crates/ssh/src/session_manager.rs`
- Modify: `crates/ssh/src/lib.rs`

- [ ] **Step 1: Add a test that repeated client acquisition reuses one connector invocation**
- [ ] **Step 2: Add a test that invalidating the manager forces the next acquisition to reconnect**
- [ ] **Step 3: Add a test that manager disconnect clears the cached client**

### Task 2: Implement the shared SSH session manager

**Files:**
- Create: `crates/ssh/src/session_manager.rs`
- Modify: `crates/ssh/src/lib.rs`

- [ ] **Step 1: Add a lazy, cloneable `SshSessionManager` around a shared `RusshClient`**
- [ ] **Step 2: Expose channel acquisition plus explicit invalidate/disconnect operations**
- [ ] **Step 3: Keep the implementation testable with an internal connector abstraction**

### Task 3: Make SFTP clients work on top of an existing SSH session

**Files:**
- Modify: `crates/sftp/src/russh_impl.rs`
- Modify: `crates/sftp/src/lib.rs`

- [ ] **Step 1: Add a constructor that opens an SFTP subsystem over an existing shared client**
- [ ] **Step 2: Preserve raw SFTP pipeline support by opening extra subsystem channels through the shared client**
- [ ] **Step 3: Keep owned-connect behavior unchanged for existing callers**

### Task 4: Migrate SSH terminal shell and history loading

**Files:**
- Modify: `crates/terminal/src/ssh_backend.rs`
- Modify: `crates/terminal/src/terminal.rs`

- [ ] **Step 1: Store a tab-scoped `SshSessionManager` on SSH terminals**
- [ ] **Step 2: Make shell integration setup and interactive shell channels open through the manager**
- [ ] **Step 3: Make SSH history loading exec through the manager instead of opening a new connection**

### Task 5: Migrate terminal view SSH consumers

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`
- Modify: `crates/terminal_view/src/sidebar/file_manager_panel.rs`
- Modify: `crates/terminal_view/src/sidebar/server_monitor_panel.rs`

- [ ] **Step 1: Pass the tab’s `SshSessionManager` into SSH sidebar consumers**
- [ ] **Step 2: Make file manager transfer/browse clients and `cd` completion SFTP use the shared session**
- [ ] **Step 3: Make server monitor exec commands use the shared session**

### Task 6: Verify targeted and regression coverage

- [ ] **Step 1: Run shared-session tests**

Run: `cargo test -p ssh session_manager -- --nocapture`
Expected: PASS

Run: `cargo test -p sftp -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run terminal regressions**

Run: `cargo test -p terminal -- --nocapture`
Expected: PASS

Run: `cargo test -p terminal_view -- --nocapture`
Expected: PASS
