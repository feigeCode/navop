# Remote Desktop Provider Extensions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move RDP and VNC protocol implementations into downloadable `onetcli-extensions` remote desktop provider packages while keeping built-in RDP/VNC new-connection options visible; opening an existing or newly created RDP/VNC connection prompts to download the missing provider before continuing.

**Architecture:** Keep `remote_desktop_view` and the RDP/VNC creation forms as built-in host UI, and make `remote_desktop` a lightweight provider client, manifest parser, and helper-process adapter. Add `remote_desktop_provider` as a first-class extension kind beside database drivers, with packages installed under `extensions/remote_desktop_providers/<id>` and marketplace entries resolved through the existing manifest/download/install pipeline. RDP reuses the existing helper process first; VNC is migrated to a matching helper so `vnc-rs` no longer needs to be in the main application dependency graph.

**Tech Stack:** Rust 2024, GPUI, existing `extension-runtime` marketplace downloader, existing database IPC extension patterns, JSON-line remote desktop helper protocol, `tools/rdp-helper`, new VNC helper package in `onetcli-extensions`.

---

## Current Progress

- Main repo: remote desktop provider manifest/registry, `remote_desktop_provider` extension kind, marketplace install guard, and open-time provider check are implemented.
- Main repo: RDP/VNC new connection options remain built in; provider discovery is used only when a connection is opened.
- Main repo review fix: remote desktop provider discovery now uses the same `one_core::storage::get_config_dir()/extensions/remote_desktop_providers` directory as `ExtensionRegistry`, so a provider installed from the prompt is found by the next open-time guard/backend lookup.
- Main repo review fix: helper startup now honors provider manifest `entry.command`, `entry.args`, and `entry.working_dir` instead of only the command path.
- Main repo: `remote_desktop` no longer depends on `vnc-rs`; old in-process VNC modules were removed from the compiled module tree and deleted.
- `onetcli-extensions`: RDP and VNC provider package manifests, helper crates, package verification scripts, marketplace manifest generation, release workflow support, and root marketplace entries are implemented.
- `onetcli-extensions` review fix: remote desktop provider packaging now finds binaries produced by independent helper crates built with `cargo build --manifest-path`, whose default target directory is under the helper crate.
- Verified so far:
  - `cargo test -p remote_desktop -- --nocapture`
  - `cargo test -p extension-runtime remote_desktop_provider -- --nocapture`
  - `cargo test -p extension-runtime remote_desktop_provider_install -- --nocapture`
  - `cargo test -p main remote_desktop_kinds -- --nocapture`
  - `CLANG_MODULE_CACHE_PATH=/tmp/clang-cache XDG_CACHE_HOME=/tmp/onetcli-xdg-cache cargo check -p remote_desktop -p remote_desktop_view -p extension-runtime -p main` with elevated sandbox permissions because GPUI Metal shader compilation writes clang module cache under the user cache directory.
  - In `onetcli-extensions`: `node --test tests/scripts.test.mjs`
  - In `onetcli-extensions`: `cargo check --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml`
  - In `onetcli-extensions`: `cargo test --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml -- --nocapture`
  - In `onetcli-extensions`: `cargo check --manifest-path extensions/remote-desktop/vnc-helper/Cargo.toml`
  - In `onetcli-extensions`: `cargo test --manifest-path extensions/remote-desktop/vnc-helper/Cargo.toml -- --nocapture`

### Task 1: Add Remote Desktop Provider Manifest And Registry

**Files:**
- Create: `crates/remote_desktop/src/provider.rs`
- Create: `crates/remote_desktop/src/provider_registry.rs`
- Modify: `crates/remote_desktop/src/lib.rs`
- Test: `crates/remote_desktop/src/provider_registry.rs`

- [ ] **Step 1: Write registry tests**

Add tests that create temporary provider directories containing `remote_desktop_provider.json`, then assert:
- `RemoteDesktopProviderRegistry::load_from_dir(root)` loads providers sorted by `id`.
- duplicate provider ids keep the first directory.
- missing or invalid manifests are skipped in the report.
- relative `entry.command` remains relative when it exists inside the manifest directory, matching database driver behavior.

Run: `cargo test -p remote_desktop provider_registry -- --nocapture`

Expected: fails because the provider types do not exist.

- [ ] **Step 2: Implement manifest types**

Create `RemoteDesktopProviderManifest` with these fields:
- `id: String`
- `name: String`
- `description: String`
- `version: String`
- `protocol: RemoteDesktopProtocol`
- `entry: RemoteDesktopProviderEntry { command, args, working_dir }`
- `capabilities: RemoteDesktopCapabilities`
- `ui: RemoteDesktopProviderUi { icon, default_port }`
- `manifest_dir: PathBuf` skipped from serde

