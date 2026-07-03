# Connection Import Wasm Extension Design

## Summary

This design rebuilds connection import as a **window-based import center** backed by
**Wasm connection-import extensions**. Importers no longer live as hard-coded
`ImportSourceKind` branches inside the application. Instead, extensions declare import
capabilities in their manifest, run parser logic in a Wasm Component, and access system
resources only through host-controlled APIs.

The product flow becomes:

1. Open a dedicated connection import window from the home page.
2. Choose one or more source applications.
3. Scan selected importers.
4. Preview extracted database and SSH records in one list.
5. Edit a record in the matching database or SSH editor window, or save it directly.
6. Persist selected records through the application's normal connection-save pipeline.

The importer is responsible for parsing and normalizing source data. The host is
responsible for permissions, platform paths, file reads, directory reads, keychain /
credential access, timeouts, and persistence.

## Scope

This design covers:

- Replacing the current import dialog with a dedicated import window.
- Replacing source-specific UI branching with an importer registry.
- Defining a stable connection import protocol for database and SSH records.
- Defining Wasm importer manifest contributions.
- Defining Host APIs for file, directory, secret, platform, and logging access.
- Defining preview, edit, direct-save, and batch-save behavior.
- Defining a migration path from current Rust importers to Wasm importers.

Out of scope for the first implementation stage:

- Migrating every existing importer to Wasm in one pass.
- Solving proprietary password decryption for sources that do not expose supported
  credential APIs.
- Letting importers provide arbitrary custom UI.
- Letting Wasm importers write to the local database directly.
- Supporting network downloads or remote scans inside importers.

## Problem Statement

The current connection import implementation is too tightly coupled:

1. `ImportSourceKind` is a hard-coded enum. Every new source touches core model code,
   dispatch code, UI filtering, icon mapping, and tests.
2. Database and SSH import paths are split into separate APIs such as
   `preview_connections` and `preview_ssh_connections`, which forces the UI to know
   which source produces which kind of record.
3. Import adapters live inside `crates/connection_importer`, so messy source formats
   such as Navicat plist/XML, DBeaver JSON, and Xshell sessions accumulate in one core
   crate.
4. The current dialog is too small for a multi-step workflow with source selection,
   scanning, preview, per-record status, editing, direct saving, and error details.
5. File access and credential access are implemented ad hoc per adapter instead of
   through a consistent permission boundary.

The result is not an extension mechanism. It is a collection of source-specific parsers
wired into the product with match statements.

## Design Goals

1. Add new importers without changing the import window UI.
2. Use Wasm Components for importer implementation and isolation.
3. Keep system capabilities behind Host APIs, not inside Wasm.
4. Support both database and SSH imports through one preview model.
5. Keep importers read-only. They return draft records; they never save directly.
6. Allow records to be edited in type-specific editor windows before saving.
7. Support direct save for valid preview records.
8. Keep password import explicit, status-bearing, and platform-safe.
9. Preserve a staged migration path so current Rust importers can be wrapped first and
   moved to Wasm incrementally.

## Chosen Architecture

Use a two-layer importer system:

- **Importer Registry**: The application-facing registry that lists all importers and
  runs scan / preview. The UI only talks to this registry.
- **Importer Providers**: Backends that supply importers. The first implementation can
  include a `LegacyRustImporterProvider` for existing code and a `WasmImporterProvider`
  for extension-provided importers.

The long-term path is Wasm-first. The short-term migration keeps current importers
usable while the UI and protocol are stabilized.

### Why Not Direct Wasm-Only Migration

Moving every importer to Wasm immediately would combine protocol design, runtime
integration, UI replacement, parser migration, and credential behavior into one large
change. The safer path is:

1. Introduce the new protocol and window.
2. Wrap current importers behind the same registry.
3. Add Wasm importer support through the same registry.
4. Migrate first-party importers one by one.

## Protocol Model

The protocol is pure data and must be serializable. It is shared by Rust providers,
Wasm providers, the preview UI, editor windows, and save pipeline.

### Importer Descriptor

```rust
pub struct ImporterDescriptor {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub vendor: Option<String>,
    pub supported_platforms: Vec<Platform>,
    pub output_kinds: Vec<ImportRecordKind>,
    pub capabilities: ImporterCapabilities,
}

pub enum ImportRecordKind {
    Database,
    Ssh,
}

pub struct ImporterCapabilities {
    pub supports_scan: bool,
    pub supports_password_import: bool,
    pub supports_manual_file_pick: bool,
    pub supports_incremental_preview: bool,
}
```

