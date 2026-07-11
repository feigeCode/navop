# Notepad-- and Zed Remote Editor Extensions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add separately installable Notepad-- and Zed remote-file editor extensions, package and verify them, then install the current OnetCli development build and both extensions on this Mac.

**Architecture:** Reuse the existing static composite extension format and `contributes.remoteFileEditors` runtime contract. Add one package per editor; Zed has separate macOS and Linux contributions so each platform uses an unambiguous command candidate. No host runtime behavior changes are required.

**Tech Stack:** JSON extension manifests, Node.js built-in test runner, shell-based composite packager/verifier, Rust/Cargo macOS application bundle.

---

### Task 1: Add failing registration and manifest contract tests

**Files:**
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/tests/scripts.test.mjs`

- [ ] **Step 1: Add marketplace registration assertions**

Add this test:

```js
test("Notepad-- and Zed editors are registered as static composite extensions", () => {
  const globalManifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, "manifest.json"), "utf8"),
  );
  const expected = [
    {
      id: "notepad-minus-minus-editor",
      name: "Notepad-- External Editor",
      releaseTag: "notepad-minus-minus-editor-v0.1.0",
      manifest: "notepad-minus-minus-editor/manifest.json",
    },
    {
      id: "zed-editor",
      name: "Zed External Editor",
      releaseTag: "zed-editor-v0.1.0",
      manifest: "zed-editor/manifest.json",
    },
  ];

  for (const item of expected) {
    const entry = globalManifest.extensions.find(
      (extension) => extension.id === item.id,
    );
    assert.equal(entry?.kind, "composite");
    assert.equal(entry?.name, item.name);
    assert.equal(entry?.version, "0.1.0");
    assert.equal(entry?.release_tag, item.releaseTag);
    assert.equal(entry?.manifest, item.manifest);
  }
});
```

- [ ] **Step 2: Add extension manifest contract assertions**

Add this test to read the two source `extension.json` files and assert their
complete command contracts:

```js
test("Notepad-- and Zed editor manifests declare platform-specific commands", () => {
  const readManifest = (id) => JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, `extensions/wasm/${id}/extension.json`),
      "utf8",
    ),
  );
  const notepadManifest = readManifest("notepad-minus-minus-editor");
  const zedManifest = readManifest("zed-editor");
  const notepadEditor = notepadManifest.contributes.remoteFileEditors[0];
  const [zedMacos, zedLinux] = zedManifest.contributes.remoteFileEditors;

  assert.equal(notepadEditor.id, "notepad-minus-minus");
assert.deepEqual(notepadEditor.platforms, ["macos"]);
assert.deepEqual(notepadEditor.fileMasks, ["*"]);
assert.equal(notepadEditor.priority, 100);
assert.deepEqual(notepadEditor.command.programCandidates, [
  "/Applications/Notepad--.app/Contents/MacOS/Notepad--",
]);
assert.deepEqual(notepadEditor.command.args, ["{file}"]);

assert.deepEqual(zedMacos.platforms, ["macos"]);
assert.deepEqual(zedMacos.command.programCandidates, [
  "/Applications/Zed.app/Contents/MacOS/zed",
]);
assert.deepEqual(zedLinux.platforms, ["linux"]);
assert.deepEqual(zedLinux.command.programCandidates, ["zed"]);
  assert.deepEqual(zedMacos.fileMasks, ["*"]);
  assert.deepEqual(zedLinux.fileMasks, ["*"]);
  assert.equal(zedMacos.priority, 90);
  assert.equal(zedLinux.priority, 90);
assert.deepEqual(zedMacos.command.args, ["{file}"]);
assert.deepEqual(zedLinux.command.args, ["{file}"]);
});
```

- [ ] **Step 3: Run the focused tests and confirm the red state**

Run:

```bash
rtk node --test --test-name-pattern='Notepad--|Zed' tests/scripts.test.mjs
```

Expected: FAIL because the marketplace entries and source manifests do not yet
exist.

### Task 2: Add the Notepad-- static composite extension

**Files:**
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/notepad-minus-minus-editor/extension.build.json`
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/notepad-minus-minus-editor/extension.json`
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/notepad-minus-minus-editor/README.md`
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/manifest.json`

- [ ] **Step 1: Add build metadata**

```json
{
  "id": "notepad-minus-minus-editor",
  "kind": "composite",
  "language": "static",
  "path": "extensions/wasm/notepad-minus-minus-editor",
  "targets": ["universal"],
  "releaseTagPrefix": "notepad-minus-minus-editor-v",
  "r2Prefix": "extensions/notepad-minus-minus-editor"
}
```

- [ ] **Step 2: Add the runtime manifest**

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.notepad-minus-minus",
  "name": "Notepad-- External Editor",
  "description": "Static composite extension for editing OnetCli SFTP remote files with Notepad--.",
  "version": "0.1.0",
  "publisher": "OnetCli",
  "engines": { "onetcli": ">=0.8.6" },
  "categories": ["Editor", "SFTP"],
  "contributes": {
    "remoteFileEditors": [
      {
        "id": "notepad-minus-minus",
        "displayName": "Notepad--",
        "platforms": ["macos"],
        "fileMasks": ["*"],
        "priority": 100,
        "command": {
          "programCandidates": [
            "/Applications/Notepad--.app/Contents/MacOS/Notepad--"
          ],
          "args": ["{file}"]
        }
      }
    ]
  }
}
```

