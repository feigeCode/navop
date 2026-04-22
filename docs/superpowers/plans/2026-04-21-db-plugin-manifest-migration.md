# DB Plugin Manifest Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`
> (recommended) or `superpowers:executing-plans` to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the `db_view` database view plugin layer and migrate database-specific
UI differences into `crates/db` manifests so `db_view` becomes a unified renderer ready
for future IPC-backed plugins.

**Architecture:** Introduce manifest and capability types in
`crates/db/src/plugin_manifest.rs`, implement them in every built-in database plugin,
then switch `db_view` call sites from `DatabaseViewPluginRegistry` to
`DbManager`/`DatabasePlugin` manifest reads. Replace per-database forms, menus, toolbars,
and table-designer conditionals with generic manifest-driven renderers and finally remove
the old view-plugin registry.

**Tech Stack:** Rust 2024, gpui, gpui_component, `rust_i18n`, existing `db` plugin
system, `db_view` dialogs/forms, `serde` (for IPC-ready manifest types).

**Reference design:** `docs/superpowers/specs/2026-04-21-db-plugin-manifest-design.md`

---

### Global validation policy

Every task below ends with a **mandatory validation gate** before the task is closed:

```bash
cargo fmt --check
cargo clippy -p db -p db_view -- -D warnings
cargo check -p db -p db_view -p main
```

Additionally, each task lists the focused `cargo test` commands that must pass. The
migration is NOT complete until the final regression sweep (Task 12) is green.

If a task changes GPUI UI behavior, manually verify the scenarios listed in that task's
"Manual smoke" block before marking it done.

---

### Task 0: Baseline inventory and guard rails

**Files:**
- Read-only: `crates/db_view/src/database_view_plugin.rs`
- Read-only: `crates/db_view/src/**/*_view_plugin.rs`
- Read-only: `crates/db_view/src/common/db_connection_form.rs`
- Read-only: `crates/db_view/src/common/database_editor_view.rs`
- Read-only: `crates/db_view/src/common/schema_editor_view.rs`
- Read-only: every file listed in design doc Appendix B

- [ ] Confirm every call site in design-doc Appendix B still exists. If any line numbers
  shifted, update the design doc's Appendix B.
- [ ] Record the current tree-event inventory (`DbTreeViewEvent`) and the objects-event
  inventory (`DatabaseObjectsEvent`) into a scratch note; the action manifest must cover
  every variant used by any plugin's `build_context_menu` or `build_toolbar_buttons`.
- [ ] Record the current `DbFormConfig::*` factory list (MySQL, PostgreSQL, MSSQL, Oracle,
  SQLite, ClickHouse, DuckDB) and the field sets each produces. This is the reference
  truth for manifest output during migration.
- [ ] Enumerate the current `*_view_plugin.rs` methods that produce non-data values
  (e.g. `create_connection_form`, `create_database_editor_view`). Record whether each has
  a schema/create-schema counterpart and which databases support them.
- [ ] Confirm that `GlobalDbState::get_plugin(&DatabaseType)` returns an `Arc<dyn DatabasePlugin>`
  accessible from GPUI call sites (`crates/db/src/manager.rs:747`).

Validation: no code changes. Exit when the scratch note is complete and Appendix B is
in sync.

### Task 1: Define the manifest contract in `crates/db`

**Files:**
- Create: `crates/db/src/plugin_manifest.rs`
- Modify: `crates/db/src/plugin.rs` (add module + trait methods)
- Modify: `crates/db/src/lib.rs` (re-export manifest types)
- Modify: `crates/db/Cargo.toml` (ensure `serde`, `serde_derive` are available; they
  are already pulled in transitively — verify)
- Modify: `crates/db/README.md`
- Test: `crates/db/src/plugin_manifest.rs` (unit tests module)

- [ ] Create `plugin_manifest.rs` with the types specified in design doc
  "Proposed Core Types":
  `DatabaseUiManifest`, `DatabaseUiCapabilities`, `DatabaseFormKind`,
  `DatabaseFormManifest`, `DatabaseFormTab`, `DatabaseFormField`,
  `DatabaseFormFieldType`, `FormSelectOption`, `FormVisibilityRule`,
  `FormValueCondition`, `FormDefaultRule`, `ReferenceDataKind`,
  `DatabaseActionManifest`, `DatabaseActionDescriptor`, `DatabaseActionId`,
  `DatabaseActionTarget`, `DatabaseActionPlacement`, `DatabaseActionToolbarScope`,
  `DatabaseFormSubmission`.
