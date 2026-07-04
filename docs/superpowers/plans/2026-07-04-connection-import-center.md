# Connection Import Center Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two-step connection import dialog with the designed dedicated import center window: source selection, source-scoped scan, unified preview, row selection, row editing, direct save, batch save, and per-row save status.

**Architecture:** Keep `connection-import-protocol` as the cross-layer data contract. Add runtime scan support to `extension-runtime`, add a main-owned import save/status model, and render a popup import center window that consumes manifest-declared Wasm importers without source-specific UI branches. Reuse existing database and SSH form windows by adding an `initial_connection` prefill path that saves as a new connection instead of editing an existing one.

**Tech Stack:** Rust, GPUI, gpui-component, Wasmtime Component runtime, `connection-import-protocol`, `extension-runtime`, `db_view::ConnectionFormWindow`, `terminal_view::SshFormWindow`.

---

## File Structure

- Create `main/src/home/connection_import_window.rs`
  Dedicated window view. Owns source rows, scan/preview state, row status, row selection, edit/direct save actions, and rendering.
- Create `main/src/home/connection_import_model.rs`
  Pure state helpers for source selection, scan merge, preview row status, save result transitions, and endpoint duplicate identity.
- Modify `main/src/home/connection_import_actions.rs`
  Expose `scan_import_sources`, `preview_import_records`, `save_import_record`, and `save_import_records`. Keep storage and notifier side effects outside the window view.
- Modify `main/src/home/connection_import_draft.rs`
  Keep import record to `StoredConnection` conversion, add accessors needed by the window, and avoid growing UI state here.
- Modify `main/src/home/mod.rs`
  Export the new window/model modules and remove the old dialog/preview module wiring after migration.
- Modify `main/src/home_tab.rs`
  Open the import center popup from the toolbar import button.
- Modify `crates/extension-runtime/src/connection_import_provider.rs`
  Add manifest importer scan support by calling the existing `ConnectionImportComponentRuntime::scan`.
- Modify `crates/db_view/src/connection_form_window.rs`
  Add `initial_connection: Option<StoredConnection>` to prefill database form windows without treating them as edits.
- Modify `crates/terminal_view/src/ssh_form_window.rs`
  Add `initial_connection: Option<StoredConnection>` to prefill SSH form windows without treating them as edits.
- Add/update tests:
  `main/src/home/connection_import_model_tests.rs`,
  `main/src/home/connection_import_draft_tests.rs`,
  `crates/extension-runtime/src/extension_runtime_wasm_contract_tests.rs`,
  `crates/db_view/src/connection_form_window.rs`,
  `crates/terminal_view/src/ssh_form_window.rs`.

## Task 1: Provider Scan Support

**Files:**
- Modify: `crates/extension-runtime/src/connection_import_provider.rs`
- Modify: `crates/extension-runtime/src/extension_runtime_wasm_contract_tests.rs`

- [x] **Step 1: Write the failing provider scan test**

Add a test beside `connection_import_provider_previews_dbeaver_and_termius_wasm_fixtures`:

```rust
#[test]
fn connection_import_provider_scans_each_selected_manifest_importer() {
    let tmp = tempfile::tempdir().unwrap();
    write_wasm_importer_extension(
        tmp.path(),
        WasmImporterFixture {
            extension_dir: "dbeaver",
            extension_id: "com.onetcli.importer.dbeaver",
            importer_id: "dbeaver",
            runtime_id: "dbeaver-importer",
            display_name: "DBeaver",
            output_kind: "database",
            component_name: "dbeaver.component.wasm",
            core_wat: dbeaver_importer_core_wat(),
            candidate_id: "dbeaver-data-sources",
        },
    );

    let reports = futures::executor::block_on(scan_manifest_connection_importers(
        tmp.path(),
        &["com.onetcli.importer.dbeaver/dbeaver".to_string()],
    ))
    .unwrap();

    assert_eq!(1, reports.len());
    assert_eq!("dbeaver", reports[0].importer_id);
    assert!(matches!(
        reports[0].availability,
        connection_import_protocol::ImporterAvailability::Available { .. }
    ));
}
```

- [x] **Step 2: Run the provider scan test and verify RED**

Run:

```bash
rtk cargo test -p extension-runtime connection_import_provider_scans_each_selected_manifest_importer
```

