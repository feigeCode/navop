# Protocol and Records

Use this before designing importer structs. The extension WIT exposes host functions, but preview output is a JSON string decoded by the host protocol. Match the host protocol exactly.

## Canonical Sources

Check these before adding fields:

```text
wit/connection-import.wit
../onetcli/crates/connection-import-protocol/src/model.rs
../onetcli/crates/extension-runtime/src/connection_import_provider.rs
```

If WIT and the Rust protocol appear to disagree, treat the Rust `connection-import-protocol` decode model as authoritative for JSON returned by `preview`.

## Top-Level Import Record

Every preview record needs the shared top-level fields:

| Field | Guidance |
| --- | --- |
| `id` | Deterministic, importer-scoped, stable across scans. |
| `importer_id` | Matches the contributed importer id. |
| `source_label` | Human-facing source name, such as `Navicat` or `OpenSSH Config`. |
| `source_id` | Original app id, path, or profile key when available. |
| `kind` | `database`, `ssh`, or `port_forwarding` in JSON. |
| `display_name` | Name shown in preview rows. |
| `database` | Present only for database records. |
| `ssh` | Present only for SSH records. |
| `port_forwarding` | Present only for port-forwarding records. |
| `password_status` | `included`, `missing`, `unsupported`, or `permission_denied`. |
| `warnings` | Non-fatal data loss, unsupported fields, or skipped secret details. |

Do not add arbitrary fields to the JSON and expect the host to preserve them.

## Database Records

The WIT database model is:

```text
database_type: mysql | postgresql | sqlite | duckdb | sqlserver | oracle | clickhouse | external
external_database_id: optional string
name: string
host: string
port: optional u16
username: string
password: optional string
database: optional string
extra_params: list of key/value pairs
```

Mapping guidance:

- Use `external` only when the host has no native type for the application database kind.
- Use `external_database_id` for source-specific engines such as Redis or MongoDB if the protocol has not grown a native enum yet.
- Keep SQLite/DuckDB file paths in `host` or `database` consistently with the existing importer pattern.
- Put query parameters or unsupported source fields in `extra_params` only when the host can use or display them safely.
- If `include_passwords` is false, set `password` to null and `password_status` according to whether a password exists but was withheld, is missing, or is unsupported.

## SSH Records

For SSH details, use `ssh-connection-import`. The protocol pitfall is auth method JSON shape:

```json
"auto_public_key"
```

or:

```json
{"private_key":{"key_path":"~/.ssh/id_ed25519","passphrase":null}}
```

Do not emit:

```json
{"kind":"auto_public_key"}
```

That produces host decode errors such as `unknown variant kind`.

## Secrets

Secrets must follow `ImportOptions.include_passwords`:

- `false`: never return plaintext passwords, private key material, passphrases, or decrypted tokens.
- `true`: return secrets only when the application stores them in a supported and user-approved way.
- Unsupported encryption should produce `password_status = "unsupported"` plus a warning if useful.
- Missing values should produce `missing`, not `unsupported`.

If the importer must query OS secrets, use the host `read-secret` function and handle `missing`, `permission-denied`, and `unsupported` explicitly.

## Serialization Tests

Add tests that inspect JSON, not only Rust structs:

```rust
let json = serde_json::to_value(&record).unwrap();
assert_eq!(json["kind"], "database");
assert_eq!(json["password_status"], "unsupported");
```

For enum-like fields, test the exact JSON shape. This catches serde tagging mistakes before the host UI sees them.

## Warnings

Use warnings for data that was skipped but does not invalidate the record:

- Unsupported proxy or tunnel settings.
- Encrypted passwords not imported.
- Ambiguous database type inferred from a path.
- Malformed optional blocks ignored.

Use stable warning codes so tests can assert them.