- [ ] **Step 3: Add the README**

```markdown
# Notepad-- External Editor

This static composite extension contributes Notepad-- as an external editor for
OnetCli SFTP remote files. The host application owns the SFTP connection,
temporary file, change watcher, conflict prompt, and upload workflow.

The extension contains no executable code and receives no credentials. It only
declares the standard macOS Notepad-- executable and the `{file}` argument.

After installation, right-click a remote file and choose **Edit With Notepad--**.
For a non-standard installation, configure the executable path in OnetCli
Settings under **Remote File Editor**.
```

- [ ] **Step 4: Register the marketplace entry**

Insert a composite entry beside the existing Notepad++ entry:

```json
{
  "id": "notepad-minus-minus-editor",
  "kind": "composite",
  "name": "Notepad-- External Editor",
  "version": "0.1.0",
  "release_tag": "notepad-minus-minus-editor-v0.1.0",
  "description": "Static composite extension for editing OnetCli SFTP remote files with Notepad--.",
  "file_extensions": [],
  "manifest": "notepad-minus-minus-editor/manifest.json"
}
```

### Task 3: Add the Zed static composite extension

**Files:**
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/zed-editor/extension.build.json`
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/zed-editor/extension.json`
- Create: `/Users/hufei/RustroverProjects/onetcli-extensions/extensions/wasm/zed-editor/README.md`
- Modify: `/Users/hufei/RustroverProjects/onetcli-extensions/manifest.json`

- [ ] **Step 1: Add build metadata**

```json
{
  "id": "zed-editor",
  "kind": "composite",
  "language": "static",
  "path": "extensions/wasm/zed-editor",
  "targets": ["universal"],
  "releaseTagPrefix": "zed-editor-v",
  "r2Prefix": "extensions/zed-editor"
}
```

- [ ] **Step 2: Add the runtime manifest**

```json
{
  "schema_version": 1,
  "id": "com.onetcli.editor.zed",
  "name": "Zed External Editor",
  "description": "Static composite extension for editing OnetCli SFTP remote files with Zed.",
  "version": "0.1.0",
  "publisher": "OnetCli",
  "engines": { "onetcli": ">=0.8.6" },
  "categories": ["Editor", "SFTP"],
  "contributes": {
    "remoteFileEditors": [
      {
        "id": "zed-macos",
        "displayName": "Zed",
        "platforms": ["macos"],
        "fileMasks": ["*"],
        "priority": 90,
        "command": {
          "programCandidates": [
            "/Applications/Zed.app/Contents/MacOS/zed"
          ],
          "args": ["{file}"]
        }
      },
      {
        "id": "zed-linux",
        "displayName": "Zed",
        "platforms": ["linux"],
        "fileMasks": ["*"],
        "priority": 90,
        "command": {
          "programCandidates": ["zed"],
          "args": ["{file}"]
        }
      }
    ]
  }
}
```

- [ ] **Step 3: Add the README**

```markdown
# Zed External Editor

This static composite extension contributes Zed as an external editor for
OnetCli SFTP remote files. The host application owns the SFTP connection,
temporary file, change watcher, conflict prompt, and upload workflow.

The extension contains no executable code and receives no credentials. It uses
the standard Zed application executable on macOS and the `zed` PATH command on
Linux.

After installation, right-click a remote file and choose **Edit With Zed**. For
a non-standard installation, configure the executable path in OnetCli Settings
under **Remote File Editor**.
```

- [ ] **Step 4: Register the marketplace entry**

```json
{
  "id": "zed-editor",
  "kind": "composite",
  "name": "Zed External Editor",
  "version": "0.1.0",
  "release_tag": "zed-editor-v0.1.0",
  "description": "Static composite extension for editing OnetCli SFTP remote files with Zed.",
  "file_extensions": [],
  "manifest": "zed-editor/manifest.json"
}
```

### Task 4: Make the extension tests green

**Files:**
- Verify: `/Users/hufei/RustroverProjects/onetcli-extensions/tests/scripts.test.mjs`
- Verify: both new extension directories and `/Users/hufei/RustroverProjects/onetcli-extensions/manifest.json`

- [ ] **Step 1: Run focused contract tests**

```bash
rtk node --test --test-name-pattern='Notepad--|Zed' tests/scripts.test.mjs
```

Expected: all matching tests PASS.

- [ ] **Step 2: Run the complete script suite**

```bash
rtk node --test tests/scripts.test.mjs
```

Expected: zero failures.

- [ ] **Step 3: Check formatting and unintended changes**

```bash
rtk git diff --check
rtk git status --short
```

Expected: no whitespace errors; the pre-existing untracked `connection.ncx`
remains untouched and untracked.

