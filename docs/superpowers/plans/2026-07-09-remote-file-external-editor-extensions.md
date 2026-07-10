# Remote File External Editor Extensions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a marketplace-driven external editor provider system for SFTP remote files, with host-owned download, watch, conflict-check, and upload behavior.

**Architecture:** Composite extensions contribute `remoteFileEditors` descriptors in `extension.json`. `extension-runtime` validates and registers those descriptors in the runtime catalog. `remote_file_editor` owns the secure host workflow for temp files, process launch, file watching, conflict checks, and SFTP upload. `terminal_view` and `sftp_view` render dynamic external editor actions from the registered catalog while preserving built-in edit and image-preview behavior.

**Tech Stack:** Rust 2024, GPUI, `gpui_component`, `extension-runtime` composite manifests, `one-core` settings, `remote_file_editor`, `sftp`, `notify`, `tokio`, `process-util`.

---

## File Structure

- Modify: `crates/extension-runtime/src/extension/manifest/contributes.rs`
  - Adds manifest structs for `contributes.remoteFileEditors`.
- Modify: `crates/extension-runtime/src/types.rs`
  - Adds registered runtime catalog structs.
- Modify: `crates/extension-runtime/src/catalog.rs`
  - Stores registered remote file editor contributions and exposes read APIs.
- Modify: `crates/extension-runtime/src/registration.rs`
  - Registers remote file editor contributions from composite manifests.
- Modify: `crates/extension-runtime/src/extension_runtime_contract_tests.rs`
  - Covers manifest-to-catalog behavior.
- Modify: `crates/core/src/settings.rs`
  - Stores user default editor, open mode, remote conflict preference, and local executable overrides.
- Create: `crates/remote_file_editor/src/external_rules.rs`
  - Matches file masks, filters by platform, orders editor candidates.
- Create: `crates/remote_file_editor/src/external_launcher.rs`
  - Resolves candidates, renders args, launches editor processes without shell execution.
- Create: `crates/remote_file_editor/src/external_session.rs`
  - Manages temp files, local file watch, remote snapshots, conflict decision, and upload state.
- Create: `crates/remote_file_editor/src/external_editor.rs`
  - Public facade used by SFTP UI crates.
- Modify: `crates/remote_file_editor/src/lib.rs`
  - Exports external editor facade and pure helper types.
- Modify: `crates/remote_file_editor/Cargo.toml`
  - Adds `notify` and `process-util` dependencies.
- Modify: `crates/terminal_view/src/sidebar/file_manager_panel.rs`
  - Adds external editor context menu entries and routes selection to `remote_file_editor`.
- Modify: `crates/sftp_view/src/file_list_panel.rs`
  - Adds external editor menu entries.
- Modify: `crates/sftp_view/src/lib.rs`
  - Routes external editor events.
- Modify: `crates/remote_file_editor/locales/remote_file_editor.yml`
  - Adds labels and notifications.
- Modify: `main/src/setting_tab.rs`
  - Adds remote file editor preference settings.
- Modify: `main/locales/main.yml`
  - Adds settings labels.
- Create: marketplace extension package in the extension marketplace repository or local fixture:
  - `extension.json` for `com.onetcli.editor.notepad-plus-plus`.

## Task 1: Manifest Schema For Remote File Editors

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/contributes.rs`
- Modify: `crates/extension-runtime/src/extension/manifest/parser_tests.rs`

- [ ] **Step 1: Add failing parser test**

Add a parser test that loads this manifest JSON and verifies one remote file editor contribution:

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.notepad-plus-plus",
  "name": "Notepad++ External Editor",
  "version": "0.1.0",
  "engines": { "onetcli": ">=0.1.0" },
  "contributes": {
    "remoteFileEditors": [{
      "id": "notepad-plus-plus",
      "displayName": "Notepad++",
      "platforms": ["windows"],
      "fileMasks": ["*"],
      "priority": 100,
      "command": {
        "programCandidates": [
          "${env:ProgramFiles}\\Notepad++\\notepad++.exe",
          "${env:ProgramFiles(x86)}\\Notepad++\\notepad++.exe"
        ],
        "args": ["{file}"]
      }
    }]
  }
}
```

Run: `rtk cargo test -p extension-runtime manifest_loads_remote_file_editor_contributions`