- [ ] All types must derive `Clone`, `Debug`, `Serialize`, `Deserialize`. Enums also
  derive `PartialEq`, `Eq`, and `Hash` where appropriate (needed for action-ID
  indexing). `DatabaseUiCapabilities` additionally derives `Default`.
- [ ] Add `pub const DATABASE_UI_MANIFEST_VERSION: u32 = 1;` and use it inside
  `DatabaseUiManifest::default()`.
- [ ] Extend `DatabasePlugin` in `plugin.rs` with:
  - `fn ui_manifest(&self) -> DatabaseUiManifest;`
  - `fn resolve_reference_data(&self, kind: ReferenceDataKind, context: &HashMap<String,String>) -> Vec<FormSelectOption> { Vec::new() }`
- [ ] Provide a default `ui_manifest()` that projects existing trait booleans
  (`supports_schema`, `uses_schema_as_database`, `supports_sequences`,
  `supports_functions`, `supports_procedures`) into `DatabaseUiCapabilities`. This
  avoids per-plugin duplication for the capability half of the manifest.
- [ ] Keep the new API free of GPUI types, closures, and Rust function pointers.
- [ ] Re-export from `crates/db/src/lib.rs`:
  `pub use plugin_manifest::*;` — follow existing crate public-API conventions.
- [ ] Update `crates/db/README.md` so the crate description reflects that `plugin.rs` +
  `plugin_manifest.rs` now own both database operations and UI manifest contracts.
- [ ] Add unit tests in `plugin_manifest.rs`:
  - default `DatabaseUiCapabilities` has all booleans `false` and `table_engines` empty.
  - `serde_json::to_string` + `from_str` round-trips for all manifest types (smoke).
  - `FormVisibilityRule` evaluator helper returns the correct result for each
    `FormValueCondition` variant.

Run: `cargo fmt --check`, `cargo clippy -p db -- -D warnings`, `cargo check -p db`,
`cargo test -p db plugin_manifest::`.
Expected: `db` compiles with the new manifest API in place, unit tests pass.

### Task 2: Move `engines()` into `crates/db` and add reference-data plumbing

**Files:**
- Modify: `crates/db/src/plugin.rs` (add `fn engines(&self) -> Vec<String>` with
  default empty)
- Modify: `crates/db/src/mysql/plugin.rs` (override `engines()`)
- Modify: `crates/db_view/src/database_view_plugin.rs` (mark `fn get_engines` as
  forwarding to `db` plugin during the transition — removed in Task 10)
- Modify: `crates/db_view/src/table_designer_tab.rs` (no user-visible change yet)

- [ ] Move the MySQL engine list from `mysql_view_plugin.rs::get_engines` into
  `crates/db/src/mysql/plugin.rs::engines`. Do not change the list contents.
- [ ] Default `DatabasePlugin::engines()` returns `vec![]`.
- [ ] In `DatabasePlugin::ui_manifest()` default impl, populate
  `capabilities.table_engines` from `self.engines()`. This becomes the single source of
  truth for the engine list.
- [ ] During transition, keep `DatabaseViewPlugin::get_engines` but have it delegate to
  `db::DatabasePlugin::engines()` via `GlobalDbState` until Task 5 removes the call
  site. This prevents Task 5 from being blocked by Task 2's churn.
- [ ] Add unit test that `MySqlPlugin::new().ui_manifest().capabilities.table_engines`
  matches the old `MySqlDatabaseViewPlugin::get_engines()` output verbatim.

Run: `cargo fmt --check`, `cargo clippy -p db -p db_view -- -D warnings`,
`cargo check -p db -p db_view`, `cargo test -p db mysql::`.
Expected: engine list available from `db` crate; `db_view` unchanged in behavior.

### Task 3: Implement built-in manifests — MySQL (canonical reference)

**Files:**
- Modify: `crates/db/src/mysql/plugin.rs`
- Reference: `crates/db_view/src/mysql/mysql_view_plugin.rs`
- Reference: `crates/db_view/src/common/db_connection_form.rs` (`DbFormConfig::mysql()`)
- Reference: `crates/db_view/src/mysql/database_form.rs`

