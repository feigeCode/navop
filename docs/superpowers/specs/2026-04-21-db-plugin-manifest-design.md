# Database Plugin Manifest Design

## Summary

This design removes the `db_view`-side database view plugin registry and replaces it
with a **manifest-driven integration model** owned by `crates/db`.

After the change, database-specific differences are expressed as **serializable metadata,
capability declarations, and stable action identifiers** returned by `DatabasePlugin`.
`crates/db_view` becomes a unified GPUI shell that renders forms, menus, toolbars, and
conditional panels from that metadata instead of instantiating per-database view plugins.

This is the first stage of a two-stage architecture:

- **Stage 1 (this design):** collapse the dual-plugin system (`DatabasePlugin` +
  `DatabaseViewPlugin`) into a single plugin that returns pure data for UI integration.
- **Stage 2 (follow-up):** introduce an IPC-backed plugin adapter so external plugins
  (e.g. written in Go) can participate in the same manifest contract without any UI code.

## Scope

This design covers:

- Removing `DatabaseViewPluginRegistry` from the main runtime path.
- Moving database-specific UI differences into `crates/db/src/plugin.rs`.
- Replacing per-database connection/database/schema forms with manifest-driven forms.
- Replacing per-database context-menu and toolbar generation with manifest-driven actions.
- Moving table-designer and column-editor visibility decisions to database-plugin
  capabilities.
- Defining a **stable, serializable boundary** that can later be implemented by an IPC
  adapter.
- Defining dynamic form behavior (visibility rules, computed defaults, cross-field
  dependencies) using declarative primitives.

Out of scope for this stage:

- Implementing external plugin-process management.
- Finalizing an IPC wire protocol (JSON/MessagePack/gRPC) or transport.
- Supporting arbitrary custom widgets in plugin-provided UI.
- Refactoring unrelated database execution, caching, or SQL-editor code.
- Rewriting Redis / MongoDB views — they are separate view modules.

## Problem Statement

The current design splits database extensibility across two layers:

- `crates/db` owns `DatabasePlugin` and database runtime operations.
- `crates/db_view` owns `DatabaseViewPlugin` and database-specific GPUI rendering.

That split introduces five structural problems:

1. Database-specific UI logic is hard-coded inside Rust/GPUI view plugin types such as
   `mysql_view_plugin.rs` and `postgresql_view_plugin.rs`.
2. `db_view` chooses behavior by database type instead of by declared capabilities.
3. Future IPC plugins cannot participate in UI integration because they cannot return
   GPUI objects like `Entity<DbConnectionForm>` or function pointers.
4. New database support requires touching both the database runtime and the UI registry,
   which prevents a clean plugin model.
5. Capability data is scattered: the trait has `supports_schema()` / `supports_sequences()`
   in `db`, and `TableDesignerCapabilities` / `ColumnEditorCapabilities` in `db_view`.
   There is no single authoritative picture of what a database supports.

The result is that the current "plugin" system is only an internal Rust extension
pattern, not a reusable database-plugin architecture.

## Current Implementation Snapshot

Today the main call chain looks like this:

1. `main/src/main.rs` creates and registers `DatabaseViewPluginRegistry` as a GPUI global.
2. `ConnectionFormWindow::new` at `crates/db_view/src/connection_form_window.rs:54` resolves
   a view plugin and calls `create_connection_form`.
3. `db_tree_event.rs` resolves a view plugin at three sites and calls:
   - `:1147` → `create_database_editor_view`
   - `:1286` → `create_database_editor_view_for_edit`
   - `:1563` → `create_schema_editor_view`
4. `db_tree_event.rs:2616` asks the view plugin to build a context menu.
5. `database_objects_tab.rs:804` asks the view plugin to build toolbar buttons.
6. `table_designer_tab.rs:319` and `:1181` ask the view plugin for engines, column editor
   capabilities, and table designer capabilities.

The view-plugin trait mixes three unrelated responsibility types:

- **GPUI component construction** — returns `Entity<DbConnectionForm>` / `Entity<DatabaseEditorView>`.
- **Declarative capability differences** — booleans and lists of strings.
- **Action/menu wiring** — function pointers and GPUI event enums.

