---
name: ipc-driver-development
description: Use when designing, implementing, reviewing, or debugging onetcli IPC database drivers, driver.json manifests, extension-protocol wire methods, schema metadata, DDL builders, import/export, packaging, or cross-language driver runtimes.
---

# IPC Driver Development

## Overview

Build onetcli IPC database drivers from the host contract outward: `driver.json` declares the surface, `extension-protocol` defines wire JSON, and the driver routes each declared method to real database behavior.

Keep the driver language-agnostic unless the repository already provides a stronger local pattern. Rust can use `extension-driver`; other languages must still implement the same JSON-RPC methods, response shapes, and packaging contract.

## Workflow

1. Inspect the existing driver and host contract before editing. In this repo, use `extensions/ipc/duckdb` as the concrete reference; in the host repo, use `crates/extension-protocol` and `crates/db/src/ipc`.
2. Design or update `driver.json` first. Declare only methods that the binary actually handles, set dialect/capabilities so the host knows which generic features are safe to expose, and set top-level category metadata when the driver belongs in a non-default UI group.
3. Split routing into control-plane and connection-scoped methods. Keep `init`, `conn/test`, `conn/open`, and pure DDL builders independent of a live connection when possible; route query, metadata, transaction, import/export, and cursor methods through the opened connection.
4. Implement metadata from database catalog truth, not UI labels, connection names, or hardcoded defaults. If the database has a current catalog/database function, query it and use that value consistently.
5. Prefer explicit `NotSupported` / method-not-found behavior for missing methods. This lets the host use fallback behavior such as `dialect.compatible_database_type`.
6. For special connection lifecycle needs, declare them in `driver.json.connection` before changing host manager logic. The host should consume plugin lifecycle metadata, not hardcode driver ids.
7. For UI grouping such as domestic databases, declare top-level manifest metadata such as `"category": "domestic_database"`. Do not make the host classify external drivers by concrete ids like `dm`, `kingbase`, or `gbase8s`; unclassified external drivers must remain in the normal database group.
8. Add protocol-level tests before broad packaging work. Verify request JSON, response JSON, error codes, cancellation, fallback behavior, and lifecycle policy parsing.
9. Build and package with the repository scripts, then verify the archive contains `driver.json`, binary, locales, and expected entry command.

## Reference Routing

Load only the relevant reference file:

| Task | Read |
| --- | --- |
| Wire method names, params, response shapes, routing | `references/protocol.md` |
| `driver.json`, capabilities, dialect SQL contract, connection lifecycle, compatible fallback | `references/manifest.md` |
| Rust, Node.js, Python, or other runtime implementation choices | `references/language-patterns.md` |
| Tests, build, package, release checks | `references/testing-packaging.md` |

## Required Guardrails

- Treat `driver.json.methods` as a contract. If a method is listed, the binary must route it or intentionally return a typed unsupported error.
- Treat `schema/object_view` as the custom object-panel table contract. Declare it only when the driver can return `{title, columns, rows}` for the views it owns; return typed not-supported/method-not-found for unsupported views so the host can fall back to legacy metadata rendering.
- Treat table/column comment support as three independent checkpoints, required in **every** IPC driver that declares relational-table schema, not just the one currently under test: `schema/objects` returns `schema` + `comment`, `schema/columns` returns `comment`, and DDL builders emit `COMMENT ON TABLE` / `COMMENT ON COLUMN` for both create and alter (including newly-added columns, with rollback, `IS ''` clearing, and single-quote escaping). Missing any one silently breaks designer comment echo or produces "No changes detected" when only a comment changes. Do not hardcode comment behavior in the host for one driver — the host only consumes driver output. See `references/protocol.md` § Table / Column Comment Support and § Every-Driver Audit.
- Treat `driver.json.dialect` as the host-side SQL generation contract. It controls external plugin identifier quoting, pagination, boolean literals, explain fallback, and compatible DDL fallback.
- Treat `driver.json.connection` as the host-side physical connection lifecycle contract. Single-file/single-connection engines must announce that policy there; do not special-case names such as `duckdb` in `ConnectionManager` or IPC connection code.
- Treat top-level `driver.json.category` as the host-side external driver grouping contract. Use `"domestic_database"` for国产数据库 drivers that should appear in the 国产数据库 sidebar group; omit it for ordinary external database drivers.
- Keep catalog/schema semantics database-specific. For default catalogs, treat host compatibility aliases such as `main` only as filters; do not return fixed strings when the database exposes current catalog metadata.
- Include system schemas such as `information_schema` and `pg_catalog` when the target database exposes them and the host asks for schema objects. Filter only for a clear product reason.
- DDL builder methods produce SQL text only. They must not execute DDL.
- `ddl/build*` may be called as connectionless pure methods or routed through a connection with an injected `conn_id`; accept both when the builder does not require live state.
- Use structured JSON serializers/parsers for params and results. Avoid stringly-typed JSON construction except in narrow tests.
- For compatible syntax fallback, set `dialect.compatible_database_type` to an existing host `DatabaseType` only when the driver's SQL semantics are close enough for host-generated DDL.