`id` is the stable identifier used by UI state, logs, save statuses, and duplicate
detection. It replaces `ImportSourceKind` for new code.

### Scan Report

```rust
pub struct ImportScanReport {
    pub importer_id: String,
    pub availability: ImporterAvailability,
    pub discovered_files: Vec<DiscoveredFile>,
    pub warnings: Vec<ImportWarning>,
}

pub enum ImporterAvailability {
    Available { estimated_count: Option<u32> },
    Installed,
    NotInstalled,
    NoData,
    PermissionRequired,
    UnsupportedPlatform,
    Error { message: String },
}
```

Scan is allowed to report partial success. A broken importer must not block other
importers in the same window.

### Preview Record

```rust
pub struct ImportRecord {
    pub id: String,
    pub importer_id: String,
    pub source_label: String,
    pub kind: ImportRecordKind,
    pub display_name: String,
    pub database: Option<DatabaseImportRecord>,
    pub ssh: Option<SshImportRecord>,
    pub password_status: PasswordImportStatus,
    pub warnings: Vec<ImportWarning>,
}
```

Exactly one of `database` or `ssh` must be present and must match `kind`.

### Database Record

```rust
pub struct DatabaseImportRecord {
    pub database_type: DatabaseType,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub extra_params: BTreeMap<String, String>,
}
```

### SSH Record

```rust
pub struct SshImportRecord {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_method: SshImportAuthMethod,
}

pub enum SshImportAuthMethod {
    Password { password: Option<String> },
    PrivateKey { key_path: String, passphrase: Option<String> },
    Agent,
    AutoPublicKey,
}
```

## Extension Manifest

Connection importers should be contributed by existing composite extensions instead of
introducing a separate extension installation system.

Recommended manifest shape:

```json
{
  "schema_version": 1,
  "id": "com.onetcli.importer.navicat",
  "name": "Navicat Importer",
  "version": "0.1.0",
  "engines": {
    "onetcli": ">=0.7.0"
  },
  "runtime": {
    "wasm": [
      {
        "id": "navicat-importer",
        "module": "navicat_importer.wasm",
        "kind": "component",
        "timeout_ms": 5000,
        "max_memory_mb": 64
      }
    ]
  },
  "contributes": {
    "connectionImporters": [
      {
        "id": "navicat",
        "displayName": "Navicat",
        "description": "Import database connections from Navicat",
        "icon": "database",
        "outputKinds": ["database"],
        "platforms": ["macos", "windows", "linux"],
        "candidateFiles": [
          {
            "id": "navicat-macos-cc-conn",
            "platform": "macos",
            "path": "~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"
          },
          {
            "id": "navicat-macos-export",
            "platform": "macos",
            "path": "~/Documents/Navicat/connections.ncx"
          }
        ]
      }
    ]
  },
  "permissions": [
    "fs:read:~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist",
    "fs:read:~/Documents/Navicat/connections.ncx"
  ]
}
```

The manifest declares candidate files and required permissions. The Wasm module receives
file bytes only through Host API calls that validate those declarations.

## Wasm Contract

The Wasm interface should be defined with WIT and versioned as
`onet:connection-import@1.0.0`.

Conceptual interface:

```wit
package onet:connection-import@1.0.0;

interface importer {
  descriptor: func() -> importer-descriptor;
  scan: func() -> result<scan-report, import-error>;
  preview: func(options: import-options) -> result<list<import-record>, import-error>;
}
```

Importer functions must be deterministic for the same Host API responses. They must not
write to the application database or mutate source application data.

## Host API

The Host API is the only way a Wasm importer can access local resources.

Conceptual interface:

```wit
interface host {
  current-platform: func() -> platform;
  list-candidate-files: func(importer-id: string) -> list<candidate-file>;
  read-file: func(candidate-id: string) -> result<list<u8>, host-error>;
  read-directory: func(candidate-id: string) -> result<list<directory-entry>, host-error>;
  read-secret: func(query: secret-query) -> secret-result;
  log: func(level: string, message: string);
}
```

### File Access Rules

- `read-file` and `read-directory` accept candidate identifiers, not arbitrary paths.
- Candidate identifiers must come from the extension manifest or from explicit user file
  picking.