Only the last two are suitable for long-term pluginization. The first must be replaced
by data-driven rendering.

### Existing declarative seeds

The codebase already has declarative building blocks that can be evolved:

- `DbFormConfig`, `TabGroup`, `FormField`, `FormFieldType` in
  `crates/db_view/src/common/db_connection_form.rs` already describe connection forms
  as data (tabs + fields + options + defaults).
- `DatabaseOperationRequest { database_name, field_values: HashMap<String, String> }` in
  `crates/db/src/plugin.rs` already models form output as a normalized payload.
- `DatabasePlugin` already exposes reference data via `get_charsets()` and
  `get_collations(charset)`.

The migration should **promote** these seeds into first-class types owned by `crates/db`,
not invent new shapes.

## Design Goals

1. Make `crates/db` the single source of truth for database-specific differences.
2. Ensure all UI-facing plugin data is serializable and expressible without GPUI types,
   closures, or Rust function pointers.
3. Keep `db_view` as a reusable renderer and dispatcher, not a database-specific layer.
4. Preserve existing product behavior during migration (same labels, same defaults, same
   keyboard and dialog flow).
5. Create a boundary that can later be satisfied by both built-in Rust plugins and
   external IPC plugins — without changing `db_view`.
6. Keep the manifest narrow: only add field kinds and action types already needed by
   current built-in databases. Avoid premature generalization.

## Chosen Architecture

### 1. `DatabasePlugin` owns UI metadata

`crates/db/src/plugin.rs` adds a manifest-oriented API surface.

Recommended additions:

- `DatabaseUiManifest` — top-level aggregate with a `schema_version` field.
- `DatabaseUiCapabilities` — pure-data capability booleans and lists.
- `DatabaseFormKind` — `Connection | CreateDatabase | EditDatabase | CreateSchema`.
- `DatabaseFormManifest` — fields, tabs, title key, submit key.
- `DatabaseFormField`, `DatabaseFormFieldType`, `FormSelectOption`.
- `FormVisibilityRule` — declarative cross-field visibility.
- `FormDefaultRule` — declarative computed defaults.
- `DatabaseActionManifest` — declared actions with grouping.
- `DatabaseActionDescriptor`, `DatabaseActionId`, `DatabaseActionTarget`.
- `DatabaseReferenceDataKind` — runtime-queried catalogs (charsets, collations, engines).

The plugin returns **pure data structures**, never GPUI types. The only dynamic surface
is reference-data providers (see section 5) that still live on the plugin trait because
they return catalog information that may depend on server state.

### 2. `db_view` renders generic UI from manifests

`crates/db_view` provides a small set of generic renderers:

- `GenericConnectionForm` — renders `DatabaseFormManifest { kind: Connection }`.
- `GenericDatabaseOperationForm` — renders create/edit-database and create-schema forms.
- `ManifestMenuBuilder` — renders context menus from `DatabaseActionManifest`.
- `ManifestToolbarBuilder` — renders toolbars from `DatabaseActionManifest`.
- `ManifestFieldRenderer` — maps `DatabaseFormFieldType` to the corresponding GPUI
  control (Input, Select, Password, TextArea, Checkbox, FilePath).

These renderers understand a **bounded** set of field kinds and action types. That keeps
the system predictable and avoids turning the UI layer into an unbounded schema
interpreter.

### 3. Database actions are declared, not hand-built

Context menus and toolbar buttons are generated from a structured action manifest.
The plugin declares:

- which actions exist and which node types they apply to.
- grouping / separator information.
- whether an action requires an active connection.
- icon identifiers and i18n keys.
- whether an action belongs to "current node" or "selected row" semantics (toolbar only).

A small `ActionEventMapper` in `db_view` converts `DatabaseActionId` to either a
`DbTreeViewEvent` (context menus) or a `DatabaseObjectsEvent` (toolbars). This mapper is
the only place that knows about GPUI event types — plugins never see them.

### 4. Forms are schema-driven, not component-driven

Connection, create-database, edit-database, and create-schema flows are all described by
field schemas returned from the plugin. The UI renderer translates those schemas into
standard GPUI controls.