MySQL is the richest database (engines + charset + collation + auto_increment +
unsigned + enum + edit-mode locking). Implementing it first establishes the patterns.

- [ ] Implement `MySqlPlugin::ui_manifest()` returning a complete `DatabaseUiManifest`:
  - `capabilities`: every field filled in from the current view plugin's
    `TableDesignerCapabilities` / `ColumnEditorCapabilities`.
  - `forms`: `Connection`, `CreateDatabase`, `EditDatabase`. (No `CreateSchema`.)
  - `actions`: every descriptor produced by `MySqlDatabaseViewPlugin::build_context_menu`
    and `build_toolbar_buttons`, mapped to `DatabaseActionId`.
- [ ] Connection form translation:
  - Port `DbFormConfig::mysql()` tab-by-tab into `DatabaseFormTab` list.
  - Convert `FormField { placeholder: String }` → manifest `placeholder_i18n_key`.
    Where today's code uses a literal placeholder, keep a literal but wrap in a new
    i18n key if possible. Where the placeholder is already a `t!()` value, use its key.
  - Convert SSH tab's conditional fields: add
    `visible_when: FormValueCondition::Equals("true")` referencing
    `ssh_tunnel_enabled` for every SSH field except the enable toggle itself.
  - Oracle-style SID/service_name mutual exclusivity does not apply here — MySQL uses a
    single `database` field.
- [ ] Create-database form:
  - Fields: `name` (required, disabled when editing), `charset` (select with
    `options_source: MySqlCharsets`), `collation` (select with
    `options_source: MySqlCollations`).
  - `default_when` on `collation`: `when_field_changes = "charset"`,
    `via = ReferenceDataKind::MySqlCollations`.
  - Submit key: `"Common.create"` (same as current dialog footer).
- [ ] Edit-database form: same as create, with `name.disabled_when_editing = true`.
- [ ] Implement `MySqlPlugin::resolve_reference_data`:
  - `ReferenceDataKind::MySqlCharsets` → delegates to existing `self.get_charsets()`,
    converting to `FormSelectOption`.
  - `ReferenceDataKind::MySqlCollations` → reads `context["charset"]`, delegates to
    `self.get_collations(&charset)`.
  - `ReferenceDataKind::TableEngines` → from `self.engines()`.
- [ ] Actions: map every current MySQL context-menu item and toolbar button into a
  `DatabaseActionDescriptor`. Use the mapping table in the design doc.
  Preserve separator grouping by assigning `group` identifiers.
- [ ] Unit tests:
  - `manifest.forms` contains exactly 3 entries with the expected `DatabaseFormKind`.
  - Connection manifest has tabs `["general", "advanced", "ssl", "ssh", "notes"]`.
  - `ssh_host.visible_when` references `ssh_tunnel_enabled == "true"`.
  - Create-database form has `charset` and `collation` with correct
    `options_source`/`default_when`.
  - Every `DatabaseActionId` emitted by the MySQL view plugin is present in
    `manifest.actions` with correct `targets`.

Run: `cargo fmt --check`, `cargo clippy -p db -- -D warnings`,
`cargo test -p db mysql::plugin::`.
Expected: MySQL manifest passes structural assertions.

### Task 4: Implement built-in manifests — remaining databases

**Files:**
- Modify: `crates/db/src/postgresql/plugin.rs`
- Modify: `crates/db/src/mssql/plugin.rs`
- Modify: `crates/db/src/oracle/plugin.rs`
- Modify: `crates/db/src/sqlite/plugin.rs`
- Modify: `crates/db/src/clickhouse/plugin.rs`
- Modify: `crates/db/src/duckdb/plugin.rs`
- Reference: matching `crates/db_view/src/*/*_view_plugin.rs` and `*/database_form.rs`
- Reference: `crates/db_view/src/postgresql/schema_form.rs`,
  `crates/db_view/src/mssql/schema_form.rs`

Implement manifests in this order to keep diffs reviewable:

- [ ] **PostgreSQL**: connection form, create-database form, edit-database form,
  `CreateSchema` form (with optional comment field), actions. Capabilities:
  `supports_schema = true`, `supports_sequences = true`.
- [ ] **MSSQL**: connection form, create-database, edit-database, `CreateSchema`,
  actions. Capabilities: `supports_schema = true`. Charset/collation are MSSQL-specific
  (see existing `crates/db/src/mssql/plugin.rs::get_charsets`).