Expected: FAIL with unresolved function `scan_manifest_connection_importers`.

- [x] **Step 3: Implement `scan_manifest_connection_importers`**

Add this public async function next to `preview_manifest_connection_importers`:

```rust
#[cfg(feature = "wasm-components")]
pub async fn scan_manifest_connection_importers(
    composite_root: &Path,
    importer_ids: &[String],
) -> Result<Vec<connection_import_protocol::ImportScanReport>> {
    let importers = list_manifest_connection_importers(composite_root)?;
    let mut reports = Vec::new();
    for importer in importers
        .into_iter()
        .filter(|importer| importer_ids.contains(&importer.descriptor.id))
    {
        let module = importer.extension_dir.join(&importer.module);
        let runtime =
            ConnectionImportComponentRuntime::from_file(importer.descriptor.id.clone(), &module)
                .with_context(|| format!("加载连接导入 Wasm 失败: {}", module.display()))?;
        let host = ManifestConnectionImportHost::new(
            importer.candidates.clone(),
            importer.permissions.clone(),
        );
        let state = ConnectionImportHostState::new(
            importer.extension_id,
            importer.descriptor.id,
            host,
            PermissionSet::new(importer.permissions),
        );
        reports.push(runtime.scan(state).await?);
    }
    Ok(reports)
}
```

Add the `#[cfg(not(feature = "wasm-components"))]` version returning `wasm component runtime is disabled`.

- [x] **Step 4: Run provider tests and verify GREEN**

Run:

```bash
rtk cargo test -p extension-runtime connection_import_provider
```

Expected: PASS.

## Task 2: Import Center State Model

**Files:**
- Create: `main/src/home/connection_import_model.rs`
- Create: `main/src/home/connection_import_model_tests.rs`
- Modify: `main/src/home/mod.rs`

- [x] **Step 1: Write failing model tests**

Add tests for four pure behaviors:

```rust
#[test]
fn available_sources_start_selected_and_unsupported_sources_do_not() {
    let sources = ImportCenterState::new(vec![
        descriptor("dbeaver", vec![Platform::Macos]),
        descriptor("windows-only", vec![Platform::Windows]),
    ], Platform::Macos);

    assert_eq!(vec!["dbeaver".to_string()], sources.selected_source_ids());
    assert!(!sources.source("windows-only").unwrap().selectable);
}

#[test]
fn scan_reports_are_scoped_to_the_matching_source() {
    let mut state = ImportCenterState::new(vec![descriptor("dbeaver", vec![Platform::Macos])], Platform::Macos);
    state.apply_scan_reports(vec![scan_report("dbeaver", ImporterAvailability::NoData)]);

    assert!(matches!(state.source("dbeaver").unwrap().availability, ImporterAvailability::NoData));
}

#[test]
fn preview_records_become_selected_pending_rows() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![database_record("db")]);

    let row = state.rows().first().unwrap();
    assert!(row.selected);
    assert_eq!(ImportRowSaveStatus::Pending, row.save_status);
}

#[test]
fn saved_and_failed_results_are_kept_per_row() {
    let mut state = ImportCenterState::empty_for_tests();
    state.apply_preview_records(vec![database_record("db"), database_record("other")]);

    state.mark_saved("db", 42);
    state.mark_failed("other", "端口必须是 1-65535".to_string());

    assert_eq!(ImportRowSaveStatus::Saved { connection_id: Some(42) }, state.row("db").unwrap().save_status);
    assert_eq!(ImportRowSaveStatus::Failed { message: "端口必须是 1-65535".to_string() }, state.row("other").unwrap().save_status);
}
```

- [x] **Step 2: Run model tests and verify RED**

Run:

```bash
rtk cargo test -p main connection_import_model
```

Expected: FAIL because `connection_import_model` does not exist.

- [x] **Step 3: Implement pure model types**

Create:

```rust
pub(crate) struct ImportCenterState {
    sources: Vec<ImportSourceState>,
    rows: Vec<ImportPreviewRow>,
    current_platform: Platform,
}

pub(crate) struct ImportSourceState {
    pub descriptor: ImporterDescriptor,
    pub selected: bool,
    pub selectable: bool,
    pub availability: ImporterAvailability,
    pub scan_error: Option<String>,
}

pub(crate) struct ImportPreviewRow {
    pub draft: EditableImportDraft,
    pub selected: bool,
    pub save_status: ImportRowSaveStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportRowSaveStatus {
    Pending,
    Saving,
    Saved { connection_id: Option<i64> },
    Failed { message: String },
    SkippedDuplicate { existing_name: String },
}
```

