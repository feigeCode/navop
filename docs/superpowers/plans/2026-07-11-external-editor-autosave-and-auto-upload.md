# External Editor Autosave and Auto Upload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a generic extension contract for per-session external-editor autosave preparation and a default-on global setting that controls whether saved external-editor changes are uploaded automatically.

**Architecture:** Remote editor extensions may declare bounded, session-relative `workspaceFiles`; the Host validates and writes them before launching the editor without knowing editor IDs. External upload remains a Host-wide policy: when enabled the existing exact-file watcher and conflict-aware uploader run, and when disabled no watcher/controller is created. Zed declares `.zed/settings.json`; editors without a reliable session-level autosave interface remain compatible and still use the same upload path whenever they write the file.

**Tech Stack:** Rust, serde, GPUI settings UI, notify, Tokio file I/O, Node.js extension repository tests, JSON manifests.

---

## File Map

### OnetCli repository

- `crates/extension-runtime/src/extension/manifest/contributes.rs`: manifest structs for workspace files.
- `crates/extension-runtime/src/extension/manifest/parser_tests.rs`: JSON parsing/default tests.
- `crates/extension-runtime/src/types.rs`: registered runtime workspace-file type.
- `crates/extension-runtime/src/registration.rs`: validation and manifest-to-runtime transfer.
- `crates/extension-runtime/src/extension_runtime_contract_tests.rs`: catalog contract tests.
- `crates/core/src/settings/remote_file_editor.rs`: default-on auto-upload setting.
- `crates/core/src/settings.rs`: settings defaults/deserialization tests.
- `crates/remote_file_editor/src/external_workspace.rs`: safe path validation and workspace-file preparation.
- `crates/remote_file_editor/src/external_editor.rs`: carry workspace files and auto-upload session snapshot through preparation/launch.
- `crates/remote_file_editor/src/lib.rs`: module wiring/test exports where required.
- `main/src/settings/remote_file_editor_settings.rs`: auto-upload checkbox.
- `main/locales/main.yml`: English, Simplified Chinese, and Traditional Chinese labels/descriptions.
- `AGENTS.md`: durable external-editor autosave capability guidance.

### Extension repository

- `extensions/wasm/zed-editor/extension.json`: Zed macOS/Linux `.zed/settings.json` declaration and version bump.
- `extensions/wasm/zed-editor/extension.build.json`: release metadata if version/tag is stored there.
- `manifest.json`: marketplace version/tag/archive metadata.
- `tests/scripts.test.mjs`: workspace-files and compatibility assertions.
- Notepad-- and Notepad++ manifests remain free of fake autosave declarations.

### Task 1: Manifest and Runtime Workspace-File Contract

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/contributes.rs`
- Modify: `crates/extension-runtime/src/extension/manifest/parser_tests.rs`
- Modify: `crates/extension-runtime/src/types.rs`
- Modify: `crates/extension-runtime/src/registration.rs`
- Modify: `crates/extension-runtime/src/extension_runtime_contract_tests.rs`

- [ ] **Step 1: Write failing manifest parser tests**

Add assertions equivalent to:

```rust
assert_eq!(1, command.workspace_files.len());
assert_eq!(".zed/settings.json", command.workspace_files[0].path);
assert!(command.workspace_files[0].content.contains("after_delay"));
```

Also assert that a command without `workspaceFiles` receives an empty vector.

- [ ] **Step 2: Run parser tests and verify RED**

Run:

```bash
rtk cargo test -p extension-runtime manifest -- --nocapture
```

Expected: compilation/test failure because `workspace_files` does not exist.

- [ ] **Step 3: Add manifest and runtime types**

Add:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEditorWorkspaceFileContrib {
    pub path: String,
    pub content: String,
}
```

and a defaulted `workspace_files` vector on the command contribution. Add the equivalent registered runtime type and vector.

- [ ] **Step 4: Write failing registration/validation tests**

Cover valid transfer plus rejection of:

```text
empty path
/absolute/path
../escape
.zed/../../escape
Windows-style C:\escape
content above the single-file limit
combined content above the total limit
```

- [ ] **Step 5: Run registration tests and verify RED**

Run:

```bash
rtk cargo test -p extension-runtime remote_file_editor -- --nocapture
```

Expected: new validation tests fail.

- [ ] **Step 6: Implement bounded generic validation and transfer**

Use named constants and `std::path::Component`; reject `Prefix`, `RootDir`, and `ParentDir`. Treat backslashes/drive prefixes explicitly so validation is platform-independent. Transfer cloned validated files into `RegisteredRemoteFileEditorCommand`.

- [ ] **Step 7: Run extension-runtime tests and verify GREEN**

Run:

```bash
rtk cargo test -p extension-runtime
```

Expected: all extension-runtime tests pass.

- [ ] **Step 8: Commit Task 1**

