# Connection Import Wasm-Only Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hard-coded connection importer path with manifest-declared Wasm connection-import extensions.

**Architecture:** The application will discover importers from composite extension manifests, instantiate Wasm Components through a dedicated connection-import world, and expose only host-controlled file, directory, secret, platform, and log APIs. The import UI will consume generic importer descriptors and preview records instead of `ImportSourceKind` or source-specific Rust preview functions.

**Tech Stack:** Rust, Wasmtime component model, WIT, serde manifest parsing, GPUI, existing `extension-runtime`, `extension-wasm`, and `extension-component` crates.

---

### Execution Status

2026-07-02: Implementation completed in the `connection-import` worktree. The built-in importer crate was removed, manifest-declared Wasm connection importers are used by `main`, and DBeaver/Termius Wasm fixtures cover database and SSH preview paths.

### Task 1: Manifest Contribution Shape

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/contributes.rs`
- Modify: `crates/extension-runtime/src/extension/manifest/parser_tests.rs`

- [x] **Step 1: Write the failing manifest parser test**

Add a test that parses a composite extension manifest containing `contributes.connectionImporters` with one importer, one wasm runtime, one candidate file, and fs permissions. Assert that the importer id, runtime id, output kinds, platforms, candidate id, and candidate path survive parsing.

Run: `rtk cargo test -p extension-runtime manifest_parses_connection_importers`

Expected: FAIL because `ContributesManifest` has no `connection_importers` field.

- [x] **Step 2: Implement typed manifest structs**

Add `connection_importers: Vec<ConnectionImporterContrib>` to `ContributesManifest` with serde rename `connectionImporters`. Add typed structs for `ConnectionImporterContrib` and `CandidateFileContrib`. Update `total_count()`.

- [x] **Step 3: Run the manifest test**

Run: `rtk cargo test -p extension-runtime manifest_parses_connection_importers`

Expected: PASS.

### Task 2: Connection Import Protocol Crate

**Files:**
- Create: `crates/connection-import-protocol/Cargo.toml`
- Create: `crates/connection-import-protocol/src/lib.rs`
- Create: `crates/connection-import-protocol/src/model.rs`
- Modify: `Cargo.toml`

- [x] **Step 1: Write protocol tests first**

Add tests for three behaviors: database records validate only with database payloads, SSH records validate only with SSH payloads, and password status serializes/deserializes without loss.

Run: `rtk cargo test -p connection-import-protocol`

Expected: FAIL because the crate does not exist.

- [x] **Step 2: Add data-only protocol models**

Create serializable protocol structs and enums for importer descriptors, scan reports, candidate files, import records, database records, SSH records, auth methods, password status, warnings, platform, import options, host errors, and import errors.

- [x] **Step 3: Run protocol tests**

Run: `rtk cargo test -p connection-import-protocol`

Expected: PASS.

### Task 3: Connection Import Host Boundary

**Files:**
- Modify: `crates/extension-component/src/permissions.rs`
- Modify: `crates/extension-component/src/lib.rs`
- Create: `crates/extension-component/src/connection_import.rs`

- [x] **Step 1: Write host permission tests first**

Add tests proving `fs:read:<path>` permissions are recognized, arbitrary undeclared candidate ids are rejected, and secret reads return `Unsupported` without a secret backend.

Run: `rtk cargo test -p extension-component connection_import`

Expected: FAIL because connection-import host types do not exist.

- [x] **Step 2: Add host trait and permission helpers**

Define an `ExtensionConnectionImportHost` trait with platform, candidate file listing, file reads, directory reads, secret reads, and logging. Keep arbitrary path reads out of the trait.

- [x] **Step 3: Run host tests**

Run: `rtk cargo test -p extension-component connection_import`

Expected: PASS.

### Task 4: WIT World and Wasm Runtime

**Files:**
- Create: `crates/extension-api/wit/connection-import.wit`
- Modify: `crates/extension-api/wit/extension.wit`
- Modify: `crates/extension-wasm/src/bindings.rs`
- Create: `crates/extension-wasm/src/connection_import.rs`
- Create: `crates/extension-wasm/src/connection_import_tests.rs`
- Modify: `crates/extension-wasm/src/lib.rs`

- [x] **Step 1: Write Wasm runtime shape tests first**

Add tests proving a component exporting `descriptor`, `scan`, and `preview` can instantiate, and a guest can read only declared candidate ids through the host API.

Run: `rtk cargo test -p extension-wasm connection_import`

Expected: FAIL because the WIT world and runtime adapter do not exist.

- [x] **Step 2: Add WIT and bindings**

Define `onet:connection-import@1.0.0` with a `connection-importer` world, an `importer` export surface, and a `host` import interface. Generate bindings next to the existing extension bindings.

- [x] **Step 3: Add runtime adapter**

Implement `ConnectionImportComponentRuntime`, `ConnectionImportHostState`, and conversion functions between WIT records and `connection-import-protocol` models.

- [x] **Step 4: Run Wasm runtime tests**

Run: `rtk cargo test -p extension-wasm connection_import`

Expected: PASS.

### Task 5: Wasm Importer Provider

**Files:**
- Create: `crates/extension-runtime/src/connection_import_provider.rs`
- Modify: `crates/extension-runtime/src/lib.rs`
- Add tests in: `crates/extension-runtime/src/extension_runtime_wasm_contract_tests.rs`

- [x] **Step 1: Write provider tests first**

Add tests proving installed composite extensions with `connectionImporters` are listed as importer descriptors and importer ids remain scoped to extension ids to avoid collisions.

Run: `rtk cargo test -p extension-runtime connection_import_provider`

Expected: FAIL because the provider does not exist.

- [x] **Step 2: Implement provider discovery**

Load installed composite manifests, collect `connectionImporters`, resolve runtime modules, and expose descriptors without running parser code.

- [x] **Step 3: Run provider tests**

Run: `rtk cargo test -p extension-runtime connection_import_provider`

Expected: PASS.

### Task 6: Remove Main UI Dependency on Built-In Importers

**Files:**
- Modify: `main/src/home/connection_import_dialog.rs`
- Modify: `main/src/home/connection_import_source_picker.rs`
- Modify: `main/src/home/connection_import_actions.rs`
- Modify: `main/src/home/connection_import_draft.rs`
- Modify: `main/src/home/connection_import_source_icon.rs`
- Modify: `main/Cargo.toml`

- [x] **Step 1: Write UI state tests first**

Update existing source picker and draft tests to use protocol descriptors and import records instead of `ImportSourceKind`, `ImportedConnection`, and `ImportedSshConnection`.

Run: `rtk cargo test -p main connection_import`

Expected: FAIL because UI still depends on `connection_importer`.

- [x] **Step 2: Replace source model**

Make the picker use `ImporterDescriptor` and `ImporterAvailability`. Make source selection use importer id strings.

- [x] **Step 3: Replace preview model**

Make drafts wrap `ImportRecord`. Convert records into `StoredConnection` through an app-owned save pipeline.

- [x] **Step 4: Remove dependency**

Remove `connection_importer` from `main/Cargo.toml`. Keep the old crate in the workspace only if tests or migration tooling still compile against it; otherwise remove it from workspace members in a separate cleanup step.

- [x] **Step 5: Run UI tests**

Run: `rtk cargo test -p main connection_import`

Expected: PASS.

### Task 7: Cleanup Built-In Implementation

**Files:**
- Modify: root `Cargo.toml`
- Delete or quarantine: `crates/connection_importer`

- [x] **Step 1: Verify no production dependency remains**

Run: `rtk rg -n "connection_importer|ImportSourceKind|preview_connections|preview_ssh_connections" main crates --glob "*.rs" --glob "Cargo.toml"`

Expected: only old crate files or no matches outside cleanup targets.

- [x] **Step 2: Remove workspace member if unused**

Remove `crates/connection_importer` from workspace members and dependency tables when no production crate depends on it.

- [x] **Step 3: Run workspace checks**

Run: `rtk cargo check -p extension-runtime -p extension-wasm -p main`

Expected: PASS.

### Self-Review

- Spec coverage: The plan implements manifest contributions, protocol models, host boundary, Wasm runtime, provider discovery, UI model replacement, and built-in importer cleanup from the approved design.
- Placeholder scan: No implementation step depends on an unspecified "later" task.
- Type consistency: The plan consistently uses `ImporterDescriptor`, `ImporterAvailability`, `ImportRecord`, `DatabaseImportRecord`, and `SshImportRecord` as the cross-layer contract.