Expected before implementation: compile failure because `remote_file_editors` and related structs do not exist.

- [ ] **Step 2: Add manifest structs**

Add this field to `ContributesManifest`:

```rust
#[serde(default, rename = "remoteFileEditors")]
pub remote_file_editors: Vec<RemoteFileEditorContrib>,
```

Add structs:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteFileEditorContrib {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default, rename = "fileMasks")]
    pub file_masks: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    pub command: RemoteFileEditorCommandContrib,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteFileEditorCommandContrib {
    #[serde(default, rename = "programCandidates")]
    pub program_candidates: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
}
```

Update `ContributesManifest::total_count` to include `remote_file_editors.len()`.

- [ ] **Step 3: Verify parser test**

Run: `rtk cargo test -p extension-runtime manifest_loads_remote_file_editor_contributions`

Expected after implementation: the parser test passes and verifies `id`, `display_name`, `platforms`, `file_masks`, `priority`, `program_candidates`, and `args`.

## Task 2: Runtime Catalog Registration

**Files:**
- Modify: `crates/extension-runtime/src/types.rs`
- Modify: `crates/extension-runtime/src/catalog.rs`
- Modify: `crates/extension-runtime/src/registration.rs`
- Modify: `crates/extension-runtime/src/extension_runtime_contract_tests.rs`

- [ ] **Step 1: Add failing catalog test**

Add a test named `runtime_catalog_registers_remote_file_editors` that creates a base manifest, pushes one `RemoteFileEditorContrib`, builds `ExtensionRuntimeCatalog::from_manifests`, and asserts:

```rust
let editors = catalog.remote_file_editors();
assert_eq!(1, editors.len());
assert_eq!("com.example.tools", editors[0].extension_id);
assert_eq!("notepad-plus-plus", editors[0].id);
assert_eq!("Notepad++", editors[0].display_name);
assert_eq!(vec!["windows"], editors[0].platforms);
assert_eq!(vec!["*"], editors[0].file_masks);
assert_eq!(100, editors[0].priority);
```

Run: `rtk cargo test -p extension-runtime runtime_catalog_registers_remote_file_editors`

Expected before implementation: compile failure because the catalog accessor and registered types do not exist.

- [ ] **Step 2: Add registered catalog types**

In `types.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRemoteFileEditorContribution {
    pub extension_id: String,
    pub id: String,
    pub editor_key: String,
    pub display_name: String,
    pub platforms: Vec<String>,
    pub file_masks: Vec<String>,
    pub priority: i32,
    pub command: RegisteredRemoteFileEditorCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRemoteFileEditorCommand {
    pub program_candidates: Vec<String>,
    pub args: Vec<String>,
}
```

`editor_key` is `format!("{}::{}", extension_id, id)`.

- [ ] **Step 3: Add catalog storage and accessor**

In `ExtensionRuntimeCatalog`, add:

```rust
remote_file_editors: Vec<RegisteredRemoteFileEditorContribution>,
```

Initialize it in `empty()`.

Add:

```rust
pub fn remote_file_editors(&self) -> &[RegisteredRemoteFileEditorContribution] {
    &self.remote_file_editors
}
```

- [ ] **Step 4: Register contributions**

In `registration.rs`, call `self.register_remote_file_editors(&manifest);` inside `register_manifest`.

Add a private method that pushes `RegisteredRemoteFileEditorContribution` values into the catalog. The method copies manifest fields and computes `editor_key`.

- [ ] **Step 5: Verify catalog test**

Run: `rtk cargo test -p extension-runtime runtime_catalog_registers_remote_file_editors`

Expected after implementation: the catalog test passes.

## Task 3: User Settings For External Editor Preferences

**Files:**
- Modify: `crates/core/src/settings.rs`

- [ ] **Step 1: Add failing settings tests**

Add tests:

```rust
#[test]
fn remote_file_editor_settings_default_to_builtin_with_conflict_check() {
    let settings = AppSettings::default();
    assert_eq!(RemoteFileOpenMode::BuiltIn, settings.remote_file_editor.open_mode);
    assert!(settings.remote_file_editor.check_remote_modified_before_upload);
    assert!(settings.remote_file_editor.default_external_editor.is_none());
    assert!(settings.remote_file_editor.overrides.is_empty());
}

#[test]
fn app_settings_deserializes_remote_file_editor_defaults() {
    let settings: AppSettings = serde_json::from_value(serde_json::json!({
        "locale": "en",
        "theme_mode": "dark"
    }))
    .expect("legacy settings should deserialize");
    assert_eq!(RemoteFileOpenMode::BuiltIn, settings.remote_file_editor.open_mode);
    assert!(settings.remote_file_editor.check_remote_modified_before_upload);
}
```

Run: `rtk cargo test -p one-core remote_file_editor_settings`

Expected before implementation: compile failure because settings types do not exist.

- [ ] **Step 2: Add settings types**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileOpenMode {
    BuiltIn,
    External,
}

impl Default for RemoteFileOpenMode {
    fn default() -> Self {
        Self::BuiltIn
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RemoteFileEditorOverride {
    pub editor_key: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileEditorUserSettings {
    #[serde(default)]
    pub open_mode: RemoteFileOpenMode,
    #[serde(default)]
    pub default_external_editor: Option<String>,
    #[serde(default = "default_true")]
    pub check_remote_modified_before_upload: bool,
    #[serde(default)]
    pub overrides: Vec<RemoteFileEditorOverride>,
}

impl Default for RemoteFileEditorUserSettings {
    fn default() -> Self {
        Self {
            open_mode: RemoteFileOpenMode::BuiltIn,
            default_external_editor: None,
            check_remote_modified_before_upload: true,
            overrides: Vec::new(),
        }
    }
}
```

Add `remote_file_editor: RemoteFileEditorUserSettings` to `AppSettings` with `#[serde(default)]`, and update `Default for AppSettings`.

- [ ] **Step 3: Verify settings tests**

Run: `rtk cargo test -p one-core remote_file_editor_settings`

Expected after implementation: settings tests pass.

## Task 4: Editor Matching And Ordering Rules

**Files:**
- Create: `crates/remote_file_editor/src/external_rules.rs`
- Modify: `crates/remote_file_editor/src/lib.rs`

- [ ] **Step 1: Add failing rule tests**

Create tests in `external_rules.rs`:

```rust
#[test]
fn wildcard_mask_matches_file_extension() {
    assert!(matches_file_mask("app.log", "*.log"));
    assert!(!matches_file_mask("app.txt", "*.log"));
}

#[test]
fn star_mask_matches_any_file() {
    assert!(matches_file_mask("Dockerfile", "*"));
}

#[test]
fn editor_key_orders_default_before_priority() {
    let editors = vec![
        editor("com.a::low", "Low", 10),
        editor("com.b::high", "High", 100),
    ];
    let ordered = order_editors(editors, Some("com.a::low"));
    assert_eq!("com.a::low", ordered[0].editor_key);
}
```

Run: `rtk cargo test -p remote_file_editor external_rules`

Expected before implementation: compile failure because the module and helpers do not exist.

- [ ] **Step 2: Implement matching helpers**

Implement:

```rust
pub fn matches_file_mask(file_name: &str, mask: &str) -> bool
pub fn editor_matches_file(file_name: &str, masks: &[String]) -> bool
pub fn editor_supports_platform(platforms: &[String]) -> bool
pub fn order_editors(
    editors: Vec<ExternalEditorCandidate>,
    default_editor: Option<&str>,
) -> Vec<ExternalEditorCandidate>
```

The mask matcher supports `*` and `?`. Empty mask lists match all files. Platform comparison is case-insensitive. `editor_supports_platform` uses `cfg!(target_os = "...")`.

- [ ] **Step 3: Verify rule tests**

Run: `rtk cargo test -p remote_file_editor external_rules`

Expected after implementation: rule tests pass.

## Task 5: Command Resolution And Safe Launcher

**Files:**
- Create: `crates/remote_file_editor/src/external_launcher.rs`
- Modify: `crates/remote_file_editor/Cargo.toml`
- Modify: `crates/remote_file_editor/src/lib.rs`

- [ ] **Step 1: Add dependencies**

Add:

```toml
process-util = { workspace = true }
```

Do not add a shell parsing dependency. Arguments stay as structured `Vec<String>`.

- [ ] **Step 2: Add failing launcher tests**

Add tests:

```rust
#[test]
fn renders_supported_argument_templates() {
    let args = render_args(
        &["--reuse-window".to_string(), "{file}".to_string(), "{name}".to_string()],
        &LaunchTemplateContext {
            file: "/tmp/edit/app.conf".into(),
            remote_path: "/etc/app.conf".into(),
            name: "app.conf".into(),
        },
    );
    assert_eq!(vec!["--reuse-window", "/tmp/edit/app.conf", "app.conf"], args);
}

#[test]
fn rejects_empty_program() {
    let error = validate_program("").expect_err("empty program should fail");
    assert!(error.to_string().contains("empty"));
}
```

Run: `rtk cargo test -p remote_file_editor external_launcher`

Expected before implementation: compile failure because launcher helpers do not exist.

- [ ] **Step 3: Implement launcher helpers**

Implement:

```rust
pub struct LaunchTemplateContext {
    pub file: String,
    pub remote_path: String,
    pub name: String,
}

pub fn render_args(args: &[String], context: &LaunchTemplateContext) -> Vec<String>
pub fn validate_program(program: &str) -> anyhow::Result<()>
pub fn launch_external_editor(program: &str, args: &[String]) -> anyhow::Result<()>
```

`launch_external_editor` uses:

```rust
let mut command = std::process::Command::new(program);
command.args(args);
process_util::configure_background_child(&mut command);
command.spawn()?;
```

It must not invoke `cmd`, `powershell`, `sh`, or `bash` implicitly.

- [ ] **Step 4: Verify launcher tests**

Run: `rtk cargo test -p remote_file_editor external_launcher`

Expected after implementation: launcher tests pass.

## Task 6: External Edit Session State

**Files:**
- Create: `crates/remote_file_editor/src/external_session.rs`
- Modify: `crates/remote_file_editor/Cargo.toml`
- Modify: `crates/remote_file_editor/src/lib.rs`

- [ ] **Step 1: Add dependencies**

Add:

```toml
notify = { workspace = true }
```

If `notify` is already available as a transitive dependency, still add it explicitly because `remote_file_editor` uses it directly.

- [ ] **Step 2: Add failing pure session tests**

Add tests:

```rust
#[test]
fn unchanged_remote_snapshot_allows_upload() {
    let opened = RemoteFileSnapshot { size: 12, modified_secs: 100 };
    let current = RemoteFileSnapshot { size: 12, modified_secs: 100 };
    assert_eq!(UploadDecision::Upload, decide_upload(opened, Some(current)));
}

#[test]
fn changed_remote_snapshot_requires_conflict_prompt() {
    let opened = RemoteFileSnapshot { size: 12, modified_secs: 100 };
    let current = RemoteFileSnapshot { size: 13, modified_secs: 101 };
    assert_eq!(UploadDecision::Conflict, decide_upload(opened, Some(current)));
}

#[test]
fn missing_remote_snapshot_requires_missing_remote_prompt() {
    let opened = RemoteFileSnapshot { size: 12, modified_secs: 100 };
    assert_eq!(UploadDecision::RemoteMissing, decide_upload(opened, None));
}
```

Run: `rtk cargo test -p remote_file_editor external_session`

Expected before implementation: compile failure because session types do not exist.

- [ ] **Step 3: Implement pure session state**

Implement:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteFileSnapshot {
    pub size: u64,
    pub modified_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadDecision {
    Upload,
    Conflict,
    RemoteMissing,
}

pub fn decide_upload(
    opened: RemoteFileSnapshot,
    current: Option<RemoteFileSnapshot>,
) -> UploadDecision
```

Add temp path helpers:

```rust
pub fn sanitized_file_name(name: &str) -> String
pub fn session_temp_file(cache_root: &Path, session_id: &str, remote_path: &str) -> PathBuf
```

- [ ] **Step 4: Add async session skeleton**

Add an `ExternalEditSession` struct that stores:

```rust
pub struct ExternalEditSession {
    pub remote_path: String,
    pub local_path: PathBuf,
    pub opened_snapshot: RemoteFileSnapshot,
    pub last_uploaded_snapshot: RemoteFileSnapshot,
}
```

The first pass can keep watcher orchestration in `external_editor.rs` while pure state stays here.

- [ ] **Step 5: Verify session tests**

Run: `rtk cargo test -p remote_file_editor external_session`

Expected after implementation: session tests pass.

## Task 7: External Editor Public Facade

**Files:**
- Create: `crates/remote_file_editor/src/external_editor.rs`
- Modify: `crates/remote_file_editor/src/lib.rs`

- [ ] **Step 1: Add public API**

Expose:

```rust
pub struct ExternalEditorOpenRequest {
    pub remote_path: String,
    pub editor_key: String,
}

pub fn open_remote_file_external_editor<T: 'static>(
    request: ExternalEditorOpenRequest,
    client: Arc<Mutex<RusshSftpClient>>,
    window: &mut Window,
    cx: &mut Context<T>,
)
```

The facade loads the selected registered editor contribution, resolves the command, reads the remote file, writes the temp file, launches the editor, and starts a file watcher.

- [ ] **Step 2: Add missing client notification paths**

If the SFTP client is absent, the caller keeps showing the existing "SFTP client is not connected" notification. If the selected editor cannot resolve a program, the facade pushes a notification explaining that the executable path must be configured.

- [ ] **Step 3: Implement upload debounce**

Use `notify` events to trigger a debounce timer before reading the temp file and uploading. A debounce delay between 500ms and 1000ms is acceptable. The debounce must collapse repeated write events from the same save operation.

- [ ] **Step 4: Implement conflict prompt**

Before upload, call `stat(remote_path)` and use `decide_upload`. On conflict, prompt the user with:

- `Overwrite Remote`
- `Reload Remote`
- `Cancel`

Only `Overwrite Remote` calls `write_file`. `Reload Remote` downloads the remote file into the same temp path and updates the snapshot.

- [ ] **Step 5: Verify crate builds**

Run: `rtk cargo check -p remote_file_editor`

Expected after implementation: crate compiles.

## Task 8: Terminal File Manager Context Menu

**Files:**
- Modify: `crates/terminal_view/src/sidebar/file_manager_panel.rs`
- Modify: `crates/remote_file_editor/locales/remote_file_editor.yml`

- [ ] **Step 1: Add labels**

Add locale keys:

```yaml
RemoteFileEditor:
  action:
    edit_external:
      en: Edit With External Editor
      zh-CN: 使用外部编辑器编辑
      zh-HK: 使用外部編輯器編輯
```

Add editor-specific missing executable notification under `RemoteFileEditor.notification`.

- [ ] **Step 2: Add menu construction helper**

Add a helper in `file_manager_panel.rs` that gets matching registered external editors for a file path and appends submenu items under `Edit With External Editor`.

If the current popup menu API does not support nested submenus, append flat entries:

```text
Edit With Notepad++
Edit With Visual Studio Code
```

- [ ] **Step 3: Route external editor selection**

When the user selects an editor, call:

```rust
open_remote_file_external_editor(
    ExternalEditorOpenRequest {
        remote_path: path_for_edit.clone(),
        editor_key: editor.editor_key.clone(),
    },
    client,
    window,
    cx,
);
```

Keep the existing `Common.edit` item unchanged.

- [ ] **Step 4: Verify terminal_view builds**

Run: `rtk cargo check -p terminal_view`

Expected after implementation: crate compiles and the existing built-in edit path remains present.

## Task 9: Standalone SFTP View Context Menu

**Files:**
- Modify: `crates/sftp_view/src/file_list_panel.rs`
- Modify: `crates/sftp_view/src/lib.rs`

- [ ] **Step 1: Extend file list panel events**

Add an event variant:

```rust
EditExternal {
    full_path: String,
    editor_key: String,
}
```

- [ ] **Step 2: Add menu entries**

Use the same editor matching helper behavior as terminal file manager. Keep `Common.edit` unchanged for built-in editor.

- [ ] **Step 3: Route event in `sftp_view/src/lib.rs`**

In the remote context menu event handler, call `open_remote_file_external_editor` with the selected `editor_key`.

- [ ] **Step 4: Verify sftp_view builds**

Run: `rtk cargo check -p sftp_view`

Expected after implementation: crate compiles.

## Task 10: Settings UI For User Preferences

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`
- Modify: `crates/core/src/settings.rs`

- [ ] **Step 1: Add settings labels**

Add labels under `Settings`:

```yaml
RemoteFileEditor:
  title:
    en: Remote File Editor
    zh-CN: 远程文件编辑器
    zh-HK: 遠端檔案編輯器
  open_mode:
    en: Default open mode
    zh-CN: 默认打开方式
    zh-HK: 預設打開方式
  built_in:
    en: Built-in editor
    zh-CN: 内置编辑器
    zh-HK: 內置編輯器
  external:
    en: External editor
    zh-CN: 外部编辑器
    zh-HK: 外部編輯器
  conflict_check:
    en: Check whether the remote file changed before upload
    zh-CN: 上传前检查远程文件是否已被修改
    zh-HK: 上傳前檢查遠端檔案是否已被修改
```

- [ ] **Step 2: Add settings group**

Add a `Remote File Editor` settings group that includes:

- Dropdown for `open_mode`.
- Dropdown for `default_external_editor` populated from installed contributions.
- Checkbox for `check_remote_modified_before_upload`.

Use `SettingField::render` for the default editor dropdown if dynamic catalog data is easier to access through a custom render closure.

- [ ] **Step 3: Add override configuration entry point**

For each installed external editor with no resolved executable path, render a small configure button that lets the user pick the program path with `prompt_for_paths`. Store the result in `remote_file_editor.overrides`.

- [ ] **Step 4: Verify main builds**

Run: `rtk cargo check -p main`

Expected after implementation: app crate compiles.

## Task 11: Marketplace Notepad++ Extension

**Files:**
- Create in the extension marketplace repository or local extension fixture:
  - `extension.json`
  - `README.md`
  - optional `icon.png`

- [ ] **Step 1: Create Notepad++ composite extension manifest**

Use:

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.notepad-plus-plus",
  "name": "Notepad++ External Editor",
  "version": "0.1.0",
  "engines": { "onetcli": ">=0.1.0" },
  "categories": ["Editor", "SFTP"],
  "description": "Use Notepad++ to edit SFTP remote files from OnetCli.",
  "contributes": {
    "remoteFileEditors": [{
      "id": "notepad-plus-plus",
      "displayName": "Notepad++",
      "platforms": ["windows"],
      "fileMasks": ["*"],
      "priority": 100,
      "command": {
        "programCandidates": [
          "${env:ProgramFiles}\\Notepad++\\notepad++.exe",
          "${env:ProgramFiles(x86)}\\Notepad++\\notepad++.exe"
        ],
        "args": ["{file}"]
      }
    }]
  }
}
```

- [ ] **Step 2: Package as composite extension**

The package root must contain `extension.json` so `extension_package_layout.rs` detects `ExtensionKind::Composite`.

- [ ] **Step 3: Add marketplace entry**

Add a marketplace entry with `kind: "composite"` and a platform-specific artifact for Windows.

## Task 12: End-To-End Verification

**Files:**
- Read-only verification across affected crates.

- [ ] **Step 1: Run targeted tests**

Run:

```bash
rtk cargo test -p extension-runtime remote_file_editor
rtk cargo test -p one-core remote_file_editor_settings
rtk cargo test -p remote_file_editor external_rules
rtk cargo test -p remote_file_editor external_launcher
rtk cargo test -p remote_file_editor external_session
```

Expected: all targeted tests pass.

- [ ] **Step 2: Run build checks**

Run:

```bash
rtk cargo check -p remote_file_editor
rtk cargo check -p terminal_view
rtk cargo check -p sftp_view
rtk cargo check -p main
```

Expected: all affected crates compile.

- [ ] **Step 3: Manual smoke test**

Install the Notepad++ composite extension on Windows, open an SSH terminal file manager, right-click a remote text file, choose `Edit With Notepad++`, save the file in Notepad++, and verify the remote file is uploaded.

Expected: the edited content appears on the remote server, and no built-in editor behavior regresses.

- [ ] **Step 4: Conflict smoke test**

Open a remote file externally, modify the same remote file outside OnetCli, then save the local temp file.

Expected: OnetCli prompts before overwrite and does not silently clobber the remote change.

## Self-Review Checklist

- The design uses extension marketplace composite packages for editor definitions.
- The host remains responsible for SFTP credentials, temp files, conflict checks, and upload.
- Notepad++ is not hard-coded in application settings.
- `remoteFileEditors` can support other editors without changing host code.
- Built-in remote editor and image preview behavior remain intact.
- The implementation is split into independently testable pure logic, catalog registration, UI wiring, and host workflow tasks.