Validation must require non-empty `id`, `name`, and `entry.command`. It must reject path separators in `id`.

- [ ] **Step 3: Implement registry loading**

Create `RemoteDesktopProviderRegistry` with:
- `load_default()`
- `load_from_dir(root: &Path)`
- `load_from_dir_with_report(root: &Path)`
- `load_provider_from_dir(dir: &Path)`
- `find(protocol: RemoteDesktopProtocol)`
- `find_by_id(id: &str)`
- `providers()`

Default directory must be `get_config_dir()/extensions/remote_desktop_providers`, with `ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR` override for local development.

- [ ] **Step 4: Export the provider API**

Export the manifest, registry, loaded/skipped report entries, and default directory helpers from `remote_desktop::lib`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p remote_desktop provider_registry -- --nocapture
```

Expected: provider registry tests pass.

### Task 2: Convert Remote Desktop Backend Creation To Provider Helper Adapter

**Files:**
- Modify: `crates/remote_desktop/src/backend.rs`
- Modify: `crates/remote_desktop/src/backends/rdp.rs`
- Modify: `crates/remote_desktop/src/config.rs`
- Modify: `crates/remote_desktop/src/helper_protocol.rs`
- Test: `crates/remote_desktop/src/backends/rdp.rs`

- [ ] **Step 1: Write backend selection tests**

Add tests proving:
- `create_backend(options, registry)` returns a helper backend when a provider for the protocol exists.
- missing provider returns an error that includes the protocol label.
- provider command is resolved relative to `manifest_dir`.

Run: `cargo test -p remote_desktop backend -- --nocapture`

Expected: fails because `create_backend` still hardcodes RDP/VNC.

- [ ] **Step 2: Introduce a generic helper backend**

Rename the host-side helper adapter from RDP-specific to provider-generic:
- `RdpBackend` becomes `HelperProcessBackend`.
- status messages use `options.protocol.label()` instead of hardcoded `RDP`.
- helper process path comes from `RemoteDesktopProviderManifest.entry.command`.
- environment fallback `ONETCLI_RDP_HELPER` remains only as a compatibility fallback for protocol `Rdp` while the new provider package is being introduced.

- [ ] **Step 3: Preserve protocol-specific key conversion**

Keep `HelperRequest::from_remote_input` in the host for the current JSON-line protocol. RDP still receives scancode key requests. VNC helper will initially accept the same shape and map scancode/named keys inside the helper package.

- [ ] **Step 4: Remove hardcoded VNC backend from default creation**

Make `create_backend` use the provider registry. Leave the old VNC modules temporarily compiled until the VNC helper package lands, but route normal app creation through provider lookup so missing VNC can be detected before opening.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p remote_desktop backend provider_registry -- --nocapture
```

Expected: backend and provider registry tests pass.

### Task 3: Add Extension Runtime Support For Remote Desktop Providers

**Files:**
- Modify: `crates/extension-runtime/src/extension/kind.rs`
- Create: `crates/extension-runtime/src/extension/remote_desktop_provider.rs`
- Modify: `crates/extension-runtime/src/extension/mod.rs`
- Modify: `crates/extension-runtime/src/extension_downloader.rs`
- Modify: `crates/extension-runtime/src/extension_package_layout.rs`
- Test: `crates/extension-runtime/src/extension/provider_tests.rs`
- Test: `crates/extension-runtime/src/extension_downloader_archive_tests.rs`
- Test: `crates/extension-runtime/src/extension_downloader_network_tests.rs`

- [ ] **Step 1: Write extension kind tests**

Add tests that assert:
- serde parses `"remote_desktop_provider"` into `ExtensionKind::RemoteDesktopProvider`.
- `ExtensionKind::RemoteDesktopProvider.dir_name()` is `"remote_desktop_providers"`.
- package detection recognizes `remote_desktop_provider.json`.
- install name for provider packages comes from manifest field `id`.

Run: `cargo test -p extension-runtime remote_desktop_provider -- --nocapture`

Expected: fails because the extension kind is missing.

- [ ] **Step 2: Extend ExtensionKind**

Add `RemoteDesktopProvider` to `ExtensionKind`, `dir_name()`, and `all()`.

- [ ] **Step 3: Add provider implementation**