```bash
rtk git add crates/extension-runtime
rtk git commit -m "feat(extension): add external editor workspace files"
```

### Task 2: Safe Session Workspace Preparation

**Files:**
- Create: `crates/remote_file_editor/src/external_workspace.rs`
- Modify: `crates/remote_file_editor/src/lib.rs`
- Modify: `crates/remote_file_editor/src/external_editor.rs`

- [ ] **Step 1: Write failing pure path/preparation tests**

Test a helper with this contract:

```rust
pub(crate) fn workspace_file_path(session_dir: &Path, relative: &str) -> Result<PathBuf>;
```

Assert `.zed/settings.json` resolves below the session directory and invalid paths fail. Add an async preparation test that writes nested files and preserves exact content.

- [ ] **Step 2: Run remote-file-editor tests and verify RED**

```bash
rtk cargo test -p remote_file_editor external_workspace -- --nocapture
```

Expected: module/helper missing.

- [ ] **Step 3: Implement workspace preparation**

Implement focused helpers:

```rust
pub(crate) async fn prepare_workspace_files(
    session_dir: &Path,
    files: &[RegisteredRemoteFileEditorWorkspaceFile],
) -> Result<()>;
```

Create parent directories and write files with Tokio. Do not use shell commands or expand content templates.

- [ ] **Step 4: Carry workspace files into `ExternalEditLaunch`**

Clone the registered command workspace files into the launch request. Change preparation so the downloaded main file and declared workspace files are fully written before `launch_external_editor` is called.

- [ ] **Step 5: Add a regression test for watcher filtering**

Extract/cover the exact-file event predicate and assert an event for `.zed/settings.json` is rejected while the main file is accepted.

- [ ] **Step 6: Run Task 2 tests and verify GREEN**

```bash
rtk cargo test -p remote_file_editor
```

Expected: all remote-file-editor tests pass.

- [ ] **Step 7: Commit Task 2**

```bash
rtk git add crates/remote_file_editor
rtk git commit -m "feat(editor): prepare extension workspace files"
```

### Task 3: Default-On Auto-Upload Setting

**Files:**
- Modify: `crates/core/src/settings/remote_file_editor.rs`
- Modify: `crates/core/src/settings.rs`

- [ ] **Step 1: Write failing settings tests**

Assert:

```rust
assert!(RemoteFileEditorUserSettings::default().auto_upload_external_changes);
```

Deserialize an old settings JSON with the field omitted and expect `true`; deserialize explicit `false` and expect `false`.

- [ ] **Step 2: Run settings tests and verify RED**

```bash
rtk cargo test -p one-core remote_file_editor_settings -- --nocapture
```

Expected: missing field/compilation failure.

- [ ] **Step 3: Implement the setting**

Add:

```rust
#[serde(default = "default_auto_upload_external_changes")]
pub auto_upload_external_changes: bool,
```

with a default function returning `true`, and set it explicitly in `Default`.

- [ ] **Step 4: Run settings tests and verify GREEN**

```bash
rtk cargo test -p one-core remote_file_editor_settings
```

- [ ] **Step 5: Commit Task 3**

```bash
rtk git add crates/core/src/settings.rs crates/core/src/settings/remote_file_editor.rs
rtk git commit -m "feat(settings): configure external editor auto upload"
```

### Task 4: Apply Auto-Upload Policy to Every External Editor

**Files:**
- Modify: `crates/remote_file_editor/src/external_editor.rs`
- Modify: `crates/remote_file_editor/src/external_edit_controller.rs` only if test seams are required

- [ ] **Step 1: Write a failing launch-policy contract test**

Extract a pure decision such as:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalUploadMode {
    Disabled,
    Watch { check_conflict: bool },
}
```

Assert all combinations of `auto_upload_external_changes` and `check_remote_modified_before_upload` produce the behavior matrix in the design.

- [ ] **Step 2: Run the focused test and verify RED**

```bash
rtk cargo test -p remote_file_editor external_upload_mode -- --nocapture
```

- [ ] **Step 3: Implement conditional watcher/controller creation**

Read both settings at session creation. Always launch the editor after file/workspace preparation. Only create `notify::RecommendedWatcher`, `ExternalEditController`, and `ExternalEditWatchLoop` in `Watch` mode. Preserve existing conflict-check behavior when enabled.

- [ ] **Step 4: Run remote-file-editor tests and verify GREEN**

```bash
rtk cargo test -p remote_file_editor
```

- [ ] **Step 5: Commit Task 4**

```bash
rtk git add crates/remote_file_editor
rtk git commit -m "feat(editor): honor external auto upload setting"
```

### Task 5: Settings UI and Locales

**Files:**
- Modify: `main/src/settings/remote_file_editor_settings.rs`
- Modify: `main/locales/main.yml`

- [ ] **Step 1: Add the checkbox item**

Insert `auto_upload_item(default_settings)` between default editor and conflict check. The field reads and persists `settings.remote_file_editor.auto_upload_external_changes` using `AppSettings::update_and_save`.

- [ ] **Step 2: Add three-locale strings**

Add keys:

```yaml
auto_upload:
  en: Automatically Upload External Editor Changes
  zh-CN: 自动上传外部编辑器的修改
  zh-HK: 自動上傳外部編輯器的修改