Implement `new`, `empty_for_tests`, `selected_source_ids`, `source`, `rows`, `row`, `apply_scan_reports`, `apply_preview_records`, `toggle_source`, `toggle_row`, `mark_saving`, `mark_saved`, `mark_failed`, and `mark_duplicate`.

- [x] **Step 4: Run model tests and verify GREEN**

Run:

```bash
rtk cargo test -p main connection_import_model
```

Expected: PASS.

## Task 3: Duplicate Identity

**Files:**
- Modify: `main/src/home/connection_import_actions.rs`
- Modify: `main/src/home/connection_import_draft.rs`
- Modify: `main/src/home/connection_import_draft_tests.rs`

- [x] **Step 1: Write failing duplicate identity tests**

Add tests proving:

```rust
#[test]
fn database_duplicate_identity_uses_type_host_port_username_and_database() {
    let draft = EditableImportDraft::new(database_import("prod"));

    assert_eq!(
        "db:mysql:mysql.example.test:3306:root:app",
        draft.duplicate_identity().unwrap()
    );
}

#[test]
fn ssh_duplicate_identity_uses_host_port_and_username() {
    let draft = EditableImportDraft::new(ssh_import("jump"));

    assert_eq!("ssh:ssh.example.test:2222:deploy", draft.duplicate_identity().unwrap());
}
```

Add action-level tests for duplicate detection with a fake list helper:

```rust
#[test]
fn duplicate_detection_matches_existing_connection_identity() {
    let draft = EditableImportDraft::new(database_import("prod"));
    let existing = draft.to_stored_connection().unwrap();

    assert_eq!(
        Some("prod".to_string()),
        duplicate_connection_name(&draft, &[existing]).unwrap()
    );
}
```

- [x] **Step 2: Run draft tests and verify RED**

Run:

```bash
rtk cargo test -p main connection_import_draft
```

Expected: FAIL because duplicate identity helpers do not exist.

- [x] **Step 3: Implement duplicate identity helpers**

Add:

```rust
pub(crate) fn duplicate_connection_name(
    draft: &EditableImportDraft,
    existing: &[StoredConnection],
) -> Result<Option<String>, String> {
    let wanted = draft.duplicate_identity()?;
    for connection in existing {
        let candidate = EditableImportDraft::from_stored_connection_for_identity(connection)?;
        if candidate.duplicate_identity()? == wanted {
            return Ok(Some(connection.name.clone()));
        }
    }
    Ok(None)
}
```

Make database identity normalize host, default port, username, database, and database type. Make SSH identity normalize host, port, and username. Add `ImportSaveResult` later in Task 5 together with the row save state tests that consume it.

- [x] **Step 4: Run draft/action tests and verify GREEN**

Run:

```bash
rtk cargo test -p main connection_import_draft
```

Expected: PASS.

## Task 4: Standard Form Prefill for Imported Records

**Files:**
- Modify: `crates/db_view/src/connection_form_window.rs`
- Modify: `crates/terminal_view/src/ssh_form_window.rs`
- Modify: `main/src/new_connection/form_page.rs`
- Modify: `main/src/home_tab.rs`

- [x] **Step 1: Write failing prefill tests**

Add tests:

```rust
#[test]
fn database_form_prefill_does_not_enter_edit_mode() {
    let initial_connection = StoredConnection::from_db_connection(test_db_config("imported"));
    let config = ConnectionFormWindowConfig::for_tests(DatabaseType::MySQL)
        .with_initial_connection(initial_connection);

    assert!(!config.is_editing());
    assert!(config.initial_connection.is_some());
}

#[test]
fn ssh_form_prefill_does_not_enter_edit_mode() {
    let initial_connection = StoredConnection::new_ssh("imported".to_string(), test_ssh_params(), None);
    let config = SshFormWindowConfig::for_tests().with_initial_connection(initial_connection);

    assert!(!config.is_editing());
    assert!(config.initial_connection.is_some());
}
```

- [x] **Step 2: Run prefill tests and verify RED**

Run:

```bash
rtk cargo test -p db_view database_form_prefill_does_not_enter_edit_mode
rtk cargo test -p terminal_view ssh_form_prefill_does_not_enter_edit_mode
```

Expected: FAIL because config helpers and `initial_connection` do not exist.

- [x] **Step 3: Add `initial_connection` to config structs**

For `ConnectionFormWindowConfig`:

```rust
pub struct ConnectionFormWindowConfig {
    pub db_type: DatabaseType,
    pub external_driver_id: Option<String>,
    pub editing_connection: Option<StoredConnection>,
    pub initial_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
}
```

Load `editing_connection.as_ref().or(initial_connection.as_ref())` into the form, but compute `is_editing` from `editing_connection.is_some()` only.

For `SshFormWindowConfig`, add the same field and load `editing_connection.as_ref().or(initial_connection.as_ref())` while keeping `is_editing` based only on `editing_connection`.

Update all existing config literals with `initial_connection: None`.

- [x] **Step 4: Run form tests and verify GREEN**

Run:

```bash
rtk cargo test -p db_view database_form_prefill_does_not_enter_edit_mode
rtk cargo test -p terminal_view ssh_form_prefill_does_not_enter_edit_mode
```

Expected: PASS.

## Task 5: Dedicated Import Window UI

**Files:**
- Create: `main/src/home/connection_import_window.rs`
- Modify: `main/src/home/mod.rs`
- Modify: `main/src/home_tab.rs`
- Delete after migration: `main/src/home/connection_import_dialog.rs`, `main/src/home/connection_import_preview_view.rs`, `main/src/home/connection_import_source_picker.rs`

- [x] **Step 1: Write failing window state tests**

Add `main/src/home/connection_import_window_tests.rs` only for non-rendering helpers:

```rust
#[test]
fn import_window_button_state_requires_selected_source_before_scan() {
    let window = ConnectionImportWindowModel::new_for_tests(vec![descriptor("dbeaver")]);
    assert!(window.can_scan());

    let mut window = window;
    window.toggle_source("dbeaver");
    assert!(!window.can_scan());
}

#[test]
fn batch_save_only_targets_selected_pending_or_failed_rows() {
    let mut window = ConnectionImportWindowModel::empty_for_tests();
    window.apply_preview_records(vec![database_record("a"), database_record("b")]);
    window.mark_saved("a", Some(1));

    assert_eq!(vec!["b".to_string()], window.batch_save_row_ids());
}
```

- [x] **Step 2: Run window tests and verify RED**

Run:

```bash
rtk cargo test -p main connection_import_window
```

Expected: FAIL because `connection_import_window` does not exist.

- [x] **Step 3: Implement the popup window**

Use this helper:

```rust
pub(crate) fn show_connection_import_window(
    parent: Entity<HomePage>,
    parent_window: AnyWindowHandle,
    cx: &mut App,
) {
    open_popup_window(
        PopupWindowOptions::new(t!("Home.import").to_string()).size(1040.0, 720.0),
        move |window, cx| {
            cx.new(|cx| ConnectionImportWindow::new(parent, parent_window, window, cx))
        },
        cx,
    );
}
```

Render:

- Title bar: `导入连接` centered, with `刷新扩展`, `扫描`, `保存所选` buttons.
- Left source pane: one row per `ImporterDescriptor`, checkbox, icon, display name, availability/status text.
- Right pane: source-scoped scan summary and scrollable preview rows.
- Preview row: checkbox, source label, database/SSH type, title, endpoint, password status, warnings, save status, `编辑`, `保存` actions.

Actions:

- `refresh_extensions` reloads manifest descriptors.
- `scan_selected` calls `scan_import_sources`, applies reports, then calls `preview_import_records`.
- `save_row` marks row saving, calls `save_import_record`, then marks saved/duplicate/failed.
- `save_selected` runs `save_row` for each selected pending/failed row.
- `edit_row` converts draft to `StoredConnection` and opens `ConnectionFormWindow` or `SshFormWindow` with `initial_connection`.

- [x] **Step 4: Wire the toolbar import button**

Replace:

```rust
show_connection_import_dialog(window, cx);
```

with:

```rust
show_connection_import_window(cx.entity(), window.window_handle(), cx);
```

- [x] **Step 5: Run main import tests and verify GREEN**

Run:

```bash
rtk cargo test -p main connection_import
```

Expected: PASS.

