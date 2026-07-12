# Shared Team Connection Form Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Centralize connection-form team assignment, selection state, validation, ownership metadata, and internationalization in a reusable workspace crate.

**Architecture:** Add a `connection-form` UI-support crate between `one-core` and protocol-specific view crates. Keep team domain/key operations in `one-core`, while consumers retain their layouts and persistence code.

**Tech Stack:** Rust 2024, GPUI, gpui-component, rust-i18n, Cargo workspace tests/checks/clippy.

---

### Task 1: Add the shared team form contract with TDD

**Files:**
- Create: `crates/connection_form/Cargo.toml`
- Create: `crates/connection_form/locales/connection_form.yml`
- Create: `crates/connection_form/src/lib.rs`
- Create: `crates/connection_form/src/team.rs`
- Modify: `Cargo.toml`

- [x] Write tests for personal-first option ordering, key-status labels, selected team values, and new/edit owner assignment.
- [x] Run `rtk cargo test -p connection-form` and confirm the new contract tests fail because the API is absent.
- [x] Implement the minimum public types and pure assignment contract.
- [x] Run `rtk cargo test -p connection-form` and confirm the tests pass.
- [x] Add GPUI select-state creation, selection reading, refresh replacement, and key validation wrappers.
- [x] Run `rtk cargo check -p connection-form`.

### Task 2: Migrate connection view crates

**Files:**
- Modify: `crates/db_view/Cargo.toml`
- Modify: `crates/db_view/src/common/db_connection_form.rs`
- Modify: `crates/redis_view/Cargo.toml`
- Modify: `crates/redis_view/src/redis_form_window.rs`
- Modify: `crates/mongodb_view/Cargo.toml`
- Modify: `crates/mongodb_view/src/mongo_form_window.rs`
- Modify: `crates/terminal_view/Cargo.toml`
- Modify: `crates/terminal_view/src/ssh_form_window.rs`
- Modify: `crates/terminal_view/src/serial_form_window.rs`
- Modify: `crates/port_forwarding_view/Cargo.toml`
- Modify: `crates/port_forwarding_view/src/selects.rs`
- Modify: `crates/port_forwarding_view/src/form_window.rs`
- Modify: `crates/port_forwarding_view/src/view.rs`
- Modify: `crates/remote_desktop_view/Cargo.toml`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form.rs`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form/selects.rs`
- Modify: `crates/remote_desktop_view/src/remote_desktop_form/view.rs`

- [x] Add the workspace dependency to every consumer.
- [x] Replace local team item types and initialization with `connection_form::team` APIs.
- [x] Replace local selected-id and save validation code with the shared contract.
- [x] Replace local team labels with shared localized label functions.
- [x] Run `rtk cargo check` for each affected package and fix migration errors without changing layout.

### Task 3: Remove duplicate translations and review the result

**Files:**
- Modify: `crates/db_view/locales/db_view.yml`
- Modify: `crates/redis_view/locales/redis_view.yml`
- Modify: `crates/mongodb_view/locales/mongodb_view.yml`
- Modify: `crates/terminal_view/locales/terminal_view.yml`
- Modify: `crates/port_forwarding_view/locales/port_forwarding_view.yml`
- Modify: `crates/remote_desktop_view/locales/remote_desktop_view.yml`

- [x] Remove connection-form `TeamSync` blocks after all call sites use the shared crate.
- [x] Search for duplicate `TeamSelectItem`, `team_select_name`, and local `TeamSync.team_label` references.
- [x] Review `git diff` for API leakage, ownership regressions, locale gaps, and unrelated edits.
- [x] Run formatting, targeted tests, and package checks; run warnings-as-errors clippy for the new crate.
- [x] Re-run the requirement checklist and record the full affected-crate clippy blockers from pre-existing code.

Verification note: warnings-as-errors Clippy for all affected crates is blocked by pre-existing lints outside this change, including `crates/ui/src/highlighter/registry.rs::register_wasm`, Redis CLI/tree files, and MongoDB collection/tree files. `connection-form` itself passes `cargo clippy --all-targets --no-deps -- -D warnings`.
