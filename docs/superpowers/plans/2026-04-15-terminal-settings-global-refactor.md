# Terminal Settings Global Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move terminal settings ownership into `terminal_view`, persist them in a dedicated JSON file, and replace `HomePage`-driven fan-out with a GPUI global settings store that every terminal view subscribes to directly.

**Architecture:** Add a `terminal_view::settings` module that owns the `TerminalSettings` snapshot, JSON persistence, legacy-seed migration, and a global `Entity<TerminalSettingsStore>`. Each `TerminalView` reads the snapshot on creation and subscribes to store change events; `main` only provides a legacy seed during startup and no longer participates in runtime terminal settings broadcast.

**Tech Stack:** Rust, GPUI globals/entities/events, serde/serde_json, existing `one_core::storage::get_config_dir`

---

### Task 1: Add terminal settings store in `terminal_view`

**Files:**
- Create: `crates/terminal_view/src/settings.rs`
- Modify: `crates/terminal_view/src/lib.rs`
- Test: `crates/terminal_view/src/settings.rs`

- [ ] Define `TerminalSettings`, `TerminalSettingsEvent`, `TerminalSettingsStore`, and `GlobalTerminalSettings`.
- [ ] Add load/save helpers for `terminal-settings.json` using `get_config_dir()`.
- [ ] Add `init_terminal_settings(cx, legacy_seed)` and public access/update helpers.
- [ ] Write unit tests for defaults, persistence-path helpers, and legacy-seed preference when JSON is absent.

### Task 2: Make `TerminalView` consume the global store directly

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/settings.rs`

- [ ] Replace `TerminalViewEvent` settings fan-out with direct updates into `TerminalSettingsStore`.
- [ ] On `TerminalView` construction, apply the current settings snapshot and subscribe to `TerminalSettingsEvent::Changed`.
- [ ] Keep per-view apply methods, but drive them from snapshot changes rather than `HomePage`.
- [ ] Add a regression test for store update semantics so runtime changes emit exactly one changed snapshot.

### Task 3: Remove `main`-side ownership and wire startup migration

**Files:**
- Modify: `main/src/main.rs`
- Modify: `main/src/onetcli_app.rs`
- Modify: `main/src/setting_tab.rs`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `main/src/home_tab.rs`
- Test: `main/src/setting_tab.rs`

- [ ] Introduce a small legacy mapping from `AppSettings` terminal fields into `terminal_view::TerminalSettings`.
- [ ] Initialize `terminal_view` global settings once during startup with the legacy seed.
- [ ] Remove terminal settings group UI and `HomePage` terminal-settings broadcast bookkeeping.
- [ ] Delete obsolete `AppSettings` terminal defaults/tests that are no longer source-of-truth, keeping only migration helpers if still needed.

### Verification

- [ ] `cargo test -p terminal_view terminal_settings -- --nocapture`
- [ ] `cargo test -p main app_settings_ -- --nocapture`
- [ ] `cargo check -p terminal_view -p main`