- Host expands `~` and platform-specific variables.
- Host rejects path traversal and paths outside declared permissions.
- Missing files are normal scan outcomes, not fatal global errors.

### Secret Access Rules

- Password import is disabled unless the user enables "include passwords".
- Wasm importers never talk to macOS Keychain, Windows Credential Manager, or Linux
  Secret Service directly.
- `read-secret` is implemented by the host per platform.
- Every record must keep an explicit `PasswordImportStatus`.

```rust
pub enum SecretResult {
    Included { value: String },
    Missing,
    PermissionDenied,
    Unsupported,
}
```

Unsupported proprietary password decryption must remain `Unsupported`, not a best-effort
guess.

## Import Window

The entry remains on the home page next to "new connection", but it opens a dedicated
window instead of a dialog.

Window responsibilities:

- Load importer descriptors from the registry.
- Let users select source applications.
- Run scan for selected importers.
- Show scan state and source-level errors.
- Show preview records in a scrollable list.
- Support row selection, direct save, batch save, and edit.

Recommended layout:

```text
┌──────────────────────────────────────────────────────────────┐
│ 导入连接                                      [刷新扩展] [扫描] │
├──────────────────┬───────────────────────────────────────────┤
│ 应用来源          │ 预览结果                                  │
│                  │                                           │
│ □ DBeaver        │ □ Navicat MySQL        数据库   [编辑][保存] │
│ □ Navicat        │   root@10.2.4.55:3306                     │
│ □ Xshell         │                                           │
│ □ Termius        │ □ Prod SSH             SSH     [编辑][保存] │
│                  │   root@prod.example.com:22                │
└──────────────────┴───────────────────────────────────────────┘
```

Source list behavior:

- Available importers are selected by default.
- Unsupported importers are visible but disabled.
- Permission-required importers are selectable and show an authorization or manual-file
  action.
- Source errors are shown inline and do not block other sources.

Preview list behavior:

- Records from database and SSH importers appear in the same list.
- Each row shows source application, type, title, endpoint, password status, warnings,
  edit action, and save action.
- Rows can be selected for batch save.
- Saved rows remain visible with saved status.
- Failed saves remain editable with the error shown.

## Editing Behavior

Importers do not provide custom editing UI. The application opens a standard editor
based on `ImportRecordKind`.

### Database Editor

Opened for `ImportRecordKind::Database`.

Fields:

- Name
- Database type
- Host
- Port
- Username
- Password
- Database
- Extra parameters

Actions:

- Save
- Save and continue to next selected record
- Cancel

### SSH Editor

Opened for `ImportRecordKind::Ssh`.

Fields:

- Name
- Host
- Port
- Username
- Auth method
- Password
- Private key path
- Passphrase

Actions:

- Save
- Save and continue to next selected record
- Cancel

## Direct Save Behavior

Direct save bypasses the editor and persists the preview record as-is.

Validation rules:

- Database records require name, database type, host, and valid port.
- SSH records require name, host, valid port, username, and auth method.
- Records missing required fields cannot be directly saved and must be edited.
- Duplicate detection uses normalized endpoint identity, not display name.

Save result states:

- Pending
- Saving
- Saved
- Failed with message
- Skipped as duplicate

## Save Pipeline

The save pipeline owns conversion from preview record to application storage models.

```rust
pub trait ImportSavePipeline {
    fn validate(&self, record: &ImportRecord) -> Result<(), ImportValidationError>;
    fn save(&self, record: ImportRecord) -> Result<ImportSaveResult, ImportSaveError>;
}
```

Database records convert into `DbConnectionConfig`. SSH records convert into `SshParams`
or the existing SSH connection storage model. Importers never receive storage handles.

## Runtime Integration

Recommended module structure:

```text
crates/connection-import-protocol
  src/model.rs
  src/manifest.rs
  src/error.rs

crates/connection-import-host
  src/registry.rs
  src/host_api.rs
  src/file_access.rs
  src/secret_access.rs
  src/save_pipeline.rs

crates/connection-import-wasm
  src/runtime.rs
  src/bindings.rs
  src/adapter.rs

main/src/home/connection_import_window.rs
main/src/home/connection_import_source_list.rs
main/src/home/connection_import_preview_table.rs
main/src/home/connection_import_database_editor.rs
main/src/home/connection_import_ssh_editor.rs
main/src/home/connection_import_save_actions.rs
```