Create `RemoteDesktopProviderExtensionProvider` implementing `ExtensionProvider` by delegating to `remote_desktop::RemoteDesktopProviderRegistry`. Summary fields:
- `kind = ExtensionKind::RemoteDesktopProvider`
- `name = manifest.id`
- `description = manifest.description` or `"<name> remote desktop provider"`
- `version = manifest.version` or `"0.0.0"`
- `icon = manifest.ui.icon` when present
- `default_port = manifest.ui.default_port`

Add `remote_desktop` as a dependency of `extension-runtime` if it is not already present.

- [ ] **Step 4: Register provider and package detection**

Register the new provider in `builtin_registry`. Update package layout and install-name logic to recognize `remote_desktop_provider.json`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p extension-runtime remote_desktop_provider extension_downloader -- --nocapture
```

Expected: remote desktop provider install/detect tests pass.

### Task 4: Keep Built-In New Connection Options

**Files:**
- Modify: `main/src/new_connection/connection_kind.rs`
- Test: `main/src/new_connection/connection_kind.rs`

- [ ] **Step 1: Preserve visibility tests**

Keep or add tests proving `NewConnectionKind::all()` always contains `NewConnectionKind::Rdp` and `NewConnectionKind::Vnc`, even when no remote desktop provider is installed. This preserves the database IPC interaction model: users can create a connection first, then install the missing provider when they open it.

Run: `cargo test -p main remote_desktop_kinds -- --nocapture`

Expected: passes after extension/provider changes because RDP/VNC remain hardcoded creation options.

- [ ] **Step 2: Avoid provider-driven form discovery**

Do not replace `NewConnectionKind::Rdp` or `NewConnectionKind::Vnc` with provider-derived variants. The provider registry is used at open time only.

- [ ] **Step 3: Preserve existing connection storage**

Keep `ConnectionType::Rdp` and `ConnectionType::Vnc` in storage. New remote desktop connections still save the same `RemoteDesktopParams` shape and connection type based on protocol, so existing sync/cloud data remains compatible.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p main remote_desktop_kinds -- --nocapture
```

Expected: new connection availability tests pass.

### Task 5: Prompt Download For Existing Connections Missing A Provider

**Files:**
- Create: `crates/extension-runtime/src/remote_desktop_provider_install.rs`
- Modify: `crates/extension-runtime/src/lib.rs`
- Modify: `main/src/home/home_strategy.rs`
- Modify: `main/src/home/home_tabs.rs`
- Test: `crates/extension-runtime/src/remote_desktop_provider_install.rs`
- Test: `main/src/home/home_strategy.rs`

- [ ] **Step 1: Write requirement tests**

Add tests proving:
- `required_provider_for_protocol(Rdp)` returns provider id `"rdp"`.
- `find_remote_desktop_provider_entry(entries, "rdp")` matches only `ExtensionKind::RemoteDesktopProvider`.
- marketplace install installs a fake `remote_desktop_provider.json` package into `remote_desktop_providers/rdp`.

Run: `cargo test -p extension-runtime remote_desktop_provider_install -- --nocapture`

Expected: fails because install guard is missing.

- [ ] **Step 2: Mirror database driver install guard**

Implement `open_remote_desktop_connection_with_provider_guard` following `database_driver_install.rs`:
- if installed provider exists for the protocol, call the opener immediately.
- if missing, prompt `"需要安装远程桌面插件"`.
- on confirmation, fetch default marketplace manifest, find matching `remote_desktop_provider`, download with progress, install through `ExtensionRegistry`, then open the connection.

- [ ] **Step 3: Use guard in HomePage open strategy**

Change `RemoteDesktopOpenStrategy` to call the guard instead of directly calling `home.open_remote_desktop`.

- [ ] **Step 4: Make HomePage implement opener trait**

