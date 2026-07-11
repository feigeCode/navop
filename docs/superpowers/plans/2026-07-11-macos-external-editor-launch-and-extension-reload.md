# macOS External Editor Launch and Extension Reload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make composite extension reload fast and reliable, and launch Notepad--/Zed remote files through macOS LaunchServices while preserving executable discovery and confirmation.

**Architecture:** Add a defaulted `RemoteFileEditorLaunchMode` to the manifest and registered runtime command, then plan either a direct process or `/usr/bin/open -a <bundle>` without a shell. Split extension reload behavior by kind so only language extensions trigger Tree-sitter WASM loading. Update the two editor extensions to version 0.1.1 and opt their macOS contributions into `macos_open`.

**Tech Stack:** Rust, serde, GPUI extension host, `std::process::Command`, Node.js extension repository tests, macOS LaunchServices.

---

### Task 1: Add failing manifest and runtime registration tests

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/parser_tests.rs`
- Modify: `crates/extension-runtime/src/extension_runtime_contract_tests.rs`

- [ ] Add a parser test manifest with `"launchMode": "macos_open"` and assert the parsed command mode.
- [ ] Extend the existing omitted-field test to assert `direct` is the default.
- [ ] Extend runtime catalog registration tests to assert the registered command preserves `macos_open`.
- [ ] Run:

```bash
rtk cargo test -p extension-runtime remote_file_editor -- --nocapture
```

Expected: compile/test failure because the launch-mode type and fields do not exist.

### Task 2: Implement the launch-mode data contract

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/contributes.rs`
- Modify: `crates/extension-runtime/src/types.rs`
- Modify: `crates/extension-runtime/src/registration.rs`

- [ ] Add the serialized enum:

```rust
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileEditorLaunchMode {
    #[default]
    Direct,
    MacosOpen,
}
```

- [ ] Add `#[serde(default, rename = "launchMode")]` to the manifest command.
- [ ] Add the mode to `RegisteredRemoteFileEditorCommand` and copy it during registration.
- [ ] Re-run the focused extension-runtime tests and confirm they pass.

### Task 3: Add failing launch command planning tests

**Files:**
- Modify: `crates/remote_file_editor/src/external_launcher.rs`

- [ ] Add tests asserting a direct launch plan keeps the original executable and arguments.
- [ ] Add tests asserting Notepad-- and Zed executable paths derive their nearest `.app` Bundle.
- [ ] Add a rejection test for `/usr/local/bin/editor` under `macos_open`.
- [ ] Add a test asserting the macOS launch plan is exactly:

```text
program: /usr/bin/open
args: -a, /Applications/Notepad--.app, /tmp/remote file.html
```

- [ ] Run:

```bash
rtk cargo test -p remote_file_editor external_launcher -- --nocapture
```

Expected: failure because launch planning and Bundle derivation are not implemented.

### Task 4: Implement direct and macOS LaunchServices command planning

**Files:**
- Modify: `crates/remote_file_editor/src/external_launcher.rs`
- Modify: `crates/remote_file_editor/src/external_editor.rs`
- Modify: `crates/remote_file_editor/src/lib.rs`

- [ ] Add a small `ExternalLaunchCommand` value containing `program` and `args`.
- [ ] Implement `.app` ancestor derivation using `Path::ancestors` and a case-insensitive `.app` extension check.
- [ ] Plan `Direct` as the current executable plus rendered args.
- [ ] On macOS, plan `MacosOpen` as `/usr/bin/open`, followed by `-a`, the derived Bundle, and rendered args.
- [ ] On non-macOS, return an explicit unsupported launch-mode error.
- [ ] Change `launch_external_editor` to spawn the planned command and keep shell-free argument passing.
- [ ] Carry the registered launch mode into `ExternalEditLaunch` and use it from `launch`.
- [ ] Run all `remote_file_editor` tests.

### Task 5: Add failing scoped reload tests

