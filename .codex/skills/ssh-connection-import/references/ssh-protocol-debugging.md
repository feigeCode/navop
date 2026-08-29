# SSH Protocol Debugging

Use this when preview logs show decode failures or OpenSSH import appears in the UI but returns no rows.

## Auth Method JSON

The host Rust protocol expects serde enum JSON, not the WIT record shape.

Valid examples:

```json
"auto_public_key"
```

```json
"agent"
```

```json
{"private_key":{"key_path":"~/.ssh/id_ed25519","passphrase":null}}
```

```json
{"private_key_material":{"private_key":null,"passphrase":null,"file_name_hint":"id_ed25519"}}
```

```json
{"password":{"password":null}}
```

Invalid for host preview decode:

```json
{"kind":"auto_public_key"}
```

The error:

```text
unknown variant `kind`, expected one of `password`, `private_key`, `private_key_material`, `agent`, `auto_public_key`
```

means the importer returned internally tagged JSON. Fix importer structs or custom serialization, then add a `serde_json::to_value` assertion.

## Serialization Test Pattern

Test JSON shape directly:

```rust
let record = /* build preview record */;
let json = serde_json::to_value(&record).unwrap();
assert_eq!(json["ssh"]["auth_method"], "auto_public_key");
```

For private keys:

```rust
assert_eq!(
    json["ssh"]["auth_method"],
    serde_json::json!({"private_key":{"key_path":"~/.ssh/id_ed25519","passphrase":null}})
);
```

## Preview Failure Trace

When preview fails:

1. Confirm `scan` discovered candidate files.
2. Confirm the WASM component read the file through `read-file`, not a direct local path.
3. Add or inspect parser tests for the source file.
4. Serialize the first record to JSON in a unit test.
5. Compare enum shapes with `../onetcli/crates/connection-import-protocol/src/model.rs`.
6. Repackage and reinstall the composite extension.
7. Check host logs again.

Do not spend time changing candidate paths if the log says protocol decode failed; at that point the source file was already found and the component returned JSON.

## Local Reinstall Checklist

Run:

```bash
rtk cargo test -p openssh_config_importer_wasm
rtk cargo build --release -p openssh_config_importer_wasm --target wasm32-wasip2
rtk node scripts/release-driver.mjs openssh-config-importer 0.1.0 --target universal --artifact-dir /tmp/onetcli-composite-local/openssh-config
```

Verify:

```text
~/.config/one-hub/extensions/composite/com.onetcli.importer.openssh-config/extension.json
~/.config/one-hub/extensions/composite/com.onetcli.importer.openssh-config/wasm/openssh_config_importer_wasm.wasm
```

If the UI still shows old behavior, confirm the app loaded this installed path and not a stale package.