The actual crate split can be adjusted during implementation, but the responsibility
split should stay intact:

- Protocol is data-only.
- Host owns permissions and IO.
- Wasm runtime owns component instantiation and call dispatch.
- UI owns selection, preview, edit, and user-facing status.
- Save pipeline owns validation and persistence.

## Migration Plan

### Stage 1: Registry and Window

- Add protocol types.
- Add importer registry.
- Wrap current Rust importers as legacy registry entries.
- Replace the dialog with a dedicated import window.
- Unify database and SSH preview records.
- Add type-specific edit windows.
- Add direct save and batch save.

Stage 1 should preserve current DBeaver, Navicat, Xshell, FinalShell, Termius, DataGrip,
HeidiSQL, TablePlus, Sequel Ace, and Beekeeper behavior where supported.

### Stage 2: Wasm Host API

- Add WIT contract for connection import.
- Add `contributes.connectionImporters` manifest parsing.
- Add Host API implementation for platform, candidate files, file reads, directory
  reads, secrets, and logging.
- Add a Wasm importer test harness.
- Register Wasm importers in the same importer registry.

### Stage 3: First-Party Wasm Importers

Migrate importers incrementally:

1. DBeaver first, because JSON parsing is straightforward and useful for validating the
   database record path.
2. Xshell second, because it validates SSH record preview and SSH editor behavior.
3. Navicat third, because plist/XML candidates validate multi-file and multi-format
   parsing.

### Stage 4: Extension Distribution

- Show connection importers in extension catalog metadata.
- Support install, upgrade, disable, and permission review for importer extensions.
- Allow refreshing the import window after installing or disabling extensions.

## Error Handling

Errors are scoped by importer and operation.

- A scan error for Navicat must not prevent Xshell preview.
- A malformed file produces an importer-level error or warning.
- A malformed record can be skipped with a row warning.
- A failed direct save stays on the row and can be retried after editing.
- Wasm timeout produces an importer error and does not crash the window.
- Host API permission denial produces a user-facing permission-required state.

## Testing Strategy

Protocol tests:

- Descriptors serialize and deserialize without losing IDs or output kinds.
- Database and SSH records reject mismatched `kind` payloads.
- Password status survives round trips.

Registry tests:

- Registry lists legacy and Wasm importers through the same interface.
- Unsupported importers are returned as disabled descriptors.
- One importer failure does not abort all selected scans.

Host API tests:

- `read-file` rejects undeclared candidate IDs.
- `read-file` rejects path traversal.
- Missing files return a normal missing-file result.
- `read-secret` returns `Unsupported` on platforms without a configured secret backend.

Wasm runtime tests:

- A sample Wasm importer returns a descriptor.
- A sample Wasm importer reads an authorized candidate file.
- A sample Wasm importer cannot read an undeclared file.
- Timeout is reported as an importer error.

UI state tests:

- The import window starts with available sources selected.
- Unsupported sources cannot be selected.
- Preview rows are generated for both database and SSH records.
- Editing a database row opens the database editor.
- Editing an SSH row opens the SSH editor.
- Direct save rejects incomplete records.
- Batch save preserves per-row statuses.

## Acceptance Criteria

- The home page import button opens a dedicated import window.
- The window shows importer sources from a registry, not a hard-coded UI whitelist.
- Users select source applications before scanning.
- Scan and preview are source-scoped and scrollable.
- Preview shows database and SSH records in one list.
- Each row supports selection, edit, and direct save.
- Database rows open a database editor window.
- SSH rows open an SSH editor window.
- Saving uses application save logic, not importer logic.
- A new importer can be added through the registry without editing preview UI code.
- Wasm importers can access files and secrets only through Host API.
- File and secret permission failures are visible and recoverable.
- The first stage can run existing Rust importers through the new registry.

## Key Decisions

1. Use existing composite extension manifests with `contributes.connectionImporters`
   instead of adding a new extension installation system.
2. Use Wasm Components for importer implementation.
3. Keep all filesystem and secret access in Host APIs.
4. Keep importers read-only and storage-agnostic.
5. Use application-owned standard editors instead of importer-provided UI.
6. Support legacy Rust importers during migration, but expose them through the same
   registry contract as Wasm importers.
