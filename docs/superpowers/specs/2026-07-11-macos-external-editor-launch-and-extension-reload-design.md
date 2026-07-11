# macOS External Editor Launch and Extension Reload Design

## Goal

Fix two defects found while testing remote-file editor extensions:

1. Reloading a static composite extension blocks the UI while all language WASM
   extensions are synchronously recompiled.
2. Launching a macOS editor through its Bundle executable does not reliably open
   files in an existing application instance and leaves GUI applications tied to
   an unsuitable direct-child launch path.

## Confirmed Root Causes

`MainExtensionViewHost::reload` currently calls one global reload function for
every extension kind. That function always calls `reload_language_extensions`,
which loads and compiles every installed Tree-sitter WASM module on the GPUI
thread before refreshing the composite runtime catalog. Reloading the static
Notepad-- extension therefore produces extensive Cranelift compilation logs and
an unresponsive window even though no language extension changed.

The Notepad-- macOS implementation handles `QEvent::FileOpen` from LaunchServices.
When another instance already owns its shared-memory marker, a second direct
invocation of `Notepad--.app/Contents/MacOS/Notepad-- <file>` returns without
forwarding the command-line file. Zed also behaves as a GUI application and
should be handed off through LaunchServices instead of being retained as a
direct OnetCli child process.

## Kind-Aware Extension Reload

Split runtime refresh into explicit scopes:

- Language reload unregisters the selected language, reloads language
  extensions, refreshes the global runtime catalog, and refreshes runtime
  contributions.
- Composite and other non-language reloads skip language loading and refresh
  only the global runtime catalog and runtime contributions.
- The post-install/uninstall refresh path may continue to perform a full refresh
  until it receives extension-kind context; this change specifically guarantees
  that the per-item reload action is kind-aware.

The implementation must keep the existing synchronous `ExtensionViewHost`
contract for this focused fix. The non-language path contains only local
manifest scanning and GPUI global updates and is expected to complete quickly.

## macOS Launch Mode Contract

Add an optional `launchMode` property to a remote-file editor command. Supported
serialized values are:

- `direct`: existing behavior and default when omitted;
- `macos_open`: validate the declared executable candidate, derive its owning
  `.app` Bundle, then invoke `/usr/bin/open` through LaunchServices.

Example:

```json
{
  "launchMode": "macos_open",
  "programCandidates": [
    "/Applications/Notepad--.app/Contents/MacOS/Notepad--"
  ],
  "args": ["{file}"]
}
```

`programCandidates` retains its existing responsibilities:

- determine whether the editor is installed;
- provide the executable shown in the first-launch confirmation;
- support a local executable override.

For `macos_open`, the launcher finds the nearest ancestor whose extension is
`.app`. For the verified Notepad-- path it derives
`/Applications/Notepad--.app`; for Zed it derives `/Applications/Zed.app`.
Failure to find an `.app` ancestor returns an explicit launch error.

The final process invocation is logically:

```text
/usr/bin/open -a <derived-app-bundle> <rendered editor args>
```

The implementation continues to use `std::process::Command` directly and never
passes arguments through a shell. `macos_open` is accepted only on macOS at
launch time; other platforms return an explicit unsupported-mode error. Direct
launch behavior on Windows, Linux, and macOS remains unchanged.

## Data Flow

The extension manifest parser deserializes `launchMode` into the manifest command
model. Registration copies it into the runtime command model. Matching and local
executable resolution remain unchanged. The external-edit launch object carries
the mode alongside the executable and rendered argument templates. At launch,
the mode selects either the existing direct command or the macOS LaunchServices
command.

The local override continues to replace the editor executable and argument list
but does not replace the extension-declared launch mode. A macOS override for a
`macos_open` editor must therefore point to an executable inside an `.app`
Bundle. This preserves a simple settings model and prevents an override from
silently changing the security-relevant launch mechanism.

## Extension Updates

Update the Notepad-- macOS contribution and Zed macOS contribution to declare
`"launchMode": "macos_open"`. The Zed Linux contribution and Windows Notepad++
contribution omit the property and retain direct launch.

Increment the Notepad-- and Zed extension versions from `0.1.0` to `0.1.1`,
including build/release metadata and marketplace entries, so the corrected
packages are distinguishable and upgradeable.

## Testing

Use TDD to cover:

1. manifest parsing defaults an omitted `launchMode` to `direct`;
2. manifest parsing and runtime registration preserve `macos_open`;
3. `.app` Bundle derivation succeeds for Notepad-- and Zed executable paths;
4. Bundle derivation rejects an executable outside an `.app` directory;
5. launch command planning produces `/usr/bin/open`, `-a`, the Bundle path, and
   the rendered file argument without invoking a shell;
6. existing direct launch command planning is unchanged;
7. reload scope selection includes language loading only for language
   extensions;
8. composite reload refreshes the runtime catalog without calling language
   loading;
9. both extension repository manifests declare `macos_open` only for their
   macOS contributions and use version `0.1.1`.

Run focused tests for `extension-runtime`, `remote_file_editor`, and
`extension_view`, followed by the extension repository's complete Node script
suite. Build and verify both `0.1.1` composite archives.

## Local Delivery

Build a new release application Bundle from an isolated worktree, replace the
local `/Applications/OnetCli.app`, replace only the two installed editor
extension directories, and restart OnetCli. Preserve all unrelated extensions,
settings, connections, and uncommitted repository changes.

Manual verification must demonstrate:

- reloading Notepad-- does not trigger Cranelift language compilation logs or an
  unresponsive window;
- opening two remote files with an already-running Notepad-- instance delivers
  both files to that instance;
- opening a remote file with Zed uses LaunchServices and Zed remains independent
  of OnetCli lifecycle;
- saving still triggers the existing watcher, conflict check, and SFTP upload
  workflow.

## Non-Goals

- Making all extension installation and reload operations asynchronous.
- Changing language extension compilation behavior when a language extension is
  explicitly reloaded.
- Bundling, updating, or signing third-party editor applications.
- Adding shell command support or arbitrary launch scripts.
- Changing remote-file synchronization and conflict semantics.
