# Remote File External Editor Extensions Design

## Goal

Add extension-marketplace support for editing SFTP remote files with local external editors such as Notepad++, VS Code, Zed, or Sublime Text. The host application owns all remote-file safety and transfer behavior, while installed composite extensions contribute editor descriptors through a stable manifest contribution point.

This design addresses GitHub issue #98: SFTP remote-file editing should support opening an external editor such as Notepad++.

## Non-Goals

- Do not hard-code Notepad++ or any other editor into global application settings.
- Do not let extensions read SFTP credentials, open SFTP sessions, or write remote files directly.
- Do not require a Wasm runtime for static editor descriptors.
- Do not replace the existing built-in remote text editor.
- Do not change image preview behavior.
- Do not make double-click use external editors by default in the first implementation.
- Do not implement a full WinSCP-style rule editor in the first pass.

## Current Context

The repository already has the required host-side pieces:

- `remote_file_editor` opens a built-in GPUI editor window for remote text files.
- `sftp` exposes `stat`, `read_file`, and `write_file`.
- `terminal_view` and `sftp_view` both route remote file edit actions into `remote_file_editor`.
- `extension-runtime` already supports composite extensions with `extension.json`, marketplace installation, and manifest contribution registration.

The missing piece is a first-class extension contribution point for external remote-file editors.

## Architecture

The system is split into two responsibility layers:

```text
Installed composite extensions
  contribute remoteFileEditors descriptors
        |
        v
extension-runtime catalog
  validates and registers editor descriptors
        |
        v
remote_file_editor host workflow
  selects editor, downloads temp file, launches editor,
  watches local saves, checks remote conflicts, uploads changes
        |
        v
SFTP client
  stat / read_file / write_file
```

The host application remains the only owner of remote-file I/O. Extensions only provide data that describes how an editor can be launched on a supported platform.

## Extension Contribution Point

Composite extensions gain a new manifest field:

```json
{
  "contributes": {
    "remoteFileEditors": []
  }
}
```

Each contribution describes one external editor:

```json
{
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
}
```

The contribution fields are:

- `id`: stable local editor id inside the extension.
- `displayName`: user-facing editor name.
- `platforms`: supported platforms. Valid values are `windows`, `macos`, and `linux`. An empty list means all platforms.
- `fileMasks`: semicolon-free glob-like masks such as `*.txt`, `*.log`, or `*`. An empty list means all files.
- `priority`: higher values sort first when multiple editors match a file.
- `command.programCandidates`: candidate program paths or program names. The host resolves and validates these candidates.
- `command.args`: argument template list. If empty, the host appends `{file}`.

## Example Extensions

### Notepad++

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.notepad-plus-plus",
  "name": "Notepad++ External Editor",
  "version": "0.1.0",
  "engines": { "onetcli": ">=0.1.0" },
  "categories": ["Editor", "SFTP"],
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

### VS Code

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.vscode",
  "name": "VS Code External Editor",
  "version": "0.1.0",
  "engines": { "onetcli": ">=0.1.0" },
  "categories": ["Editor", "SFTP"],
  "contributes": {
    "remoteFileEditors": [{
      "id": "vscode",
      "displayName": "Visual Studio Code",
      "platforms": ["windows", "macos", "linux"],
      "fileMasks": ["*.js", "*.ts", "*.json", "*.md", "*.rs", "*.go", "*.py", "*.yaml", "*.yml"],
      "priority": 80,
      "command": {
        "programCandidates": ["code"],
        "args": ["--reuse-window", "{file}"]
      }
    }]
  }
}
```

## Registered Editor Model

The extension runtime registers manifest contributions into a catalog model:

```rust
pub struct RegisteredRemoteFileEditorContribution {
    pub extension_id: String,
    pub id: String,
    pub display_name: String,
    pub platforms: Vec<String>,
    pub file_masks: Vec<String>,
    pub priority: i32,
    pub command: RegisteredRemoteFileEditorCommand,
}

pub struct RegisteredRemoteFileEditorCommand {
    pub program_candidates: Vec<String>,
    pub args: Vec<String>,
}
```

The globally stable editor identity is:

```text
<extension_id>::<editor_id>
```

For example:

```text
com.onetcli.editor.notepad-plus-plus::notepad-plus-plus
```

## User Settings

Application settings store user preferences and local overrides only. They do not define the editor ecosystem.

```rust
pub struct RemoteFileEditorUserSettings {
    pub open_mode: RemoteFileOpenMode,
    pub default_external_editor: Option<String>,
    pub check_remote_modified_before_upload: bool,
    pub overrides: Vec<RemoteFileEditorOverride>,
}

pub enum RemoteFileOpenMode {
    BuiltIn,
    External,
}

