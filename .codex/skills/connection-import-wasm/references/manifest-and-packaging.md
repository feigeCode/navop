# Manifest and Packaging

Use this when adding or installing a composite importer.

## Extension Manifest

Each importer extension needs an `extension.json` with:

- Stable extension id: `com.onetcli.importer.<tool>`.
- WASM runtime entry with `kind = "component"`.
- `fs:read:*` permissions for every candidate file or directory.
- `contributes.connectionImporters` entry wired to the runtime id.
- Platform-specific candidate files.

Minimal shape:

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
    "fs:read:~/Library/Application Support/<Tool>/config.json",
    "fs:read:%APPDATA%/<Tool>/config.json"
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
        "path": "~/Library/Application Support/<Tool>/config.json"
      }]
    }]
  }
}
```

## Root Marketplace Manifest

Add the composite extension to the repo root `manifest.json` so packaging scripts and local installs can discover it. Keep ids and versions synchronized with `extension.json`.

## Permission Matching

The host validates permissions before reading candidate files. For every candidate:

```json
{"id":"tool-macos-config","path":"~/Library/Application Support/Tool/config.json"}
```

there must be a matching permission:

```json
"fs:read:~/Library/Application Support/Tool/config.json"
```

Directory candidates should grant the directory path, not an unrelated child path.

## Platform Paths

Use portable path variables in manifests:

- macOS/Linux home: `~/...`
- Windows roaming app data: `%APPDATA%/...`
- Windows local app data: `%LOCALAPPDATA%/...`
- Windows profile: `%USERPROFILE%/...`

Do not hard-code `/Users/<name>` or `C:\Users\<name>`.

## Local Package Commands

Build and package:

```bash
rtk cargo build --release -p <tool>_importer_wasm --target wasm32-wasip2
rtk node scripts/release-driver.mjs <tool>-importer 0.1.0 --target universal --artifact-dir /tmp/onetcli-composite-local/<tool>
```

Expected install location:

```text
~/.config/one-hub/extensions/composite/com.onetcli.importer.<tool>
```

Expected files:

```text
extension.json
wasm/<tool>_importer_wasm.wasm
```

## Packaging Tests

Run:

```bash
rtk node --test tests/scripts.test.mjs
```

Add tests when a packaging rule changes:

- Root manifest entry is included.
- Composite manifest has required runtime module.
- Candidate files and permissions are copied intact.
- Platform-specific paths remain unmodified.

## Common Manifest Failures

| Symptom | Check |
| --- | --- |
| Importer not visible | Root `manifest.json`, local install path, `engines.onetcli`, manifest schema. |
| Candidate cannot be read | Missing `fs:read:*` permission or unsupported path variable. |
| Runtime load fails | `runtimeId` mismatch, wrong WASM module filename, or component not built for `wasm32-wasip2`. |
| Visible on wrong OS | Missing or incorrect `platforms` and `candidateFiles.platform`. |
