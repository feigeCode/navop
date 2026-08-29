# New Application Playbook

Use this when adding an importer for an application that onetcli does not support yet. The goal is to leave a future agent with enough evidence to implement the next importer without rediscovering the framework.

## Definition of Done

An application importer is ready when all of these are true:

- The extension repo contains one focused WASM component for the application.
- The parser handles each supported product edition and platform path declared in `extension.json`.
- `descriptor`, `scan`, and `preview` return JSON that the host `connection-import-protocol` accepts.
- Unit tests cover fixtures, edge cases, and serialized protocol shapes.
- The component builds for `wasm32-wasip2`.
- The composite package installs under `~/.config/one-hub/extensions/composite/<extension-id>` and the host can list and preview it.
- User secrets are gated by `ImportOptions.include_passwords` and are never returned by accident.

## Phase 1: Discover the Application Storage

Collect facts before writing parser code:

- Product name, vendor, edition names, and versions.
- Output kind: `database`, `ssh`, `port-forwarding`, or a combination.
- Platform paths for macOS, Windows, and Linux. Include edition-specific variants such as Lite, Store, Team, portable, or cloud-profile editions.
- Storage format: JSON, XML, plist, SQLite, INI, SSH-style config, directory tree, encrypted blob, keychain reference, or mixed.
- Stable record identity fields: connection UUID, profile name, path segment, hostname, or project id.
- Secret storage: inline plaintext, OS keychain, encrypted local database, unsupported, or absent.
- Representative fixtures with secrets scrubbed but structure preserved.

Prefer structured parsers over string slicing:

- JSON: `serde_json`
- XML: a real XML parser
- plist: `plist::Value::from_reader(Cursor::new(bytes))` for XML and binary plist
- SQLite: a SQLite reader or fixture abstraction
- SSH config: directive parser with quoting and comments handled deliberately

## Phase 2: Design the Importer Contract

Choose stable ids before implementation:

| Item | Convention |
| --- | --- |
| Extension id | `com.onetcli.importer.<tool>` |
| Runtime id | `<tool>-importer` |
| Crate package | `<tool>_importer_wasm` |
| Crate folder | `extensions/wasm/<tool>-importer` |
| Importer id | `<tool>` or `<tool>-<variant>` when variants cannot share parser semantics |
| Candidate id | `<tool>-<platform>-<source>` |

Map each source field to the host protocol:

- Use `source_label` for the human-facing application/source name.
- Use `source_id` for the original profile id, path, or composite key when available.
- Use deterministic `id` values. Do not use array indexes unless there is no stable identity.
- Put unsupported or ambiguous data in `warnings`, not in invented fields.
- Preserve host/port/user/database separately instead of embedding them in `display_name`.

## Phase 3: Implement the Component

Start from an existing importer with the closest storage shape:

- `dbeaver-importer` for XML/workspace-oriented database profiles.
- `navicat-importer` for plist-based database profiles and product edition variants.
- `openssh-config-importer` for SSH config and conservative host discovery.
- `tableplus-importer`, `mongodb-compass-importer`, `redis-desktop-importer`, or `jetbrains-importer` for smaller JSON/config examples.

Keep files split by responsibility:

- `src/component.rs`: WIT binding, host calls, JSON serialization wrappers.
- `src/<tool>.rs`: parsing, mapping, warnings, fixtures, tests.
- `extension.json`: component runtime, permissions, contributed importer metadata, candidate files.

Export behavior:

- `descriptor`: return a stable importer descriptor matching the manifest contribution.
- `scan`: inspect candidate files through host functions and return availability plus discovered files. Do not parse everything if a cheap existence check is enough.
- `preview`: read candidate files, parse records, apply `include_passwords`, and return import records.

## Phase 4: Add Tests First

Use fixtures from discovery. Add tests in this order:

1. One failing fixture test for the simplest real connection record.
2. Edge tests for edition-specific paths or schema variants.
3. Secret gating tests for `include_passwords = false`.
4. Serialized JSON shape tests for enum-like protocol fields.
5. Invalid/corrupt file tests that return zero records or warnings without panics.

For bugs found in the host UI, add a regression test at the lowest layer that reproduces the failure: parser, JSON serialization, packaging script, host manifest load, or host preview decode.

## Phase 5: Package and Install Locally

Use the standard release driver:

```bash
rtk cargo fmt --all --check
rtk cargo test -p <tool>_importer_wasm
rtk cargo build --release -p <tool>_importer_wasm --target wasm32-wasip2
rtk node --test tests/scripts.test.mjs
rtk node scripts/release-driver.mjs <tool>-importer 0.1.0 --target universal --artifact-dir /tmp/onetcli-composite-local/<tool>
```

Then verify the installed folder contains:

```text
~/.config/one-hub/extensions/composite/com.onetcli.importer.<tool>/extension.json
~/.config/one-hub/extensions/composite/com.onetcli.importer.<tool>/wasm/<module>.wasm
```

## Phase 6: Debug Host Preview

If the importer is visible but previews zero rows, trace in this order:

1. Root `manifest.json` includes the composite extension.
2. Installed `extension.json` has the expected `contributes.connectionImporters` entry.
3. Platform filters match the current host platform.
4. `candidateFiles` ids match `fs:read:*` permissions.
5. Host path expansion supports the path syntax used by the manifest.
6. The WASM component can read the candidate file through host functions.
7. The parser emits records.
8. The emitted JSON decodes into `connection-import-protocol`.

Host logs containing `connection import preview failed` mean the component ran far enough to return output; inspect the protocol shape before changing file discovery code.

## Hand-off Notes

When finishing an importer, leave comments or tests documenting every app-specific fact that was not obvious:

- Why a platform path exists.
- Which product edition introduced a different schema.
- Why a field is ignored.
- Whether secrets are unsupported by design or blocked by missing host capability.
- Which user-reported log line or fixture the test protects.