Forms emit a single normalized payload type (`DatabaseFormSubmission`) which composes
with the existing `DatabaseOperationRequest` for database-level operations.

This keeps the boundary future-proof for IPC: the plugin returns data such as field
names, labels, defaults, required flags, select options, visibility rules, and validation
hints — all serializable.

### 5. Reference data providers stay on the plugin

Some select-field contents are **dynamic** (server-dependent) or **large** (hundreds of
collations). Examples:

- MySQL: `charsets()` and `collations(charset)` are tables derived from the server or
  from a bundled catalog; `collations` depends on the currently selected charset.
- MSSQL: similar charset/collation pattern.
- MySQL: `engines()` is a static list.

Rather than inlining these into every form manifest, the plugin keeps a small set of
reference-data accessors:

- `fn charsets(&self) -> Vec<CharsetInfo>` (already exists)
- `fn collations(&self, charset: &str) -> Vec<CollationInfo>` (already exists)
- `fn engines(&self) -> Vec<String>` (currently in `db_view` view plugin — move here)
- Future: `fn reference_data(&self, kind: DatabaseReferenceDataKind, ctx: &HashMap<String,String>) -> Vec<FormSelectOption>`
  as a stable catch-all for IPC plugins.

The `GenericConnectionForm` renderer resolves a field marked with `options_source:
ReferenceDataKind::MySqlCharsets` by calling the plugin at render time, not at manifest
build time. This keeps the manifest small while still letting IPC plugins serve dynamic
catalogs through a serializable request/response.

### 6. Dynamic form behavior via declarative rules

Forms need **three** kinds of dynamic behavior today:

1. **Conditional visibility** — e.g. SSH tunnel fields only appear when
   `ssh_tunnel_enabled == true`.
2. **Computed defaults / dependent options** — e.g. selecting a MySQL charset changes the
   collation list and picks the default collation.
3. **Edit-mode locking** — e.g. database name is disabled when editing an existing
   database.

All three are expressed declaratively:

- `FormVisibilityRule { field: String, equals: Option<String>, not_equals: Option<String> }`
- `FormDefaultRule { when_field_changes: String, recompute_default_for: String, via: ReferenceDataKind }`
- `disabled_when_editing: bool` on every field.

The renderer interprets these rules at runtime. The plugin never executes user-defined
code across the boundary.

## Proposed Core Types

The exact names can change, but the separation must look like this. All types derive
`Serialize`/`Deserialize` so they can cross an IPC boundary unchanged.

