# Update Archive Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the auto-update pipeline install release archives correctly on macOS, Linux, and Windows instead of treating archives as executables.

**Architecture:** Add an archive extraction stage, then perform platform-specific payload installation from a staging directory. Keep rollback semantics for file replacement, replace the full `.app` bundle on macOS, replace the binary on Linux, and launch a real extracted helper binary on Windows.

**Tech Stack:** Rust, `flate2`, `tar`, Windows-only `zip`, existing `gpui` update flow

---

### Task 1: Lock in failing tests for archive handling

**Files:**
- Create: `main/src/update/extract.rs`
- Modify: `main/src/update/install.rs`
- Modify: `main/src/update/mod.rs`
- Test: `main/src/update/extract.rs`
- Test: `main/src/update/install.rs`

- [ ] **Step 1: Write failing tests for archive extraction**

Add tests that create tiny `.tar.gz` fixtures in temp directories and assert `extract_archive()` unpacks nested files into a destination directory.

- [ ] **Step 2: Write failing tests for Linux payload discovery**

Add tests that assert `locate_linux_binary()` prefers `usr/bin/onetcli` and falls back to `onetcli` in the staging root.

- [ ] **Step 3: Write failing tests for macOS bundle path resolution**

Add tests that assert `current_app_bundle_path_from_exe()` maps `/Applications/OnetCli.app/Contents/MacOS/onetcli` to `/Applications/OnetCli.app` and rejects non-bundle paths.

- [ ] **Step 4: Run targeted tests and confirm failures are for missing functionality**

Run: `cargo test -p main update::extract::tests update::install::tests -- --nocapture`

- [ ] **Step 5: Commit after the red phase**

Do not commit in this task unless explicitly requested.

### Task 2: Implement extraction and staged payload installation

**Files:**
- Create: `main/src/update/extract.rs`
- Modify: `main/src/update/install.rs`
- Modify: `main/src/update/mod.rs`
- Modify: `main/Cargo.toml`

- [ ] **Step 1: Add archive dependencies**

Declare `flate2` and `tar` in `main/Cargo.toml`, and add Windows-only `zip` support.

- [ ] **Step 2: Implement archive extraction**

Implement `extract_archive(archive, dest_dir)` with extension dispatch for `.tar.gz`, `.tgz`, and `.zip`.

- [ ] **Step 3: Rewrite `start_install_update()` around a staging directory**

Create a staged extraction directory under the update temp root, extract the archive into it, then branch into platform-specific installers.

- [ ] **Step 4: Implement platform-specific installers**

Implement:
- macOS full `.app` bundle replacement with rollback and `open -n`
- Linux binary replacement with explicit permission checks and restart
- Windows helper spawn using the extracted `onetcli.exe`

- [ ] **Step 5: Preserve rollback helpers and adapt them to new flow**

Reuse `replace_target_with_backup()` for single-file replacement and add directory move/copy helpers for macOS bundles.

### Task 3: Clean up download and startup behavior

**Files:**
- Modify: `main/src/update/download.rs`
- Modify: `main/src/update/mod.rs`
- Modify: `main/src/update/install.rs`

- [ ] **Step 1: Remove archive `chmod` from download path**

Delete the post-download `set_executable_permission(download_path)` call because archives are not executables.

- [ ] **Step 2: Add startup cleanup for stale backups**

Implement best-effort cleanup for `OnetCli.app.old`, `onetcli.old`, and `onetcli.exe.old` on normal startup before update handling exits.

- [ ] **Step 3: Keep update command contract stable**

Ensure `handle_update_command()` still returns early for helper execution and normal app launch continues for non-helper invocations.

### Task 4: Verify with formatting, tests, and build

**Files:**
- Modify: `main/src/update/extract.rs`
- Modify: `main/src/update/install.rs`
- Modify: `main/src/update/download.rs`
- Modify: `main/src/update/mod.rs`
- Modify: `main/Cargo.toml`

- [ ] **Step 1: Run formatter**

Run: `cargo fmt`

- [ ] **Step 2: Run targeted update tests**

Run: `cargo test -p main update:: -- --nocapture`

- [ ] **Step 3: Run crate build verification**

Run: `cargo build --release -p main`

- [ ] **Step 4: Summarize any platform gaps honestly**

If Windows-only tests or cross-platform manual checks were not run in this environment, report that explicitly instead of implying full coverage.