pub struct RemoteFileEditorOverride {
    pub editor_key: String,
    pub program: String,
    pub args: Vec<String>,
}
```

`editor_key` uses the globally stable identity. Overrides exist because editor install paths differ across user machines.

## Editor Selection

When rendering an external editor menu for a remote file:

1. Get registered editor contributions from the extension runtime catalog.
2. Filter by current platform.
3. Filter by file mask match.
4. Resolve user override if present.
5. Resolve the first existing program candidate if no override exists.
6. Sort by user default first, then priority descending, then display name.
7. Show available editors in the file context menu.

If a contribution matches but no program candidate resolves, show it as disabled with an action to configure the local executable path.

## Remote Edit Workflow

The host workflow for external editing is:

1. User selects an external editor for a remote file.
2. Host calls `stat(remote_path)` and stores the remote snapshot.
3. Host calls `read_file(remote_path, max_bytes)`.
4. Host writes bytes to a per-session temp file.
5. Host launches the selected program with rendered arguments.
6. Host watches the temp file with `notify`.
7. On local file change, host debounces the event.
8. Host reads the local temp file bytes.
9. Host calls `stat(remote_path)` before upload.
10. If the remote snapshot is unchanged, host calls `write_file(remote_path, bytes)`.
11. If the remote snapshot changed, host prompts for overwrite, reload remote, or cancel.
12. Host updates the session snapshot after a successful upload.

Process exit is not used as the primary upload trigger because editors often reuse an existing process and return immediately.

## Temporary Files

External edit sessions use a per-session directory:

```text
<cache-dir>/remote-edit/<session-id>/<sanitized-file-name>
```

The session id prevents collisions between two remote files with the same base name. The sanitized file name preserves the original extension for editor language detection.

The host cleans up session directories when the user closes a session or the app exits. On startup, the host removes stale `remote-edit` session directories older than a retention window.

## Conflict Handling

The host stores this snapshot when the file is opened:

```rust
pub struct RemoteFileSnapshot {
    pub size: u64,
    pub modified: SystemTime,
}
```

Before upload, it compares the current remote snapshot with the last known snapshot:

- Same `size` and `modified`: upload directly.
- Remote missing: prompt to recreate or cancel.
- Remote changed: prompt to overwrite, reload remote, or cancel.

This mirrors the WinSCP safety behavior shown in the issue screenshot without exposing remote credentials to extensions.

## Command Safety

The launcher must not execute shell strings. It must use:

```rust
std::process::Command::new(program).args(args)
```

Supported argument template variables are:

- `{file}`: local temp file path.
- `{remote_path}`: remote path.
- `{name}`: remote file base name.

Supported program candidate variables are limited to known environment expansion patterns such as:

- `${env:ProgramFiles}`
- `${env:ProgramFiles(x86)}`

The first use of an editor should confirm the resolved executable path with the user. A user-approved path is stored as an override.

## UI Behavior

Initial behavior:

- Double-click keeps the current behavior: images use image preview, non-images use the built-in editor.
- Context menu keeps `Edit` for the built-in editor.
- Context menu adds an `Edit With External Editor` submenu when matching editor contributions exist.
- The submenu lists available installed editor contributions.
- Missing executable paths are shown as disabled entries with a configure action.

Later behavior:

- Settings can let users switch default double-click behavior to external editor.
- Settings can let users choose a default external editor.

## Settings UI

Settings should expose preferences, not editor definitions:

- Default remote file open mode: built-in or external.
- Default external editor: dropdown from installed contributions.
- Check remote modification before upload: enabled by default.
- Local path override per installed editor contribution.

The full WinSCP-style editor rule manager can be added later if users need manual rule ordering beyond extension-provided priorities.

## Marketplace Packaging

External editor providers are composite extensions. A Notepad++ provider package can contain only:

```text
extension.json
icon.png
README.md
```

No new extension kind is required. The marketplace entry uses `kind: "composite"`.

## Error Handling

The host reports these errors through notifications or prompts:

- No matching external editor installed.
- Matching editor has no resolved executable path.
- External editor launch failed.
- Temp file write failed.
- Temp file watch failed.
- Remote file read failed.
- Remote file upload failed.
- Remote file changed before upload.

Failed upload does not close the edit session. The user can save again after resolving the problem.

## Testing Strategy

Unit tests cover:

- Manifest parsing for `remoteFileEditors`.
- Catalog registration.
- Platform filtering.
- File-mask matching.
- Editor ordering.
- Command template rendering.
- Program candidate resolution.
- Conflict decision logic.
- Session temp path sanitization.

Integration-level checks cover:

- Terminal file manager context menu exposes external editor entries.
- SFTP view context menu exposes external editor entries.
- Built-in editor still works.
- Image preview still works.
- Missing executable entries are disabled or route to configuration.

## Acceptance Criteria

- Installed composite extensions can contribute external remote-file editors.
- The extension runtime catalog exposes registered remote-file editor contributions.
- Terminal file manager and standalone SFTP view show matching external editor actions.
- Selecting an external editor downloads the remote file to a temp file and launches the local editor.
- Saving the temp file uploads changes back to the remote file.
- Remote conflict checks happen before upload by default.
- Notepad++ can be shipped as a marketplace composite extension without app-specific hard-coding.