- [ ] **Oracle**: connection form with `service_name` / `sid` fields (declare both
  optional; encode mutual-exclusivity with a visibility rule
  `visible_when NotEmpty` on the opposite field disabled OR leave both fields
  always visible with tooltips — pick the option that matches current UX exactly).
  Capabilities: `uses_schema_as_database = true`.
- [ ] **SQLite**: connection form with file-path field
  (`DatabaseFormFieldType::FilePath`). No create/edit-database or schema forms.
  Capabilities: `supports_rowid = true` (already exists).
- [ ] **ClickHouse**: connection form (including `schema` http/https select),
  create-database form, edit-database form, actions.
- [ ] **DuckDB**: file-path connection form. Minimal action set. Capabilities mirror
  current view plugin.
- [ ] Each plugin gets a `#[cfg(test)]` module with at least:
  - A smoke test that `ui_manifest()` returns `schema_version: 1`.
  - A test that asserts the list of form kinds matches the previous view plugin's
    dialog support.
- [ ] Cross-cutting: ensure every plugin's action set includes at most one
  `DatabaseActionDescriptor` per `(id, node_type)` pair.

Run: `cargo fmt --check`, `cargo clippy -p db -- -D warnings`,
`cargo test -p db`.
Expected: all built-in plugins return complete manifests.

### Task 5: Generic manifest-backed form rendering in `db_view`

**Files:**
- Create: `crates/db_view/src/common/manifest_renderer.rs`
- Create: `crates/db_view/src/common/generic_connection_form.rs`
- Create: `crates/db_view/src/common/generic_database_form.rs`
- Create: `crates/db_view/src/common/generic_schema_form.rs`
- Create: `crates/db_view/src/common/connection_form_binding.rs`
- Modify: `crates/db_view/src/common/mod.rs`
- Reference: `crates/db_view/src/common/db_connection_form.rs`
- Reference: `crates/db_view/src/common/database_editor_view.rs`
- Reference: `crates/db_view/src/common/schema_editor_view.rs`

- [ ] Introduce `manifest_renderer` with helpers to translate
  `DatabaseFormFieldType` + `DatabaseFormField` into the corresponding GPUI control
  state (`InputState`, `SelectState`, `Checkbox` state). Keep it narrow: supported kinds
  are `Text`, `Number`, `Password`, `TextArea`, `Select`, `Checkbox`, `FilePath`.
  Anything else panics with a clear message — we will not silently accept unknown kinds.
- [ ] Implement `FormStateMap` — a per-form owner of `{ field_id -> value: String }`.
  This is the single source of truth for visibility evaluation and submission.
- [ ] Implement `FormVisibilityEvaluator::is_visible(field, state)` using the rules on
  the field; add unit test.
- [ ] Implement `FormDefaultBridge` — observes source field changes and calls
  `plugin.resolve_reference_data(...)` to refresh dependent select options + default.
- [ ] `GenericConnectionForm` entity:
  - Inputs: `DatabaseFormManifest` + `Arc<dyn DatabasePlugin>` + `editing_connection: Option<StoredConnection>`.
  - Renders tabs, groups, fields.
  - Emits `DbConnectionFormEvent::Saved(StoredConnection)` and
    `DbConnectionFormEvent::SaveError(String)` — same events as today so
    `ConnectionFormWindow` is drop-in compatible.
  - Uses `ConnectionFormBinding::from_state_map(&state_map) -> DbConnectionConfig` to
    persist. This module centralizes field-name → config-field mapping for every
    database type.
- [ ] `GenericDatabaseOperationForm` entity:
  - Emits `DatabaseFormEvent::FormChanged(DatabaseOperationRequest)` on every change.
  - On submit, produces a `DatabaseFormSubmission` adapted into
    `DatabaseOperationRequest { database_name, field_values }`.
- [ ] `GenericSchemaOperationForm` entity: mirrors the database form for schema flows.
- [ ] Do **not** delete `DbConnectionForm` / `MySqlDatabaseForm` yet; Tasks 7–9 swap
  call sites, Task 10 deletes them.