## Common Mistakes

| Mistake | Correction |
| --- | --- |
| Hardcoding a default catalog such as `main` | Query current database/catalog metadata and map legacy aliases only during filtering. |
| Declaring future methods in `driver.json` | Add methods only after routing and tests exist, or the host will call broken paths. |
| Hardcoding single-file locking behavior in the host for one driver id | Declare `connection.single_file`, `single_connection`, `close_on_release`, and `path_fields` in the manifest so the plugin reports lifecycle policy. |
| Hardcoding 国产数据库 ids in the host UI | Declare `"category": "domestic_database"` at the top level of each matching external driver manifest. |
| Returning table names without schema/catalog context | Preserve database, schema, and object names as distinct fields. |
| `schema/objects` returns `name`/`kind`/`comment` but omits `schema` | Host can only fall back to table-name matching; table comment echo breaks or misattributes when tables share names across schemas. Return the real schema for every object. |
| `schema/columns` drops the comment column | Column comments never echo in the table designer. Map the catalog's comment/remarks value into `ColumnInfo.comment`. |
| DDL builder generates comments only in create_table, or gates them behind a never-enabled capability switch | Editing only a comment yields no change statements ("No changes detected", empty SQL preview). Add a comment diff to `ddl/build_alter_table` and enable the driver's comment capability. |
| DDL alter comments skip newly-added columns | A column added via `to_spec` (absent from `from_spec`) with a non-empty comment still needs a `COMMENT ON COLUMN` after `ALTER TABLE ... ADD`; skipping `fromColumn == null` loses the comment in preview. |
| Fixing comment/schema only in one driver or only in the host | The same issue reappears for every other database. Implement all three checkpoints per-driver; host `stringCell(cols, 3)`/`stringCell(cols, 5)` compatibility is only a fallback, not the implementation. |
| Metadata SQL references a catalog column that is not in the query context | Referencing a column from a catalog table that the query never joins (e.g. `tabid` without the join) fails with `字段 (tabid) 不在查询的任何表中`. Qualify every referenced column to an aliased FROM/JOIN target. |
| Extending protocol structs without updating explicit Rust struct literals | After adding a field such as `ObjectInfo.schema`, every Rust driver that constructs `ObjectInfo { ... }` explicitly must add it; the protocol update will otherwise break all Rust drivers at compile time. |
| Declaring `schema/object_view` without dispatch/tests | Add a connection-scoped route, init/manifest declaration, and tests for at least one rendered view shape. |
| Treating `information_schema` as a normal user schema bug | Include it when the backend supports it; qualify queries correctly. |
| Mixing SQL generation with execution | Keep DDL builders pure and let the host decide preview/execution timing. |
| Assuming Rust-only implementation | Reuse Rust SDKs when in Rust; otherwise implement the same JSON-RPC contract in the chosen language. |

## Minimal Verification

Before claiming an IPC driver change is complete, run the narrowest checks that prove it:

```bash
cargo test -p <driver_package> -- --nocapture
node --test tests/scripts.test.mjs
cargo fmt --all --check
```

For package changes:

```bash
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --release -p <driver_package> --target "$HOST_TRIPLE"
bash scripts/package-driver.sh <driver-id> "$HOST_TRIPLE" artifacts <version>
bash scripts/verify-package.sh "artifacts/<driver-id>-driver-${HOST_TRIPLE}.tar.gz"
```
