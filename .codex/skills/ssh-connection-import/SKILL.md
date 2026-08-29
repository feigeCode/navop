---
name: ssh-connection-import
description: Use when implementing or debugging onetcli SSH connection importers, including OpenSSH config, known_hosts, candidate file manifests, local composite installs, SSH auth_method JSON decode errors such as unknown variant kind, or preview scans that show available sources but return zero records.
---

# SSH Connection Import

## Overview

Build and debug SSH importers against the host protocol first. Most failures are not parsing failures; they are mismatches between manifest visibility, candidate file access, WASM output JSON, and `connection-import-protocol` decode rules.

For shared WASM component structure, manifests, packaging, and host runtime work, also use `connection-import-wasm`.

## References

| Need | Reference |
| --- | --- |
| OpenSSH config and known_hosts parsing rules | [OpenSSH Sources](references/openssh-sources.md) |
| SSH protocol JSON, auth methods, and preview failures | [SSH Protocol Debugging](references/ssh-protocol-debugging.md) |

## Protocol Contract

Inspect the host protocol before defining extension structs:

- Host model: `../onetcli/crates/connection-import-protocol/src/model.rs`
- Importer output must serialize to the host's serde shape, not a local convenience shape.

For SSH auth, match `SshImportAuthMethod` with `#[serde(rename_all = "snake_case")]`:

| Variant | JSON shape |
| --- | --- |
| `AutoPublicKey` | `"auto_public_key"` |
| `Agent` | `"agent"` |
| `PrivateKey` | `{"private_key":{"key_path":"...","passphrase":null}}` |
| `Password` | `{"password":{"password":null}}` or an omitted secret when not importing passwords |

Never emit `{"kind":"auto_public_key"}`. The host error `unknown variant kind, expected one of password, private_key, private_key_material, agent, auto_public_key` means the importer used internally tagged JSON that the host protocol does not accept.

Add a focused serialization test for each auth variant the importer emits:

```rust
let value = serde_json::to_value(&record).unwrap();
assert_eq!(value["ssh"]["auth_method"], "auto_public_key");
```

## OpenSSH Sources

Declare platform candidates and `fs:read:*` permissions for both primary config and fallback discovery:

| Source | macOS/Linux | Windows |
| --- | --- | --- |
| OpenSSH config | `~/.ssh/config` | `%USERPROFILE%/.ssh/config` |
| known hosts fallback | `~/.ssh/known_hosts` | `%USERPROFILE%/.ssh/known_hosts` |

Use `known_hosts` only as host discovery. It rarely contains username or auth metadata, so emit host-only records with empty username, parsed port, and `AutoPublicKey`.

Parsing rules for `known_hosts`:

- Parse plain hosts, comma-separated aliases, and bracketed ports like `[example.com]:2222`.
- Skip hashed hosts starting with `|`, wildcard or negated patterns, and marker lines such as `@cert-authority`.
- Ignore key material after the host pattern column.
- See [OpenSSH Sources](references/openssh-sources.md) before extending parser behavior.

## Debug Flow

When the UI shows the source as available but OpenSSH returns no rows, trace the full path:

1. Confirm the installed composite manifest contributes the importer and the current platform.
2. Confirm each candidate file path expands correctly (`~/...`, `%USERPROFILE%/...`) and has matching `fs:read:*` permission.
3. Confirm the file exists on the user's machine. `~/.ssh/config` may be absent while `~/.ssh/known_hosts` exists.
4. Run importer unit tests for parser behavior and serialized JSON shape.
5. Check host logs for `connection import preview failed`. Decode errors mean the source was read and the WASM component returned JSON, but the host rejected the protocol shape.

Treat "可导入" or "available" in the UI as manifest/source visibility only. It does not prove preview records decoded successfully.

## Local Packaging

Verify and install the composite extension before host UI testing:

```bash
rtk cargo test -p openssh_config_importer_wasm
rtk cargo build --release -p openssh_config_importer_wasm --target wasm32-wasip2
rtk node scripts/release-driver.mjs openssh-config-importer 0.1.0 --target universal --artifact-dir /tmp/onetcli-composite-local/openssh-config
```

The local install target is:

```text
~/.config/one-hub/extensions/composite/com.onetcli.importer.openssh-config
```

The installed folder must contain `extension.json` and `wasm/openssh_config_importer_wasm.wasm`.

## Common Mistakes

| Mistake | Fix |
| --- | --- |
| Defining SSH auth as `{ kind, ... }` | Serialize exactly like `connection-import-protocol::SshImportAuthMethod`. |
| Only scanning `~/.ssh/config` | Add `known_hosts` fallback because many local machines have no config file. |
| Treating known_hosts aliases as full configs | Emit conservative host-only records and skip ambiguous patterns. |
| Debugging only extension code after a UI zero-row result | Inspect host decode logs and candidate file reads. |