- [ ] Unit tests:
  - `FormVisibilityEvaluator` covers `Equals`, `NotEquals`, `In`, `NotEmpty`.
  - `ConnectionFormBinding::from_state_map` round-trip for MySQL, PostgreSQL, SQLite,
    Oracle (including optional `sid`/`service_name` exclusivity).

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`, `cargo test -p db_view common::`.
Expected: generic form renderer compiles; no existing call site changed.

### Task 6: `ActionEventMapper` and manifest-based action filtering

**Files:**
- Create: `crates/db_view/src/action_event_mapper.rs`
- Modify: `crates/db_view/src/lib.rs`

- [ ] Implement `ActionEventMapper` with two entry points:
  - `pub fn to_tree_event(id: DatabaseActionId, node_id: &str) -> DbTreeViewEvent`
  - `pub fn to_objects_event(id: DatabaseActionId, node: DbNode) -> DatabaseObjectsEvent`
  - Both functions are exhaustive matches; `DatabaseActionId` variants without a
    corresponding event return `None` (typed as `Option<...>`) so the UI can skip the
    descriptor.
- [ ] For `DumpSqlStructure`/`DumpSqlData`/`DumpSqlStructureAndData`, map to
  `DbTreeViewEvent::DumpSqlFile { mode: SqlDumpMode::... }`.
- [ ] Implement `ManifestMenuBuilder::build(node_type, node_id, connection_active,
  &DatabaseActionManifest) -> Vec<PopupMenuItem>` that filters action descriptors by
  placement, target, and `requires_active_connection`, then groups separators via the
  `group` / `submenu_of` fields.
- [ ] Implement `ManifestToolbarBuilder::build(node_type, data_node_type,
  &DatabaseActionManifest) -> Vec<ToolbarButton>` using the same filtering strategy
  and honoring `toolbar_scope` (`CurrentNode` vs `SelectedRow`).
- [ ] Unit tests:
  - Every `DatabaseActionId` variant has an entry in at least one mapper.
  - `ManifestMenuBuilder` produces identical item ordering as the current MySQL
    `build_context_menu` for `DbNodeType::Connection` / `Database` / `Table`.
  - `ManifestToolbarBuilder` produces identical buttons as the current MySQL
    `build_toolbar_buttons` for the same node/data-node pairs.

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`, `cargo test -p db_view action_event_mapper::`.
Expected: mapper + builders compile; snapshot-style tests pass.

### Task 7: Switch table designer to `db` manifests

**Files:**
- Modify: `crates/db_view/src/table_designer_tab.rs`
- Modify: `crates/db_view/src/lib.rs`
- Reference: `crates/db_view/src/database_view_plugin.rs`

- [ ] Replace both uses of `cx.global::<DatabaseViewPluginRegistry>()` in
  `table_designer_tab.rs` (lines 319 and 1181) with
  `cx.global::<GlobalDbState>().get_plugin(&db_type)?`. Read
  `plugin.ui_manifest().capabilities` once at the top of `new()` / `render_options`.
- [ ] Map new capability fields:
  - `supports_engine` → `capabilities.supports_table_engine`
  - `supports_charset` → `capabilities.supports_table_charset`
  - `supports_collation` → `capabilities.supports_table_collation`
  - `supports_auto_increment` → `capabilities.supports_auto_increment`
  - `supports_tablespace` → `capabilities.supports_tablespace`
- [ ] For the column editor, pass
  `ColumnEditorCapabilities` derived from the manifest (until Task 10 removes the
  legacy struct; keep a thin adapter `From<&DatabaseUiCapabilities>` for
  `ColumnEditorCapabilities`).
- [ ] Engines: `capabilities.table_engines` drops into `EngineSelectItem` without any
  view plugin lookup.
- [ ] Preserve current rendering behavior for engine, charset, collation,
  auto_increment, tablespace, unsigned, and enum-value controls.
