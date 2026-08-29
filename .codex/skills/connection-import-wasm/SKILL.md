---
name: connection-import-wasm
description: Use when implementing, debugging, packaging, or host-enabling onetcli WASM connection importers such as DBeaver, Navicat, Navicat Lite, Termius, connection-import.wit components, wasm32-wasip2 importers, composite extension manifests, local importer visibility, or connection import UI freezes.
---

# Connection Import WASM

## Overview

Build one connection importer as one WASM component. Keep parser logic in the extension repo, keep host capabilities generic in `../onetcli`, and verify the whole path from WIT contract to local composite extension visibility.

Use DBeaver as the reference implementation, but avoid baking DBeaver-specific assumptions into the host.

## Reference Library

When developing an importer for a new application, read the playbook first, then load the topic references needed by the app's storage format and output kind:

| Need | Reference |
| --- | --- |
| End-to-end workflow for a new application | [New Application Playbook](references/new-application-playbook.md) |
| Finding app config files across macOS, Windows, and Linux | [Source Discovery](references/source-discovery.md) |
| Mapping parser output to host protocol JSON | [Protocol and Records](references/protocol-and-records.md) |
| Writing `extension.json`, root manifest entries, and local installs | [Manifest and Packaging](references/manifest-and-packaging.md) |
| Fixtures, tests, local host checks, and release verification | [Testing and Troubleshooting](references/testing-and-troubleshooting.md) |
| Database importer patterns such as DBeaver and Navicat Lite | [Database Importers](references/database-importers.md) |
| SSH importer patterns such as OpenSSH config and known_hosts | `ssh-connection-import` |

## Repo Map

| Area | Path |
| --- | --- |
| Extension workspace | `onetcli-extensions` |
| Importer crates | `extensions/wasm/<tool>-importer` |
| Shared extension WIT | `wit/connection-import.wit` |
| Marketplace entry | `manifest.json` with `"kind": "composite"` |
| Composite packaging | `scripts/package-composite-extension.sh`, `scripts/verify-composite-package.sh`, `scripts/release-driver.mjs` |
| Host repo | `../onetcli` |
| Host WIT | `crates/extension-api/wit/connection-import.wit` |
| Host WASM runtime | `crates/extension-wasm/src/connection_import.rs` |
| Host manifest/provider | `crates/extension-runtime/src/connection_import_provider.rs`, `crates/extension-runtime/src/extension/composite_provider.rs` |
| Import UI | `main/src/home/connection_import_*` |

## Implementation Workflow

1. Implement one tool per WASM component. Prefer `extensions/wasm/<tool>-importer`, with `src/component.rs` for WIT bindings and `src/<tool>.rs` for parser logic.
2. Keep `connection-import.wit` vendored in this repo under `wit/`. Do not make extension crates import the host repo WIT directly. Add or keep drift checks against `../onetcli` when the host repo is present.
3. Generate bindings from `../../../wit` in each importer crate:

```rust
wit_bindgen::generate!({
    path: "../../../wit",
    world: "connection-importer",
});
```

4. Export `descriptor`, `scan`, and `preview`. Return JSON matching `connection-import-protocol`. Use structured parsers such as `serde_json`, `plist`, or product-specific parsers; avoid ad hoc string slicing for config formats.
5. Gate secrets strictly. If `ImportOptions.include_passwords` is false, never return plaintext passwords even when they exist in config files.
6. Declare candidates and permissions in `extension.json`. Include all product paths per platform, for example macOS `~/...` and Windows `%APPDATA%/...`.
7. Register the importer in root `manifest.json` as a composite extension. Composite importers install under `~/.config/one-hub/extensions/composite/<extension-id>`.
8. Package and install locally before host debugging. The installed folder must contain `extension.json` and `wasm/<module>.wasm`.

## Host Capability Checklist

When an importer fails in the host, fix the host generically:

- WASI Preview2: `crates/extension-wasm/src/connection_import.rs` must add `wasmtime_wasi::p2::add_to_linker_async`, store `WasiCtx`, and implement `WasiView` with a `ResourceTable`. A `wasm32-wasip2` component commonly imports `wasi:io/poll@0.2.6`.
- Manifest permissions: `%APPDATA%/...` and `~/...` file permissions must validate in `extension/manifest/security_rules.rs`.
- Candidate path expansion: `connection_import_provider.rs` must expand `~/` and `%VAR%/...` before reading files.
- Local visibility: `CompositeExtensionProvider` uses `load_and_check`; host version checks must use the app version, not the `extension-runtime` crate version.
- UI responsiveness: never call `futures::executor::block_on` from GPUI import actions/dialogs. Open the preview dialog in a loading state and run preview work through `one_core::gpui_tokio::Tokio`.
- Feature wiring: `main` must enable `extension-runtime` with `features = ["wasm-components"]`, including `--no-default-features` builds when the app should still expose WASM importers.

## Manifest Pattern

Use a composite manifest shape like this:

```json
{
  "schema_version": 1,
  "id": "com.onetcli.importer.<tool>",
  "name": "<Tool> Importer",
  "version": "0.1.0",
  "engines": { "onetcli": ">=0.7.0" },
  "runtime": {
    "wasm": [{
      "id": "<tool>-importer",
      "module": "wasm/<tool>_importer_wasm.wasm",
      "kind": "component"
    }]
  },
  "permissions": [
    "fs:read:~/Library/...",
    "fs:read:%APPDATA%/..."
  ],
  "contributes": {
    "connectionImporters": [{
      "id": "<tool>",
      "runtimeId": "<tool>-importer",
      "displayName": "<Tool>",
      "outputKinds": ["database"],
      "platforms": ["macos", "windows"],
      "candidateFiles": [{
        "id": "<tool>-macos-config",
        "platform": "macos",
        "path": "~/Library/..."
      }]
    }]
  }
}
```

## Database Importer Notes

For database tools, fixtures from one product edition are not enough. Check every edition and platform path the importer claims in `extension.json`.

Navicat-specific rules:

- Premium Lite stores shared connection data at macOS `~/Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist`.
- The Windows analogue is expected under `%APPDATA%/PremiumSoft CyberTech/Navicat CC/Common/conn.plist`; declare it separately from the classic Navicat paths.
- Navicat plist files may be XML or binary. Use `plist::Value::from_reader(Cursor::new(bytes))` when binary support matters, not XML-only parsing.
- Lite records may use lowercase fields such as `host`, `port`, `username`, and `defaultdatabase`.
- `serviceprovider = Default` is not a database type. Infer the database type from stable path/key segments such as `MySQL`, `PostgreSQL`, `Oracle`, or SQL Server-specific containers.
- Skip nested parameter dictionaries that are not database connections, including `ssh_param`, `http_param`, `ssl_param`, and `compatibility_param`.

Add focused fixtures for each schema variant. At minimum, cover classic Navicat entries and Lite `conn.plist` entries with lowercase fields and path-derived database types.

For SSH-focused importers, use `ssh-connection-import`. Keep this skill for shared WASM packaging, manifests, host capability, and database importer behavior.

## Testing

Extension repo:

```bash
rtk cargo test -p <tool>_importer_wasm
rtk cargo fmt --all --check
rtk cargo build --release -p <tool>_importer_wasm --target wasm32-wasip2
rtk node --test tests/scripts.test.mjs
rtk node scripts/release-driver.mjs <tool>-importer 0.1.0 --target universal --artifact-dir artifacts
```

Host repo:

```bash
rtk cargo test -p extension-wasm
rtk cargo test -p extension-runtime connection_import
rtk cargo test -p extension-runtime composite_provider_lists_connection_importer_with_windows_env_permission
rtk cargo check -p main
rtk cargo check -p main --no-default-features
rtk cargo build -p main
```

Add focused tests for every host ability that was missing. Useful examples include:

- A real or fixture `wasm32-wasip2` component that reproduces `wasi:io/poll@0.2.6` linker failures.
- A manifest with `%APPDATA%/...` permissions and candidate files.
- A provider/listing test proving the local composite importer appears in Installed extensions.
- A preview-provider test that runs DBeaver/Termius fixture components.
- A parser fixture for every product edition/path variant declared in the manifest.
- A serialized JSON assertion for any field backed by a host serde enum or externally tagged protocol shape.

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `component imports instance wasi:io/poll@0.2.6` | Connection-import runtime lacks WASI Preview2 linker | Add `wasmtime_wasi::p2::add_to_linker_async` plus `WasiView` state. |
| Local importer not visible | Manifest rejected by permissions or host version check | Validate `%APPDATA%` fs permissions and ensure app version is used for `engines.onetcli`. |
| Preview dialog freezes | UI thread is running WASM/filesystem work | Replace `block_on` with `Tokio::spawn` and loading-state entity updates. |
| Records import passwords when disabled | Parser ignores `include_passwords` | Ensure parser omits config and credential passwords unless enabled. |
| Component returns zero records | Candidate id, permission, or path mismatch | Check `candidateFiles`, `fs:read:*`, platform filtering, and host path expansion. |
| Navicat Lite returns zero records | Lite path/schema differs from classic Navicat | Add `Navicat CC/Common/conn.plist`, lowercase field handling, path-derived database type inference, and plist binary/XML parsing. |
| Source is visible but preview has no rows | Availability only proves manifest/candidate visibility | Check host logs for file read, WASM output, and `connection import preview failed` decode errors. |
| `unknown variant kind` during preview | Importer emitted an internally tagged enum shape rejected by `connection-import-protocol` | Match the host serde JSON shape exactly; for SSH auth use `ssh-connection-import`. |

## Guardrails

- Do not combine DBeaver, Navicat, and Termius into one component unless the product explicitly shares storage and parsing semantics. Independent tools should remain separate WASM importers.
- Do not special-case product ids in the host. Add generic manifest, permission, WIT, or runtime capability support.
- Do not let the extension repo depend on `../onetcli` at build time. Use vendored WIT and drift verification.
- Do not claim local installation works until the installed composite folder is present and the host provider can list it.
- Do not leave long-running dev app processes active after build/test work unless the user asked to run the app.
- Do not trust display labels such as `Default` as database types when the product stores type information in surrounding keys or path segments.