```rust
// crates/db/src/plugin_manifest.rs (new module; re-exported from plugin.rs)

pub const DATABASE_UI_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseUiManifest {
    pub schema_version: u32,
    pub capabilities: DatabaseUiCapabilities,
    pub forms: Vec<DatabaseFormManifest>,
    pub actions: DatabaseActionManifest,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DatabaseUiCapabilities {
    // Tree / object model
    pub supports_schema: bool,
    pub uses_schema_as_database: bool,
    pub supports_sequences: bool,
    pub supports_functions: bool,
    pub supports_procedures: bool,
    pub supports_triggers: bool,
    // Table designer
    pub supports_table_engine: bool,
    pub supports_table_charset: bool,
    pub supports_table_collation: bool,
    pub supports_auto_increment: bool,
    pub supports_tablespace: bool,
    // Column editor
    pub supports_unsigned: bool,
    pub supports_enum_values: bool,
    pub show_charset_in_column_detail: bool,
    pub show_collation_in_column_detail: bool,
    // Static lists
    pub table_engines: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseFormKind {
    Connection,
    CreateDatabase,
    EditDatabase,
    CreateSchema,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseFormManifest {
    pub kind: DatabaseFormKind,
    pub title_i18n_key: String,
    pub submit_i18n_key: String,
    pub tabs: Vec<DatabaseFormTab>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseFormTab {
    pub id: String,                    // "general", "advanced", "ssl", "ssh", "notes"
    pub label_i18n_key: String,
    pub fields: Vec<DatabaseFormField>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseFormField {
    pub id: String,
    pub label_i18n_key: String,
    pub field_type: DatabaseFormFieldType,
    pub required: bool,
    pub default_value: Option<String>,
    pub placeholder_i18n_key: Option<String>,
    pub help_i18n_key: Option<String>,
    pub options: Vec<FormSelectOption>,
    pub options_source: Option<ReferenceDataKind>,
    pub visible_when: Vec<FormVisibilityRule>,
    pub default_when: Vec<FormDefaultRule>,
    pub disabled_when_editing: bool,
    pub rows: Option<u32>,             // for TextArea
    pub min: Option<i64>,              // for Number
    pub max: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseFormFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    FilePath,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormSelectOption {
    pub value: String,
    pub label_i18n_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormVisibilityRule {
    pub when_field: String,
    pub condition: FormValueCondition,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FormValueCondition {
    Equals(String),
    NotEquals(String),
    In(Vec<String>),
    NotEmpty,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormDefaultRule {
    /// When the value of this field changes, recompute the owner field's default.
    pub when_field_changes: String,
    pub via: ReferenceDataKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceDataKind {
    MySqlCharsets,
    MySqlCollations,     // depends on current charset value
    MsSqlCharsets,
    MsSqlCollations,
    TableEngines,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DatabaseActionManifest {
    pub actions: Vec<DatabaseActionDescriptor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseActionDescriptor {
    pub id: DatabaseActionId,
    pub label_i18n_key: String,
    pub icon: Option<String>,          // resolved to `IconName` in db_view
    pub targets: Vec<DatabaseActionTarget>,
    pub placement: DatabaseActionPlacement,
    pub requires_active_connection: bool,
    pub group: Option<String>,         // for menu separators
    pub submenu_of: Option<DatabaseActionId>,
    pub toolbar_scope: Option<DatabaseActionToolbarScope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseActionId {
    // Connection
    CloseConnection,
    DeleteConnection,
    // Database / Schema
    CreateDatabase,
    EditDatabase,
    CloseDatabase,
    DeleteDatabase,
    CreateSchema,
    DeleteSchema,
    // Table
    OpenTableData,
    DesignTable,
    RenameTable,
    CopyTable,
    TruncateTable,
    DeleteTable,
    // View
    OpenViewData,
    DeleteView,
    // Query
    CreateNewQuery,
    OpenNamedQuery,
    RenameQuery,
    DeleteQuery,
    // Import / Export
    RunSqlFile,
    ImportData,
    ExportData,
    DumpSqlStructure,
    DumpSqlData,
    DumpSqlStructureAndData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseActionTarget {
    pub node_type: DbNodeType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseActionPlacement {
    ContextMenu,
    Toolbar,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseActionToolbarScope {
    CurrentNode,
    SelectedRow,
}

/// Normalized payload emitted by generic forms on submit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseFormSubmission {
    pub kind: DatabaseFormKind,
    pub field_values: HashMap<String, String>,
}
```

The essential requirement is not exact syntax but the boundary: these types must remain
plain data, `Serialize + Deserialize`, free of UI framework objects.

### Plugin trait extension

```rust
pub trait DatabasePlugin: Send + Sync {
    // ... existing methods ...

    fn ui_manifest(&self) -> DatabaseUiManifest;

    /// Resolve a reference-data catalog at render time.
    /// `context` holds the current form state so `MySqlCollations` can read
    /// the selected charset.
    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption> {
        let _ = (kind, context);
        Vec::new()
    }
}
```

Existing boolean helpers (`supports_schema`, `uses_schema_as_database`, etc.) remain on
the trait and are **also** projected into `DatabaseUiCapabilities` by the default
`ui_manifest()` implementation to avoid duplication bugs.

## Form Strategy

### Connection forms

The current `DbConnectionForm` already contains a declarative core:

- `DbFormConfig`, `FormField`, `TabGroup`, default values.

Migration:

1. Move the *type definitions* of the schema types (as `DatabaseFormManifest` + friends)
   into `crates/db`.
2. Keep the **renderer** implementation in `crates/db_view`, renamed to
   `GenericConnectionForm`.
3. Each plugin's `ui_manifest()` returns a `DatabaseFormManifest { kind: Connection }`
   built from a small DSL internal to the plugin (mirroring today's
   `DbFormConfig::mysql()` / `DbFormConfig::postgres()` factories).