### Task 5: Build and verify both extension archives

**Files:**
- Read: `/Users/hufei/RustroverProjects/onetcli-extensions/scripts/release-driver.mjs`
- Output: `/tmp/onetcli-editor-extensions/*.tar.gz`

- [ ] **Step 1: Package Notepad--**

```bash
rtk node scripts/release-driver.mjs notepad-minus-minus-editor 0.1.0 --artifact-dir /tmp/onetcli-editor-extensions
```

Expected artifact:
`/tmp/onetcli-editor-extensions/notepad-minus-minus-editor-composite-universal.tar.gz`.

- [ ] **Step 2: Package Zed**

```bash
rtk node scripts/release-driver.mjs zed-editor 0.1.0 --artifact-dir /tmp/onetcli-editor-extensions
```

Expected artifact:
`/tmp/onetcli-editor-extensions/zed-editor-composite-universal.tar.gz`.

- [ ] **Step 3: Verify both archives explicitly**

```bash
rtk bash scripts/verify-composite-package.sh /tmp/onetcli-editor-extensions/notepad-minus-minus-editor-composite-universal.tar.gz
rtk bash scripts/verify-composite-package.sh /tmp/onetcli-editor-extensions/zed-editor-composite-universal.tar.gz
```

Expected: both commands report verified packages and exit successfully.

### Task 6: Build the current OnetCli macOS application bundle

**Files:**
- Read: `/Users/hufei/RustroverProjects/onetcli/script/bundle-macos.sh`
- Output: `/Users/hufei/RustroverProjects/onetcli/target/OnetCli.app`

- [ ] **Step 1: Determine the native Rust target**

```bash
rtk rustc -vV
```

Expected on this machine: host target `aarch64-apple-darwin`.

- [ ] **Step 2: Build the release binary**

```bash
rtk cargo build --release -p main --target aarch64-apple-darwin
```

Expected: successful release build with
`target/aarch64-apple-darwin/release/onetcli`.

- [ ] **Step 3: Create the application bundle**

```bash
rtk bash script/bundle-macos.sh aarch64-apple-darwin
```

Expected: `target/OnetCli.app/Contents/MacOS/onetcli` exists and is executable.

### Task 7: Install the application and extensions locally

**Files:**
- Install app: `/Applications/OnetCli.app`
- Install extensions: `/Users/hufei/.config/one-hub/extensions/composite/notepad-minus-minus-editor`
- Install extensions: `/Users/hufei/.config/one-hub/extensions/composite/zed-editor`

- [ ] **Step 1: Request approval for writes outside the workspace**

Request escalated filesystem permission for these exact installation commands.
The commands preserve all application data and unrelated extension directories.

- [ ] **Step 2: Replace the local application Bundle atomically enough for trial use**

Run:

```bash
rtk ditto target/OnetCli.app /Applications/OnetCli.app
```

Do not terminate an existing OnetCli process. If it is running, report that a
restart is required to execute the newly copied binary.

- [ ] **Step 3: Install each extension from its verified archive**

Run package-specific extraction commands and leave every other extension
directory untouched:

```bash
rtk mkdir -p /Users/hufei/.config/one-hub/extensions/composite/notepad-minus-minus-editor
rtk mkdir -p /Users/hufei/.config/one-hub/extensions/composite/zed-editor
rtk tar -xzf /tmp/onetcli-editor-extensions/notepad-minus-minus-editor-composite-universal.tar.gz -C /Users/hufei/.config/one-hub/extensions/composite/notepad-minus-minus-editor
rtk tar -xzf /tmp/onetcli-editor-extensions/zed-editor-composite-universal.tar.gz -C /Users/hufei/.config/one-hub/extensions/composite/zed-editor
```

### Task 8: Fresh completion verification

**Files:**
- Verify: `/Applications/OnetCli.app/Contents/MacOS/onetcli`
- Verify: `/Users/hufei/.config/one-hub/extensions/composite/notepad-minus-minus-editor/extension.json`
- Verify: `/Users/hufei/.config/one-hub/extensions/composite/zed-editor/extension.json`

- [ ] **Step 1: Verify installed files and JSON**

Confirm the application executable is present and executable. Parse both
installed manifests with `plutil` or Node JSON parsing and assert their runtime
extension ids and editor candidate paths.

- [ ] **Step 2: Verify editor executable discovery inputs**

Confirm these files are executable:

```text
/Applications/Notepad--.app/Contents/MacOS/Notepad--
/Applications/Zed.app/Contents/MacOS/zed
```

- [ ] **Step 3: Inspect installed extension discovery layout**

List the composite root and confirm each installed package has `extension.json`
at the directory depth consumed by `ExtensionRuntimeCatalog::from_installed_composite_root`.

- [ ] **Step 4: Report the manual smoke-test flow**

Tell the user to restart OnetCli, open an SFTP connection, right-click a remote
text file, choose **Edit With Notepad--** or **Edit With Zed**, accept the first
launch confirmation, save the file, and verify the upload/conflict workflow.