**Files:**
- Modify: `crates/extension-runtime/src/extension_view_host.rs`

- [ ] Add a pure reload-scope selector test asserting:

```text
Language  -> reload languages
Composite -> skip language reload
DatabaseDriver/other kinds -> skip language reload
```

- [ ] Run:

```bash
rtk cargo test -p extension-runtime extension_view_host -- --nocapture
```

Expected: failure because reload scope selection does not exist.

### Task 6: Implement kind-aware per-item reload

**Files:**
- Modify: `crates/extension-runtime/src/extension_view_host.rs`

- [ ] Pass the selected extension kind into the per-item runtime reload path.
- [ ] Call `reload_language_extensions` only for `ExtensionKind::Language`.
- [ ] Always refresh the global runtime catalog and runtime contributions.
- [ ] Preserve the current full refresh behavior for install/uninstall callbacks that do not carry kind context.
- [ ] Run focused `extension-runtime` and `extension_view` tests.

### Task 7: Run main-repository quality gates

**Files:**
- Verify all modified Rust files.

- [ ] Run formatting:

```bash
rtk cargo fmt --all -- --check
```

- [ ] Run the focused test set with one test thread to avoid shared runtime and
  temporary-storage interference:

```bash
rtk cargo test -p extension-runtime -p remote_file_editor -p extension_view -- --test-threads=1
```

- [ ] Run checks:

```bash
rtk cargo check -p extension-runtime -p remote_file_editor -p extension_view -p main
```

- [ ] Run `rtk git diff --check` and review the focused diff.

### Task 8: Update Notepad-- and Zed extensions to 0.1.1 with TDD

**Files in `onetcli-extensions`:**
- Modify: `extensions/wasm/notepad-minus-minus-editor/extension.json`
- Modify: `extensions/wasm/zed-editor/extension.json`
- Modify: `manifest.json`
- Modify: `tests/scripts.test.mjs`
- Modify: both extension README files if launch behavior is documented.

- [ ] First update tests to require version `0.1.1`, release tags ending in `v0.1.1`, and `launchMode: "macos_open"` on Notepad-- and Zed macOS contributions only.
- [ ] Run the focused Node tests and confirm they fail against 0.1.0 manifests.
- [ ] Update source manifests and marketplace metadata to 0.1.1.
- [ ] Document LaunchServices behavior in both READMEs.
- [ ] Run focused tests, then:

```bash
rtk node --test tests/scripts.test.mjs
```

Expected: zero failures.

### Task 9: Package and verify corrected extension archives

**Files:**
- Output: `/tmp/onetcli-editor-extensions-0.1.1/*.tar.gz`

- [ ] Run both release drivers with version `0.1.1`.
- [ ] Verify both composite archives explicitly with `verify-composite-package.sh`.
- [ ] Inspect packaged `extension.json` files and confirm only macOS contributions use `macos_open`.

### Task 10: Review, build, install, and smoke test

**Files:**
- Install: `/Applications/OnetCli.app`
- Install: `~/.config/one-hub/extensions/composite/notepad-minus-minus-editor`
- Install: `~/.config/one-hub/extensions/composite/zed-editor`

- [ ] Perform a focused code review against the approved design and fix Critical/Important findings.
- [ ] Run completion verification immediately before claiming success.
- [ ] Build `main` release for `aarch64-apple-darwin` in an isolated worktree and create `target/OnetCli.app`.
- [ ] With approval, replace the local application Bundle and only the two editor extension directories.
- [ ] Start OnetCli and verify logs load extension versions 0.1.1.
- [ ] Reload Notepad-- and confirm no Cranelift/Wasmtime language compilation starts.
- [ ] Open a remote file twice with an already-running Notepad-- and confirm LaunchServices delivers both files.
- [ ] Open a remote file with Zed and confirm it is launched through LaunchServices and remains independent of OnetCli lifecycle.