4. The renderer maps manifest field values ↔ `DbConnectionConfig` using a dedicated
   `ConnectionFormBinding` module in `db_view` that already exists in spirit.

### Database forms

Current per-database database forms:

- `mysql/database_form.rs` (charset + collation + name)
- `mssql/database_form.rs`
- `oracle/database_form.rs`
- `clickhouse/database_form.rs`

Replace with:

- One `GenericDatabaseOperationForm` rendering `DatabaseFormManifest { kind: CreateDatabase | EditDatabase }`.
- Normalized output `DatabaseFormSubmission` → converted to existing
  `DatabaseOperationRequest { database_name, field_values }` inside the editor view.
- The charset → collation dynamic update is expressed via `FormDefaultRule` +
  `ReferenceDataKind::MySqlCollations`; the renderer subscribes to the charset field and
  re-queries the plugin.

### Schema forms

`postgresql/schema_form.rs` and `mssql/schema_form.rs` collapse into one
`GenericSchemaOperationForm` rendered from `DatabaseFormManifest { kind: CreateSchema }`.

## Action Strategy

Actions are promoted from ad-hoc closures into stable IDs. The UI ↔ plugin handshake:

1. Plugin declares `DatabaseActionDescriptor { id, targets, placement, ... }`.
2. UI filters descriptors by `(node_type, placement, state)` to decide visibility.
3. UI renders a standard menu item or toolbar button from the descriptor.
4. On click, `ActionEventMapper::to_tree_event(id, node_id)` or
   `to_objects_event(id, node)` produces the corresponding GPUI event.
5. Existing event handlers execute the action unchanged.

### Full action ID → event mapping (representative subset)

| DatabaseActionId        | → DbTreeViewEvent (menu) | → DatabaseObjectsEvent (toolbar) |
| ----------------------- | ------------------------ | -------------------------------- |
| `CloseConnection`       | `CloseConnection`        | `CloseConnection`                |
| `DeleteConnection`      | `DeleteConnection`       | `DeleteConnection`               |
| `CreateDatabase`        | `CreateDatabase`         | `CreateDatabase`                 |
| `EditDatabase`          | `EditDatabase`           | `EditDatabase`                   |
| `DeleteDatabase`        | `DeleteDatabase`         | `DeleteDatabase`                 |
| `CreateSchema`          | `CreateSchema`           | `CreateSchema`                   |
| `DeleteSchema`          | `DeleteSchema`           | `DeleteSchema`                   |
| `OpenTableData`         | `OpenTableData`          | `OpenTableData`                  |
| `DesignTable`           | `DesignTable`            | `DesignTable`                    |
| `RenameTable`           | `RenameTable`            | n/a                              |
| `CopyTable`             | `CopyTable`              | n/a                              |
| `TruncateTable`         | `TruncateTable`          | n/a                              |
| `DeleteTable`           | `DeleteTable`            | `DeleteTable`                    |
| `OpenViewData`          | `OpenViewData`           | `OpenViewData`                   |
| `DeleteView`            | `DeleteView`             | `DeleteView`                     |
| `CreateNewQuery`        | `CreateNewQuery`         | `CreateNewQuery`                 |
| `OpenNamedQuery`        | `OpenNamedQuery`         | `OpenNamedQuery`                 |
| `RenameQuery`           | `RenameQuery`            | `RenameQuery`                    |
| `DeleteQuery`           | `DeleteQuery`            | `DeleteQuery`                    |
| `RunSqlFile`            | `RunSqlFile`             | n/a                              |
| `ImportData`            | `ImportData`             | n/a                              |
| `ExportData`            | `ExportData`             | n/a                              |
| `DumpSqlStructure`      | `DumpSqlFile { StructureOnly }`  | n/a                      |
| `DumpSqlData`           | `DumpSqlFile { DataOnly }`       | n/a                      |
| `DumpSqlStructureAndData` | `DumpSqlFile { StructureAndData }` | n/a                   |

Batch actions (multi-row delete) stay in `DatabaseObjectsBatchAction`; the mapper
recognizes which action IDs are batch-eligible and routes `Vec<DbNode>` through
`DatabaseObjectsEvent::Batch`.

