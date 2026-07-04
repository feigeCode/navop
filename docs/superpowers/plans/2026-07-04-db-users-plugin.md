# Database Users Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move database user listing and user-management toolbar actions out of `db_view` and into database plugins, then expose full user operation dialogs with form and SQL preview tabs.

**Architecture:** `db` owns database-specific user SQL, form manifests, and capabilities. `db_view` renders the user list and opens a generic user operation editor that mirrors the existing database editor pattern: first tab is the operation form, second tab is generated SQL preview, and submit executes the preview SQL then refreshes the users tab.

**Tech Stack:** Rust 2024, `db::DatabasePlugin`, `db::plugin_manifest`, GPUI, `gpui-component`, existing `GlobalDbState::execute_single`, cargo tests.

---

### Task 1: Extend Plugin User Contracts

**Files:**
- Modify: `crates/db/src/plugin_manifest.rs`
- Modify: `crates/db/src/plugin.rs`
- Modify: `crates/db/src/sqlite/plugin.rs`

- [x] Add `DatabaseFormKind::{CreateUser, EditUser, DeleteUser, UserPrivileges}`.
- [x] Add `DatabaseUiCapabilities` booleans for user listing and each user operation.
- [x] Add `DatabaseUserOperationRequest` to `plugin.rs` with `user_name`, `host`, `database`, and `field_values`.
- [x] Add default plugin methods for list/create/edit/delete/privileges user SQL.
- [x] Run manifest and request contract tests: `rtk cargo test -p db plugin_manifest`, `rtk cargo test -p db database_user_operation_request_keeps_context`, and `rtk cargo test -p db test_user_management_defaults_to_unsupported`.

### Task 2: Move User Listing SQL Into Built-In Plugins

**Files:**
- Modify: `crates/db/src/mysql/plugin.rs`
- Modify: `crates/db/src/postgresql/plugin.rs`
- Modify: `crates/db/src/mssql/plugin.rs`
- Modify: `crates/db/src/oracle/plugin.rs`
- Modify: `crates/db/src/clickhouse/plugin.rs`
- Modify: `crates/db/src/sqlite/plugin.rs`
- Modify: `crates/db/src/duckdb/plugin.rs`
- Modify: `crates/db_view/src/database_users.rs`
- Modify: `crates/db_view/src/database_users_tab.rs`

- [x] Write failing plugin tests proving MySQL, PostgreSQL, MSSQL, Oracle, and ClickHouse return the current user-list SQL.
- [x] Write failing tests proving SQLite and DuckDB do not support user listing.
- [x] Implement `build_list_users_sql` in each built-in plugin.
- [x] Change `DatabaseUsersTab::reload` to request list SQL from `GlobalDbState` plugin instead of `database_users.rs`.
- [x] Delete obsolete `db_view` database-type SQL branching once callers are migrated.
- [x] Run targeted user-list SQL tests.

### Task 3: Add Plugin User Forms and SQL Builders

**Files:**
- Modify: `crates/db/src/mysql/plugin.rs`
- Modify: `crates/db/src/postgresql/plugin.rs`
- Modify: `crates/db/src/mssql/plugin.rs`
- Modify: `crates/db/src/oracle/plugin.rs`
- Modify: `crates/db/src/clickhouse/plugin.rs`
- Modify: `crates/db/locales/db.yml`

- [x] Add form manifests for create/edit/delete/privileges users where supported.
- [x] Write and satisfy SQL-builder tests for MySQL and PostgreSQL including escaping.
- [x] Add basic SQL-builder tests and implementations for MSSQL, Oracle, and ClickHouse.
- [x] Run targeted user operation SQL and UI manifest tests.

### Task 4: Build Generic User Form and User Editor View

**Files:**
- Create: `crates/db_view/src/common/generic_user_form.rs`
- Create: `crates/db_view/src/common/user_editor_view.rs`
- Modify: `crates/db_view/src/common/mod.rs`
- Modify: `crates/db_view/src/database_view_plugin.rs`

- [x] Implement `GenericUserForm` from plugin `DatabaseFormManifest`, modeled after `GenericDatabaseForm`, but emitting user operation requests.
- [x] Implement `UserEditorView` from `DatabaseEditorView` patterns: `Form` tab, `SqlPreview` tab, error display, and `get_sql`.
- [x] Add `database_view_plugin` helpers to create user editor views for each user `DatabaseFormKind`.
- [x] Run `rtk cargo check -p db_view`.

### Task 5: Wire Toolbar Actions to User Dialogs

**Files:**
- Modify: `crates/db_view/src/database_users_toolbar.rs`
- Modify: `crates/db_view/src/database_users_tab.rs`
- Modify: `crates/db_view/src/database_users_list.rs`

- [x] Replace hard-coded unimplemented toolbar actions with `DatabaseUsersTab` handlers.
- [x] Add selected-user extraction from the current row with support for `User`/`Host`, `rolname`, `name`, and `username` columns.
- [x] For add/edit/delete/privileges, open the user editor dialog using plugin forms.
- [x] On submit, execute preview SQL through `GlobalDbState::execute_single`, close the dialog, show success/failure notification, and reload users.
- [x] Warn when edit/delete/privileges is clicked without a selected user.
- [x] Run `rtk cargo check -p db_view`.

### Task 6: Preserve External Driver Compatibility

**Files:**
- Modify: `crates/db/src/ipc/plugin.rs`
- Modify: `crates/db/src/ipc/protocol.rs`
- Modify: `crates/db_view/src/database_users_tab.rs`

- [x] Keep existing `schema/users` list path for external drivers.
- [x] Route external user-list fallback through plugin-compatible behavior, not `db_view`.
- [x] Do not show user operation toolbar buttons for external drivers unless their manifest declares the relevant forms/capabilities.
- [x] Run targeted IPC/plugin tests around `schema/users`.

### Task 7: Final Verification

**Files:**
- All modified files

- [x] Run `rtk cargo fmt`.
- [x] Run `rtk cargo test -p db`.
- [x] Run `rtk cargo test -p db_view database_users`.
- [x] Run `rtk cargo check -p db -p db_view`.
- [x] Inspect `rtk git diff --stat` and `rtk git diff --check`.