auto_upload_desc:
  en: Automatically upload changes after an external editor saves the local temporary file
  zh-CN: 外部编辑器保存本地临时文件后，自动将修改上传到远程服务器
  zh-HK: 外部編輯器儲存本機暫存檔案後，自動將修改上傳到遠端伺服器
```

- [ ] **Step 3: Add/update structural UI tests if this settings module has no GPUI checkbox test seam**

Verify item order and both getter/setter field references without introducing timing-dependent GPUI tests.

- [ ] **Step 4: Run main/settings checks**

```bash
rtk cargo test -p main remote_file_editor -- --nocapture
rtk cargo check -p main
```

- [ ] **Step 5: Commit Task 5**

```bash
rtk git add main/src/settings/remote_file_editor_settings.rs main/locales/main.yml
rtk git commit -m "feat(settings): expose external auto upload toggle"
```

### Task 6: Adapt Editor Extensions Without Host Special Cases

**Files:**
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/.worktrees/macos-editor-reload-fix/extensions/wasm/zed-editor/extension.json`
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/.worktrees/macos-editor-reload-fix/manifest.json`
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/.worktrees/macos-editor-reload-fix/tests/scripts.test.mjs`

- [ ] **Step 1: Write failing extension tests**

Assert both Zed contributions declare exactly one workspace file with path `.zed/settings.json`, and parsed JSON content equals:

```json
{
  "autosave": {
    "after_delay": {
      "milliseconds": 1000
    }
  }
}
```

Assert Notepad-- and Notepad++ do not declare fake workspace autosave files.

- [ ] **Step 2: Run extension tests and verify RED**

```bash
rtk node --test tests/scripts.test.mjs
```

- [ ] **Step 3: Update Zed manifest and release metadata**

Bump Zed to the next patch version, add identical `workspaceFiles` to macOS and Linux, and update marketplace version/tag/archive metadata. Leave Notepad-- and Notepad++ behavior unchanged.

- [ ] **Step 4: Run extension tests and verifier**

```bash
rtk node --test tests/scripts.test.mjs
rtk bash scripts/verify-composite-package.sh extensions/wasm/zed-editor
rtk bash scripts/verify-composite-package.sh extensions/wasm/notepad-minus-minus-editor
rtk bash scripts/verify-composite-package.sh extensions/wasm/notepad-plus-plus-editor
```

Expected: all tests and verifier commands pass.

- [ ] **Step 5: Commit Task 6 in the extension worktree**

```bash
rtk git add extensions tests manifest.json
rtk git commit -m "feat(editor): declare Zed workspace autosave"
```

### Task 7: Documentation Rule, Review, Verification, Packaging, and Installation

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-07-11-external-editor-autosave-and-auto-upload.md` checkbox state during execution

- [ ] **Step 1: Record the durable project rule**

Document that Host autosave support must remain editor-agnostic, extensions may only declare verified native/session-level autosave configuration, and Host must not simulate keyboard input to save third-party buffers.

- [ ] **Step 2: Run formatting and focused/full verification**

```bash
rtk cargo fmt --all -- --check
rtk cargo test -p extension-runtime
rtk cargo test -p one-core remote_file_editor_settings
rtk cargo test -p remote_file_editor
rtk cargo check -p main
rtk git diff --check
```

Run extension repository tests and package verifiers again from the extension worktree.

- [ ] **Step 3: Perform code review**

Use `superpowers:requesting-code-review`; inspect contract compatibility, traversal resistance, default compatibility, watcher suppression, and absence of editor-key special cases. Apply any valid findings using `superpowers:receiving-code-review`.

- [ ] **Step 4: Build and install local artifacts**

Build the macOS application using the repository’s established local bundle script, install `/Applications/OnetCli.app`, build the updated Zed extension archive, and install it into the local composite-extension directory. Verify installed manifest/version and application binary hash.

- [ ] **Step 5: Manual acceptance**

Verify:

```text
auto upload ON + Zed: stop typing ~1s -> local write -> remote upload
auto upload OFF + Zed: local autosave occurs -> remote remains unchanged
auto upload ON + Notepad-- manual/native save: remote upload occurs
auto upload OFF + Notepad-- manual/native save: remote remains unchanged
closing Zed/Notepad-- does not close OnetCli
```

- [ ] **Step 6: Completion verification**

Use `superpowers:verification-before-completion`; audit every design acceptance criterion against source, tests, installed artifacts, and manual evidence before claiming completion.