Add `RemoteDesktopConnectionOpener for HomePage` so the guard can call back into `open_remote_desktop`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p extension-runtime remote_desktop_provider_install -- --nocapture
cargo test -p main home_strategy -- --nocapture
```

Expected: install guard tests pass.

### Task 6: Move VNC Into An External Helper Package

**Files:**
- Create in `../onetcli-extensions`: `cmd/vnc-helper` or `tools/vnc-helper`
- Create in `../onetcli-extensions`: `extensions/remote-desktop/vnc/remote_desktop_provider.json`
- Modify in `../onetcli-extensions`: `Cargo.toml`
- Modify in `onetcli`: `crates/remote_desktop/Cargo.toml`
- Modify in `onetcli`: `crates/remote_desktop/src/backends/mod.rs`

- [ ] **Step 1: Copy VNC backend logic into helper**

Create a VNC helper binary that reads the same JSON-line `HelperRequest` protocol from stdin and writes `HelperEvent` to stdout. Move or copy the VNC session loop, encoding, auth, input, and keyboard mapping from `crates/remote_desktop/src/backends/vnc*` into the extension repo.

- [ ] **Step 2: Add package manifest**

Create `remote_desktop_provider.json`:

```json
{
  "id": "vnc",
  "name": "VNC",
  "description": "VNC remote desktop provider",
  "version": "0.1.0",
  "protocol": "vnc",
  "entry": { "command": "./onetcli-vnc-helper" },
  "capabilities": {
    "clipboard_text": true,
    "resize": false,
    "cursor": true
  },
  "ui": {
    "default_port": 5900
  }
}
```

- [ ] **Step 3: Remove VNC dependency from main remote_desktop crate**

Delete `vnc_client` from `crates/remote_desktop/Cargo.toml` after the helper package compiles, and stop compiling the old in-process VNC modules.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p remote_desktop -- --nocapture
cargo check -p remote_desktop -p remote_desktop_view -p main
```

Expected: main crates pass without `vnc-rs` in the `remote_desktop` dependency graph.

### Task 7: Package RDP Provider In onetcli-extensions

**Files:**
- Move or copy: `tools/rdp-helper` into `../onetcli-extensions`
- Create: `../onetcli-extensions/extensions/remote-desktop/rdp/remote_desktop_provider.json`
- Modify: `../onetcli-extensions/Cargo.toml`
- Modify: `../onetcli-extensions/scripts/generate-marketplace-manifest.mjs`
- Create or modify: `../onetcli-extensions/scripts/package-remote-desktop-provider.sh`
- Modify: `../onetcli-extensions/manifest.json`
- Modify: `../onetcli-extensions/README.md`
- Modify: `../onetcli-extensions/README.zh-CN.md`

- [ ] **Step 1: Add RDP provider manifest**

Create `remote_desktop_provider.json`:

```json
{
  "id": "rdp",
  "name": "RDP",
  "description": "RDP remote desktop provider",
  "version": "0.1.0",
  "protocol": "rdp",
  "entry": { "command": "./onetcli-rdp-helper" },
  "capabilities": {
    "clipboard_text": true,
    "resize": true,
    "cursor": true
  },
  "ui": {
    "default_port": 3389
  }
}
```

- [ ] **Step 2: Add package script**

Create a packaging script equivalent to `scripts/package-driver.sh` that:
- reads `extensions/remote-desktop/<id>/extension.build.json`;
- copies the helper binary;
- rewrites `remote_desktop_provider.json.version`;
- rewrites `entry.command` to the packaged binary name;
- emits `<id>-remote-desktop-provider-<target>.tar.gz`.

- [ ] **Step 3: Extend marketplace manifest generation**

Update `scripts/generate-marketplace-manifest.mjs` so `loadExtensionMetadata()` searches `extensions/remote-desktop`, reads `remote_desktop_provider.json`, and emits entries with `kind: "remote_desktop_provider"`.

- [ ] **Step 4: Update root marketplace index**

Add `rdp` and `vnc` entries to `../onetcli-extensions/manifest.json`, each pointing to `<id>/manifest.json` and using kind `remote_desktop_provider`.

- [ ] **Step 5: Verify**

Run:

```bash
node ../onetcli-extensions/scripts/generate-marketplace-manifest.mjs
```

Expected: with the required env vars and checksum file, generated plugin manifests include `remote_desktop_provider` entries.

### Task 8: End-To-End Verification

**Files:**
- Main repo and `../onetcli-extensions`

- [ ] **Step 1: Rust unit tests**

Run:

```bash
cargo test -p remote_desktop -- --nocapture
cargo test -p extension-runtime -- --nocapture
cargo test -p main -- --nocapture
```

Expected: relevant package tests pass.

- [ ] **Step 2: Workspace check**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p remote_desktop -p remote_desktop_view -p extension-runtime -p main
```

Expected: all checked crates compile.

- [ ] **Step 3: Extension repo checks**

Run in `../onetcli-extensions`:

```bash
cargo check
npm test
```

Expected: helper binaries and manifest/package scripts pass existing tests.

- [ ] **Step 4: Manual behavior check**

With no remote desktop providers installed:
- New connection window still shows built-in RDP and VNC entries.
- Users can create and save new RDP/VNC connections.
- Double-clicking an existing or newly created RDP/VNC connection prompts to download the matching provider.

After installing provider packages:
- New connection window continues to show built-in RDP/VNC entries.
- Double-clicking existing connections opens the remote desktop tab.