## Task 6: Remove Old Dialog Flow

**Files:**
- Modify: `main/src/home/mod.rs`
- Delete or stop compiling: `main/src/home/connection_import_dialog.rs`
- Delete or stop compiling: `main/src/home/connection_import_preview_view.rs`
- Delete or stop compiling: `main/src/home/connection_import_source_picker.rs`

- [x] **Step 1: Search for old dialog flow**

Run:

```bash
rtk rg -n "show_connection_import_dialog|ConnectionImportPreview|ConnectionImportSourcePicker|open_dialog\\(cx, move \\|dialog" main/src/home main/src/home_tab.rs
```

Expected before cleanup: matches old files only.

- [x] **Step 2: Remove old modules from `home/mod.rs`**

Keep only modules that are still used:

```rust
pub(crate) mod connection_import_actions;
pub(crate) mod connection_import_draft;
pub(crate) mod connection_import_model;
pub(crate) mod connection_import_window;
#[cfg(test)]
mod connection_import_draft_tests;
#[cfg(test)]
mod connection_import_model_tests;
```

- [x] **Step 3: Run search again**

Run:

```bash
rtk rg -n "show_connection_import_dialog|ConnectionImportPreview|ConnectionImportSourcePicker" main/src
```

Expected: no matches.

## Task 7: Verification

**Files:**
- No production files unless verification reveals issues.

- [x] **Step 1: Format changed Rust files**

Run:

```bash
rtk cargo fmt
```

Expected: PASS.

- [x] **Step 2: Run targeted tests**

Run:

```bash
rtk cargo test -p connection-import-protocol
rtk cargo test -p extension-runtime connection_import_provider
rtk cargo test -p extension-wasm connection_import
rtk cargo test -p main connection_import
rtk cargo test -p db_view database_form_prefill_does_not_enter_edit_mode
rtk cargo test -p terminal_view ssh_form_prefill_does_not_enter_edit_mode
```

Expected: PASS.

- [x] **Step 3: Run compile checks for touched crates**

Run:

```bash
rtk cargo check -p main -p extension-runtime -p db_view -p terminal_view
```

Expected: PASS.

- [x] **Step 4: Completion audit against design**

Check these acceptance items from `docs/superpowers/specs/2026-07-01-connection-import-wasm-extension-design.md`:

- Home import button opens a dedicated import window.
- Window shows importer sources from registry, not hard-coded whitelist.
- Users select source applications before scanning.
- Scan and preview are source-scoped and scrollable.
- Preview shows database and SSH records in one list.
- Each row supports selection, edit, and direct save.
- Database rows open a database editor window.
- SSH rows open an SSH editor window.
- Saving uses application save logic, not importer logic.
- Wasm file access remains behind Host API.

Run:

```bash
rtk rg -n "show_connection_import_dialog|ConnectionImportPreview|ConnectionImportSourcePicker|ImportSourceKind|preview_connections|preview_ssh_connections" main crates --glob "*.rs" --glob "Cargo.toml"
```

Expected: no production matches for the old hard-coded import flow.

## Self-Review

- Spec coverage: Tasks cover provider scan, dedicated window, registry sources, source selection, source-scoped scan/preview, unified database/SSH preview rows, row selection, edit, direct save, batch save, duplicate skip, and standard editor prefill.
- Placeholder scan: No `TBD`, `TODO`, or unspecified future implementation steps remain.
- Type consistency: `ImporterDescriptor`, `ImportScanReport`, `ImportRecord`, `EditableImportDraft`, `ImportCenterState`, `ImportRowSaveStatus`, and `ImportSaveResult` are used consistently across tasks.

## Quality Gate Follow-Up

- [x] Split `connection_import_window.rs` into focused state/action, model, and render modules so the new/modified Rust files stay under the project file-size guardrail.
- [x] Split `connection_import_draft.rs` conversion and duplicate identity helpers into `connection_import_draft_conversion.rs`.
- [x] Preserve source-level error isolation by skipping failed scan sources during preview while still previewing available sources.
- [x] Add imported-record editor save callbacks so standard DB/SSH editors can save and continue to the next selected import row.
- [x] Update Wasm host wiring for current `SecretQuery` fields and `read_candidate_child_file`.
- [x] Re-run `rtk cargo test -p main connection_import` after the split to verify no behavior regression from the module move.
