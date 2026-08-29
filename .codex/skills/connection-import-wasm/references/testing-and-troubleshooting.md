# Testing and Troubleshooting

Use this to verify a new importer and diagnose UI preview failures.

## Extension Tests

Run the focused package tests first:

```bash
rtk cargo test -p <tool>_importer_wasm
rtk cargo fmt --all --check
rtk cargo build --release -p <tool>_importer_wasm --target wasm32-wasip2
rtk node --test tests/scripts.test.mjs
```

Fixture coverage should include:

- One normal connection.
- Empty config.
- Corrupt or unsupported config.
- Each supported platform or product edition schema.
- Secret present with `include_passwords = false`.
- Secret present with `include_passwords = true` when supported.
- Optional nested blocks such as SSH, SSL, proxy, tunnel, or workspace metadata.

## Host Tests

When host behavior changes, test in `../onetcli`:

```bash
rtk cargo test -p extension-wasm
rtk cargo test -p extension-runtime connection_import
rtk cargo check -p main
rtk cargo check -p main --no-default-features
```

Add host tests for generic host capabilities only:

- WASI Preview2 linker support.
- `%APPDATA%`, `%LOCALAPPDATA%`, `%USERPROFILE%`, and `~/` path expansion.
- Manifest permission validation.
- Composite provider listing local importers.
- Preview protocol decode failures.

Do not add product-specific parser behavior to the host.

## Local UI Debug Sequence

If the UI shows the source but preview returns no records:

1. Confirm the installed composite folder exists.
2. Inspect installed `extension.json`, not only the source tree manifest.
3. Confirm the candidate file exists on the local machine.
4. Check whether `scan` found files or only reported source availability.
5. Run importer unit tests with the local file converted into a scrubbed fixture.
6. Check host logs for `connection import preview failed`.
7. If logs show a JSON decode error, inspect serialized `preview` output shape.

Visibility in the import UI is not proof that parsing or host decode succeeded.

## Common Failures

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `component imports instance wasi:io/poll@0.2.6` | Host WASM runtime lacks WASI Preview2 support | Add Preview2 linker and `WasiView` state in the host. |
| Importer visible but no rows | Candidate exists, but parser returned empty or host rejected JSON | Check parser fixture and host decode logs. |
| Local importer missing | Composite install path, engine version, permission validation, or root manifest issue | Inspect installed folder and host extension provider logs. |
| `unknown variant kind` | Serde enum JSON shape does not match host protocol | Serialize exactly like `connection-import-protocol`. |
| Passwords appear when disabled | Parser ignores `include_passwords` | Add tests for `include_passwords = false`. |
| Records are duplicated | Unstable ids or multiple candidate paths point to same source | Deduplicate by source id or endpoint key. |
| Windows path not scanned | Manifest uses macOS path or unsupported variable | Add `%APPDATA%`, `%LOCALAPPDATA%`, or `%USERPROFILE%` candidate and permission. |

## Logging

Use host logging for evidence that cannot be returned in preview rows:

- Candidate id being parsed.
- File read failure code.
- Unsupported format version.
- Number of records parsed before filtering.

Do not log secrets, full private keys, passwords, tokens, or decrypted blobs.

## Regression Rule

Every user-reported import failure should leave one durable regression:

- Parser fixture if the app schema was not handled.
- Serialization test if host JSON decode failed.
- Manifest/script test if packaging or candidate paths were wrong.
- Host runtime test if the generic extension runtime lacked a capability.