## Runtime Flow After Migration

### Connection-form flow

1. `ConnectionFormWindow` receives a `DatabaseType`.
2. It fetches the `DatabasePlugin` via `GlobalDbState::get_plugin`.
3. It requests `plugin.ui_manifest()` and selects the `Connection` form.
4. `GenericConnectionForm` renders fields, resolving `options_source` through
   `plugin.resolve_reference_data(...)` for dynamic selects.
5. On submit, the renderer returns a `DatabaseFormSubmission`.
6. `ConnectionFormBinding` converts it into `DbConnectionConfig`; existing persistence
   and test/save flows are unchanged.

### Database / schema operation flow

1. `db_tree_event.rs` decides the desired operation kind.
2. It fetches the plugin manifest for the node's database type.
3. It opens a generic dialog using the selected `DatabaseFormManifest`.
4. Submission yields `DatabaseFormSubmission` → adapted to `DatabaseOperationRequest`.
5. The existing `build_create_database_sql` / `build_modify_database_sql` /
   `build_create_schema_sql` path consumes the request unchanged.

### Context-menu / toolbar flow

1. `db_tree_view` / `database_objects_tab` call
   `plugin.ui_manifest().actions.filter(node_type, placement, state)`.
2. `ManifestMenuBuilder` / `ManifestToolbarBuilder` produce the GPUI components.
3. Clicks call `ActionEventMapper` → emit `DbTreeViewEvent` or `DatabaseObjectsEvent`.
4. Existing handlers in `db_tree_event.rs` execute the action.

### Table-designer flow

1. `TableDesigner::new` reads `plugin.ui_manifest().capabilities` for
   `supports_engine`, `supports_charset`, etc., and `capabilities.table_engines` for the
   engine list.
2. `render_options` uses the same capabilities to decide which controls to render.
3. Charsets/collations come from `plugin.charsets()` / `plugin.collations(charset)` as
   today.

### Reference-data flow

```text
renderer
  │
  ├─ static options ─► read `DatabaseFormField.options`
  │
  └─ dynamic options ─► plugin.resolve_reference_data(
                           kind,
                           {current form field values}
                        )
                        → Vec<FormSelectOption>
```

For cross-field dependencies, the renderer observes `FormDefaultRule` and re-queries the
plugin when the source field changes, then updates the target field's selection.

## IPC Boundary Preview (Stage 2)

The design must let a future IPC adapter slot in without any `db_view` changes. Sketch:

```rust
// crates/db/src/ipc_plugin.rs (future)

pub struct IpcDatabasePlugin {
    process: ChildProcess,
    cached_manifest: DatabaseUiManifest,
}

impl DatabasePlugin for IpcDatabasePlugin {
    fn ui_manifest(&self) -> DatabaseUiManifest {
        self.cached_manifest.clone() // fetched once on plugin attach
    }

    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption> {
        self.process.request(IpcRequest::ResolveReferenceData {
            kind,
            context: context.clone(),
        })
    }

    // ... other methods forward through IPC ...
}
```

If Stage 1 is implemented correctly, the IPC plugin only adds:

- A wire protocol (e.g. JSON over stdio or length-prefixed MessagePack).
- A process-supervisor.
- A manifest-caching layer (manifest is fetched once per plugin load).

`db_view` remains unchanged because it already speaks only in pure data.

## Manifest Caching & Lifecycle

- `DatabaseUiManifest` is returned by value. Built-in plugins construct it once per call
  using `LazyLock<DatabaseUiManifest>` inside each plugin module.
- `db_view` does **not** cache manifests itself; it calls `plugin.ui_manifest()` at the
  moment of render. This keeps manifests live-reloadable for hot-plugin scenarios in
  Stage 2.
- Reference data is cached inside the renderer per form instance only; switching the
  charset refetches collations.

## i18n Key Management

- All user-visible strings in the manifest are referenced by i18n key (e.g.
  `"ConnectionForm.host"`), not by literal text.
- `db_view` resolves keys through `rust_i18n::t!` at render time.
- Keys are documented in `crates/db/src/plugin_manifest.rs` near the relevant type, and
  enforced by a CI grep test that checks every key appears in `locales/*.yml`.