- [ ] Add regression tests:
  - For each built-in database, `TableDesignerConfig` → visible controls matches a
    fixture of the pre-migration output.

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`.
Expected: table designer no longer depends on `DatabaseViewPluginRegistry`.

Manual smoke:
- Open table designer for MySQL — engine, charset, collation, auto_increment visible.
- Open for PostgreSQL — tablespace visible, no engine.
- Open for SQLite — minimal options, no engine/charset.

### Task 8: Replace context menus and toolbars with manifest-driven renderers

**Files:**
- Modify: `crates/db_view/src/db_tree_view.rs`
- Modify: `crates/db_view/src/db_tree_event.rs`
- Modify: `crates/db_view/src/database_objects_tab.rs`
- Reference: `crates/db_view/src/action_event_mapper.rs`

- [ ] Replace `db_tree_event.rs:2616` `plugin.build_context_menu(node_id, node.node_type)`
  with `ManifestMenuBuilder::build(...)` using
  `cx.global::<GlobalDbState>().get_plugin(&node.database_type)?.ui_manifest().actions`.
- [ ] Replace `database_objects_tab.rs:804` `plugin.build_toolbar_buttons(...)` with
  `ManifestToolbarBuilder::build(...)`.
- [ ] Route menu clicks through `ActionEventMapper::to_tree_event` → emit
  `DbTreeViewEvent`. Route toolbar clicks through
  `ActionEventMapper::to_objects_event` → emit `DatabaseObjectsEvent`.
- [ ] Preserve multi-row handling: the toolbar builder respects
  `DatabaseActionToolbarScope::SelectedRow` and maps to
  `DatabaseObjectsBatchAction` when the mapper returns an id that is batch-eligible
  (delete-table, delete-view, etc.).
- [ ] Preserve label text, icons, separators, and active-connection restrictions.
- [ ] Add tests:
  - For MySQL and PostgreSQL: `ManifestMenuBuilder` output for a connection node, a
    database node, a table node matches the previous `build_context_menu` output by
    label sequence and event kinds.
  - Similarly for `ManifestToolbarBuilder`.

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`, `cargo test -p db_view`.
Expected: menus and toolbars render from action manifests; fixtures unchanged.

Manual smoke:
- Right-click a MySQL connection, database, table, view, named query — verify every
  action is present and in the same order.
- Right-click a PostgreSQL schema — verify `CreateNewQuery` and `DeleteSchema`.
- Right-click a ClickHouse database — verify actions match previous behavior.
- Toolbar: open the database objects tab for MySQL and PostgreSQL and confirm buttons
  unchanged.

### Task 9: Replace connection form selection with generic manifests

**Files:**
- Modify: `crates/db_view/src/connection_form_window.rs`
- Reference: `main/src/home_tab.rs`
- Reference: `crates/db_view/src/common/db_connection_form.rs`

- [ ] Remove `DatabaseViewPluginRegistry` usage from `ConnectionFormWindow`.
- [ ] Resolve `DatabasePlugin` through `GlobalDbState::get_plugin` and request the
  `Connection` form manifest.
- [ ] Instantiate `GenericConnectionForm` with the manifest + plugin.
- [ ] Preserve behavior:
  - load-existing-connection (use `ConnectionFormBinding::from_stored_connection`).
  - test-connection button.
  - save-connection (emit `DbConnectionFormEvent::Saved` with `StoredConnection`).
  - workspace + team selection (these are shell-level, not manifest-driven).
  - error notification text.
