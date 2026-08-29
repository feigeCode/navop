# Testing and Packaging Reference

## Table of Contents

- [Local Checks](#local-checks)
- [Driver Tests](#driver-tests)
- [Manifest Tests](#manifest-tests)
- [Packaging](#packaging)
- [Release Metadata](#release-metadata)

## Local Checks

Run the smallest command that proves the changed surface.

For the existing Rust DuckDB driver:

```bash
cargo test -p duckdb_driver -- --nocapture
cargo fmt --all --check
```

For repository packaging scripts:

```bash
node --test tests/scripts.test.mjs
```

For GitHub workflow syntax:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/release.yml"); YAML.load_file(".github/workflows/upload-r2.yml"); puts "workflow yaml ok"'
```

## Driver Tests

Test at three levels:

1. Pure builder/metadata functions.
2. Handler functions with JSON params and JSON result assertions.
3. Runtime/host integration if routing, `conn_id`, cancellation, or stream behavior matters.

Minimum behavior tests for a new database driver:

- `conn/test` succeeds and reports backend errors usefully.
- `conn/open` rejects mismatched `driver_id`.
- `query/start` plus `cursor/fetch` returns stable column metadata and rows.
- `cursor/close` releases resources.
- `schema/databases` returns the real current/default catalog where applicable.
- `schema/schemas` includes system schemas when the backend exposes them.
- `schema/objects` and `schema/columns` qualify objects correctly.
- Any declared `ddl/build*` method returns expected SQL and never executes it.
- Declared unsupported features return typed unsupported errors.

Regression tests for common metadata bugs:

- `information_schema.character_sets` or equivalent system-table data can be opened with the returned schema/catalog.
- Filtering treats legacy default catalog aliases and the real current catalog as equivalent only where intended.
- Object lists include views, indexes, checks, and functions when their methods/capabilities are declared.

## Manifest Tests

Validate:

- `driver.json` parses as `IpcDriverManifest`.
- Every method is known or uses `x/...`.
- `entry.command` is non-empty and points to the packaged binary/launcher.
- `transport.name` is stable and unique per driver.
- `ui.locales_dir` exists when declared.
- `dialect.identifier_quote_left` and `identifier_quote_right` parse and produce the intended escaped identifier strings.
- `dialect.limit_style` parses and produces the intended pagination syntax.
- `dialect.bool_true` and `dialect.bool_false` produce the intended row-edit boolean literals.
- `dialect.explain_template` parses and produces the intended direct explain or fallback explain SQL.
- `dialect.compatible_database_type` parses when used.
- Capabilities match declared metadata methods.

Keep a test that loads the real `driver.json`, not only a copied fixture.

## Packaging

For current host target:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p <driver_package> --target "$HOST_TRIPLE"
mkdir -p artifacts
bash scripts/package-driver.sh <driver-id> "$HOST_TRIPLE" artifacts <version>
bash scripts/verify-package.sh "artifacts/<driver-id>-driver-${HOST_TRIPLE}.tar.gz"
```

Expected archive shape:

```text
<driver-id>/
  driver.json
  <entry binary or launcher>
  locales/
```

Windows entry binaries usually need `.exe`; keep `package-driver.sh` and `driver.json.entry.command` aligned.

## Release Metadata

Each driver needs an `extension.build.json` entry shape similar to:

```json
{
  "id": "postgres",
  "kind": "database_driver",
  "package": "postgres_driver",
  "binary": "postgres_driver",
  "path": "extensions/ipc/postgres",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "postgres-v",
  "r2Prefix": "extensions/postgres"
}
```

Release workflow inputs:

- extension id
- version
- target package files
- checksums
- `manifest/entries/*.json`

Do not make the host app release depend on extension builds. The extension repo produces packages and marketplace metadata; the host consumes published manifests and archives.