- Avoid moving translations to `crates/db` — only keys live there.

## Migration Plan (overview)

The migration happens in ordered slices. Each slice compiles and passes tests on its own
so the work can be committed incrementally and bisected.

1. Add manifest types & reference-data traits to `crates/db`.
2. Move `engines()` from `db_view` view plugin to `crates/db` plugin.
3. Implement `ui_manifest()` for built-in plugins, starting with MySQL as the canonical
   reference, then PostgreSQL, MSSQL, Oracle, SQLite, ClickHouse, DuckDB.
4. Add the generic field renderer and the `ActionEventMapper` in `db_view`.
5. Switch table-designer capability reads.
6. Switch context menus and toolbars.
7. Switch connection-form selection.
8. Switch database / schema editor dialogs.
9. Remove `DatabaseViewPluginRegistry` and per-DB `*_view_plugin.rs` files.
10. Update documentation (`crates/db/README.md`, `AGENTS.md`, architecture notes).
11. Run full validation sweep: `cargo fmt --check`, `cargo clippy -- -D warnings`,
    `cargo test -p db -p db_view`, manual smoke.

Detailed task breakdown lives in
`docs/superpowers/plans/2026-04-21-db-plugin-manifest-migration.md`.

## Compatibility Constraints

During migration:

- Keep behavior stable for existing built-in databases.
- Preserve current dialog titles, labels, and user-visible operations down to word order.
- Reuse existing request/SQL generation logic — only the form rendering changes.
- Avoid introducing IPC-specific abstractions into `db_view`.
- Keep tree-event and objects-event type signatures stable; only the **producer**
  switches from hand-coded per-DB logic to `ActionEventMapper`.
- Do not expose manifest types as public API outside `crates/db` until Stage 2.

The new types are designed so an IPC adapter can later translate a wire payload into the
same manifest structures without UI changes.

## Risks

1. **Dynamic select dependencies** — charset → collation is the primary example. The
   `FormDefaultRule` + `ReferenceDataKind` mechanism must be implemented and tested
   against MySQL before any other form is migrated.
2. **Batch actions** — today's toolbar uses function-pointer indirection to group
   multi-row deletes. The action mapper must preserve the `CurrentNode` vs
   `SelectedRow` distinction and the `DatabaseObjectsBatchAction` routing.
3. **Over-generalization** — extending the manifest beyond current needs (for example
   adding validation DSLs) would balloon the boundary surface. This stage limits the
   manifest to what existing UIs already do.
4. **Hidden GPUI assumptions** — some controls read GPUI state outside the form value
   map (e.g. password visibility toggle). The generic renderer must replicate this
   behavior via field-local state; no plugin code is involved.
5. **i18n key drift** — introducing keys in the manifest without updating locale files
   will produce missing-translation warnings. Validation step checks this.
6. **Existing `DbFormConfig` duplication** — during migration, both `DbFormConfig` and
   `DatabaseFormManifest` exist. The migration order (task 3 before tasks 7–8) minimizes
   the overlap window.
7. **Oracle connection form** — uses either `service_name` or `sid`, mutually exclusive
   and currently enforced at the GPUI layer. Must be expressible via `FormVisibilityRule`
   or an `at_least_one_of` validation primitive.
8. **DuckDB file-based connection** — the file-path field requires a native file dialog.
   `DatabaseFormFieldType::FilePath` must be part of the first-stage renderer.

## Validation

Required validation for the migration:

- `cargo fmt --check`
- `cargo clippy -p db -p db_view -- -D warnings`
- `cargo check -p db -p db_view -p main`
- Targeted `cargo test` coverage:
  - `crates/db/src/plugin_manifest.rs` unit tests for manifest defaults and invariants.
  - `crates/db_view` tests for:
    - generic connection form value mapping (round-trip with `DbConnectionConfig`).
    - generic database/schema operation form submission.
    - manifest-driven menu rendering (MySQL + PostgreSQL fixtures).
    - manifest-driven toolbar rendering (MySQL + PostgreSQL fixtures).
    - table-designer capability visibility rules (all 7 databases).
    - `FormVisibilityRule` evaluator.
    - `ActionEventMapper` coverage for every `DatabaseActionId` variant.