- [ ] Add targeted tests:
  - `ConnectionFormBinding::from_state_map` round-trip for every built-in database.
  - SSH-tunnel visibility toggle flips visibility for all 9 SSH fields.

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`, `cargo test -p db_view connection_form`.
Expected: connection creation/edit no longer depends on view plugins.

Manual smoke:
- New-connection dialog for MySQL, PostgreSQL, MSSQL, Oracle, SQLite, ClickHouse,
  DuckDB. All tabs, defaults, placeholders, required markers match.
- Edit an existing MySQL connection — all persisted fields rehydrate correctly.
- Test-connection still reports success/failure for a localhost MySQL instance.

### Task 10: Replace create/edit database and create-schema dialogs

**Files:**
- Modify: `crates/db_view/src/db_tree_event.rs`
- Modify: `crates/db_view/src/common/database_editor_view.rs`
- Modify: `crates/db_view/src/common/schema_editor_view.rs`
- Reference: `crates/db_view/src/*/database_form.rs` (inputs to the manifest)
- Reference: `crates/db_view/src/*/schema_form.rs`

- [ ] Remove database-specific editor view construction from `db_tree_event.rs` lines
  1147, 1286, 1563.
- [ ] For `handle_create_database`, resolve the plugin, request
  `DatabaseFormKind::CreateDatabase`, and open a generic dialog wrapping
  `GenericDatabaseOperationForm`.
- [ ] For `handle_edit_database`, request `DatabaseFormKind::EditDatabase` and pass
  the current database name as initial state; the manifest's `disabled_when_editing`
  handles the disabled name field.
- [ ] For `handle_create_schema`, request `DatabaseFormKind::CreateSchema`. If the
  plugin returns no such form, show the current "unsupported" notification.
- [ ] Keep request construction: map `DatabaseFormSubmission.field_values` into
  `DatabaseOperationRequest { database_name, field_values }` via a small helper.
- [ ] Preserve current validation: empty-SQL warning, save-error display, dialog
  titles, Create/Edit labels, refresh-tree-on-success behavior.
- [ ] Add targeted tests:
  - Create-database submit produces `DatabaseOperationRequest` identical to the one
    previously produced by `MySqlDatabaseForm::build_request`.
  - Edit-database prefills the name and disables the field.
  - Create-schema submit produces the previous schema request for PostgreSQL and MSSQL.

Run: `cargo fmt --check`, `cargo clippy -p db_view -- -D warnings`,
`cargo check -p db_view`, `cargo test -p db_view`.
Expected: database/schema dialogs no longer instantiate per-database GPUI form types.

Manual smoke:
- Create a MySQL database with charset `utf8mb4` + collation `utf8mb4_unicode_ci` —
  verify SQL preview, submit, tree refresh.
- Edit a PostgreSQL database — name disabled, owner/tablespace editable as before.
- Create a PostgreSQL schema with comment — verify SQL preview and submit.
- Create an MSSQL schema with authorization — verify the same.

### Task 11: Remove obsolete view-plugin types and clean module graph

**Files:**
- Delete: `crates/db_view/src/database_view_plugin.rs`
- Delete: `crates/db_view/src/mysql/mysql_view_plugin.rs`
- Delete: `crates/db_view/src/postgresql/postgresql_view_plugin.rs`
- Delete: `crates/db_view/src/mssql/mssql_view_plugin.rs`
- Delete: `crates/db_view/src/oracle/oracle_view_plugin.rs`
- Delete: `crates/db_view/src/sqlite/sqlite_view_plugin.rs`
- Delete: `crates/db_view/src/clickhouse/clickhouse_view_plugin.rs`
- Delete: `crates/db_view/src/duckdb/duckdb_view_plugin.rs`
- Delete: `crates/db_view/src/mysql/database_form.rs`
- Delete: `crates/db_view/src/postgresql/database_form.rs`
- Delete: `crates/db_view/src/postgresql/schema_form.rs`
- Delete: `crates/db_view/src/mssql/database_form.rs`
- Delete: `crates/db_view/src/mssql/schema_form.rs`
- Delete: `crates/db_view/src/oracle/database_form.rs`
- Delete: `crates/db_view/src/clickhouse/database_form.rs`
- Delete: `crates/db_view/src/common/db_connection_form.rs` (replaced by
  `GenericConnectionForm`)
- Modify: `crates/db_view/src/lib.rs`
- Modify: `crates/db_view/src/mysql/mod.rs`, `postgresql/mod.rs`, `mssql/mod.rs`,
  `oracle/mod.rs`, `sqlite/mod.rs`, `clickhouse/mod.rs`, `duckdb/mod.rs`
- Modify: `main/src/main.rs` (remove `DatabaseViewPluginRegistry` init)

- [ ] Delete files listed above only after confirming `rg` reports zero remaining
  references to each symbol (`DatabaseViewPlugin`, `DatabaseViewPluginRegistry`,
  `MySqlDatabaseForm`, `PostgreSqlDatabaseForm`, ..., `DbConnectionForm`).
- [ ] Remove obsolete module exports and imports from `db_view`.
- [ ] Remove `DatabaseViewPluginRegistry::new()` and its `cx.set_global()` call from
  `main/src/main.rs`.
- [ ] If any file in the deletion list is still imported from `crates/db_view/src/lib.rs`,
  stop and audit before proceeding — a missed call site means an earlier task was
  incomplete.
- [ ] Remove legacy `ColumnEditorCapabilities`/`TableDesignerCapabilities` structs from
  `db_view` (if they were kept only as adapters in Task 7, delete them too).
- [ ] Update doc comments so `db_view` no longer claims to own per-database view
  plugins.

Run: `cargo fmt --check`, `cargo clippy -p db -p db_view -p main -- -D warnings`,
`cargo check -p db -p db_view -p main`, `cargo test -p db -p db_view`,
`cargo machete` (confirm no unused dependencies were introduced).
Expected: the old view-plugin layer is fully removed from the crate graph.

### Task 12: Documentation update

**Files:**
- Modify: `crates/db/README.md`
- Modify: `crates/db_view/README.md`
- Modify: `AGENTS.md` (if it mentions the view-plugin split)
- Modify: `CLAUDE.md` (update feature-crate table entries if needed)
- Optional: add `docs/superpowers/specs/2026-04-21-db-plugin-manifest-design.md` to
  a docs index if one exists

- [ ] Update `crates/db/README.md` to document:
  - `plugin_manifest.rs` contents.
  - How to add a new database plugin: implement `DatabasePlugin` + `ui_manifest()`.
  - Where reference-data accessors live.
- [ ] Update `crates/db_view/README.md` (if present) to document:
  - Generic renderer responsibilities.
  - `ActionEventMapper` as the single seam between manifest IDs and GPUI events.
  - That `db_view` no longer contains per-DB plugin code.
- [ ] Update `CLAUDE.md` feature-crate table: `crates/db_view` no longer says "View crate
  with per-database plugin logic".
- [ ] Update `AGENTS.md` where it references the old view-plugin pattern.
- [ ] Cross-link the design + plan docs from the updated READMEs.

Run: `cargo fmt --check`, `cargo doc -p db -p db_view --no-deps` (verify doc-comment
links resolve).
Expected: documentation describes the manifest-driven architecture as the current state.

### Task 13: Verification, formatting, and regression sweep

**Files:**
- Any file touched by Tasks 1–12 for formatting fixes

- [ ] Run `cargo fmt --all`.
- [ ] Run `cargo clippy --workspace -- -D warnings`.
- [ ] Run `cargo check --workspace`.
- [ ] Run `cargo test -p db -p db_view -p main` and targeted tests added by this plan:
  - Manifest unit tests (`cargo test -p db plugin_manifest::`).
  - MySQL / PostgreSQL manifest structural tests.
  - `FormVisibilityEvaluator`, `FormDefaultBridge`.
  - `ConnectionFormBinding` round-trip for every database type.
  - `ActionEventMapper` exhaustiveness.
  - `ManifestMenuBuilder` + `ManifestToolbarBuilder` fixture tests.
  - Table-designer capability visibility regressions.
- [ ] Run `cargo machete` — no unused dependencies.
- [ ] Final manual smoke pass covering:
  - New connection dialog for MySQL, PostgreSQL, MSSQL, Oracle, SQLite, ClickHouse,
    DuckDB (tab order, placeholders, SSH tunnel toggle).
  - Edit connection for an existing MySQL entry (all fields rehydrate).
  - Create/edit database for MySQL (charset → collation dependency works).
  - Create schema for PostgreSQL and MSSQL (titles and submit button text match).
  - Context menu + toolbar for MySQL, PostgreSQL, ClickHouse across connection,
    database, table, view, named-query nodes.
  - Table designer for MySQL + PostgreSQL (engine + tablespace visibility correct).
  - Run SQL file on a MySQL database — same behavior as before migration.
  - Import + export on a MySQL table — same behavior as before migration.
- [ ] File any regressions as follow-up issues; do not close the migration while any
  regression is unresolved.

Run: `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo check --workspace && cargo test -p db -p db_view`
Expected: both crates compile cleanly, all tests pass, no clippy warnings, no manual
regressions.

---

## Rollback strategy

Each task is independently revertible because no task touches both the manifest side
and the call-site side simultaneously:

- Tasks 1–4 add new code and do not change call sites; revertible by removing the new
  manifest module.
- Tasks 5–6 add new renderers and mappers; revertible by deleting the new files.
- Tasks 7–10 swap individual call sites; each can be reverted to the previous view
  plugin call by reintroducing the corresponding `DatabaseViewPlugin` method lookup.
- Task 11 is the only destructive step; keep it in its own commit so it can be reverted
  as a unit if a regression slips through Tasks 7–10's smoke tests.

## Out-of-scope follow-ups

Not part of this plan — filed for Stage 2:

- IPC transport protocol selection and wire format.
- Plugin process supervisor, crash recovery, sandboxing.
- Custom widget extension points (Stage 1 deliberately limits field kinds).
- Hot-reload of plugin manifests.
- Plugin permission model (which plugins can create/drop databases?).

These will consume the same `DatabaseUiManifest` contract delivered by this plan.