- Manual smoke pass:
  - Open a new-connection dialog for every database type and check tab order.
  - Open create-database dialog for MySQL (with charset + collation) and PostgreSQL.
  - Open create-schema dialog for PostgreSQL and MSSQL.
  - Right-click tree nodes on MySQL, PostgreSQL, ClickHouse — verify action presence is
    unchanged vs pre-migration.
  - Open table designer for MySQL (engine + charset + collation + auto_increment) and
    PostgreSQL (no engine, tablespace).

## Follow-up (Stage 2)

Stage 2 can introduce an IPC-backed plugin adapter:

- Built-in Rust plugins and external plugins both implement the same manifest contract.
- The adapter translates IPC responses into `DatabaseUiManifest`.
- Database operations are proxied through the same action/request model.
- A plugin discovery mechanism (directory scan + `plugin.json` manifest file) loads
  external binaries.
- Process lifecycle, crash recovery, and sandboxing policies are defined per-platform.

That stage should require **zero** changes in `db_view` if this design is implemented
correctly.

## Appendix A — Full MySQL Manifest Sketch

The MySQL plugin's `ui_manifest()` output, abbreviated:

```rust
DatabaseUiManifest {
    schema_version: 1,
    capabilities: DatabaseUiCapabilities {
        supports_schema: false,
        uses_schema_as_database: false,
        supports_sequences: false,
        supports_functions: true,
        supports_procedures: true,
        supports_triggers: true,
        supports_table_engine: true,
        supports_table_charset: true,
        supports_table_collation: true,
        supports_auto_increment: true,
        supports_tablespace: false,
        supports_unsigned: true,
        supports_enum_values: true,
        show_charset_in_column_detail: true,
        show_collation_in_column_detail: true,
        table_engines: vec!["InnoDB", "MyISAM", "MEMORY", "CSV",
                             "ARCHIVE", "BLACKHOLE", "FEDERATED"]
                             .into_iter().map(String::from).collect(),
    },
    forms: vec![
        connection_form_mysql(),     // from DbFormConfig::mysql() today
        create_database_form_mysql(),// name + charset (dynamic) + collation (dependent)
        edit_database_form_mysql(),  // name disabled, charset+collation editable
    ],
    actions: DatabaseActionManifest {
        actions: vec![
            // Connection-level
            action(DatabaseActionId::RunSqlFile, "ImportExport.run_sql_file",
                   &[DbNodeType::Connection, DbNodeType::Database]),
            action(DatabaseActionId::CloseConnection, "Connection.close_connection",
                   &[DbNodeType::Connection]).always_enabled(),
            // ... (see action mapping table above) ...
        ],
    },
}
```

## Appendix B — Call-site Inventory

To be kept in sync by the migration tasks:

| File                                                   | Line | Call                                                    |
| ------------------------------------------------------ | ---- | ------------------------------------------------------- |
| `main/src/main.rs`                                     | —    | `DatabaseViewPluginRegistry::new()` + `cx.set_global()` |
| `crates/db_view/src/connection_form_window.rs`         | 54   | `plugin_registry.get(&db_type).create_connection_form`  |
| `crates/db_view/src/db_tree_event.rs`                  | 1147 | `plugin.create_database_editor_view`                    |
| `crates/db_view/src/db_tree_event.rs`                  | 1286 | `plugin.create_database_editor_view_for_edit`           |
| `crates/db_view/src/db_tree_event.rs`                  | 1563 | `plugin.create_schema_editor_view`                      |
| `crates/db_view/src/db_tree_event.rs`                  | 2616 | `plugin.build_context_menu`                             |
| `crates/db_view/src/database_objects_tab.rs`           | 804  | `plugin.build_toolbar_buttons`                          |
| `crates/db_view/src/table_designer_tab.rs`             | 319  | `view_plugin.get_engines` + `get_column_editor_capabilities` |
| `crates/db_view/src/table_designer_tab.rs`             | 1181 | `plugin.get_table_designer_capabilities`                |

Each site is replaced in the migration plan, task by task.
