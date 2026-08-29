# driver.json Manifest Reference

## Table of Contents

- [Required Shape](#required-shape)
- [Methods](#methods)
- [Dialect](#dialect)
- [Host SQL Generation](#host-sql-generation)
- [Connection Lifecycle](#connection-lifecycle)
- [Capabilities](#capabilities)
- [Category](#category)
- [UI Form](#ui-form)
- [Compatibility Fallback](#compatibility-fallback)

## Required Shape

`driver.json` is the host's first contract with an IPC database driver.

Minimal shape:

```json
{
  "id": "postgres-compatible",
  "name": "Postgres Compatible",
  "description": "Example IPC database driver",
  "category": "domestic_database",
  "version": "1.0.0",
  "entry": {
    "command": "./postgres_driver",
    "args": [],
    "working_dir": null
  },
  "transport": {
    "name": "postgres-compatible.sock",
    "connect_timeout_ms": 5000
  },
  "methods": [
    "$/ping",
    "shutdown",
    "conn/test",
    "conn/open",
    "conn/close",
    "query/start",
    "cursor/fetch",
    "cursor/close",
    "schema/object_view",
    "schema/databases",
    "schema/schemas",
    "schema/objects",
    "schema/columns"
  ],
  "dialect": {
    "identifier_quote_left": "\"",
    "identifier_quote_right": "\"",
    "compatible_database_type": "PostgreSQL"
  },
  "connection": {
    "single_file": false,
    "single_connection": false,
    "close_on_release": false,
    "path_fields": []
  },
  "capabilities": {
    "supports_schema": true,
    "uses_schema_as_database": false
  },
  "ui": {
    "icon": "Database",
    "locales_dir": "locales",
    "default_port": 5432,
    "show_in_new_connection": true
  }
}
```

The host deserializes this into `IpcDriverManifest` in `crates/db/src/ipc/registry.rs`.

`category` is optional. Omit it for normal external database drivers; only set it when the host should route the driver to a specific non-default group.

## Methods

Declare only methods the driver handles.

Rules:

- Standard method names must be known by `extension_protocol::method::ALL_METHODS`.
- Private extension methods must use the `x/...` namespace.
- A declared method can still return a typed unsupported error for a database-specific limitation, but it should not be missing from the dispatcher by accident.
- `schema/object_view` is optional. Add it only after the runtime can return the custom table shape or typed not-supported for unsupported views.
- Keep `driver.json.methods`, dispatcher match arms, and tests in sync.

Common starter set:

```json
[
  "$/ping",
  "shutdown",
  "conn/test",
  "conn/open",
  "conn/close",
  "conn/ping",
  "query/start",
  "cursor/fetch",
  "cursor/close",
  "exec/run",
  "schema/object_view",
  "schema/databases",
  "schema/schemas",
  "schema/objects",
  "schema/columns"
]
```

Add `ddl/build*`, transactions, import/export, functions, indexes, checks, and views only when implemented.

## Dialect

`dialect` is not cosmetic metadata. The host external database plugin reads it when it has to generate SQL itself, including table data pagination, row editing predicates, database/table/column/index SQL fragments, explain fallback, and DDL fallback.

Host source of truth:

- `crates/db/src/ipc/registry.rs`: `IpcDriverDialect`
- `crates/db/src/ipc/plugin.rs`: `ExternalDatabasePlugin`
- `crates/db/src/ipc/registry/tests.rs`: manifest parsing expectations
- `crates/db/src/ipc/plugin.rs` tests: generated SQL behavior

Supported dialect fields:

| Field | Meaning |
| --- | --- |
| `identifier_quote_left` | Left quote string, default is `"`. |
| `identifier_quote_right` | Optional right quote string. If omitted and left is `[`, host uses `]`; otherwise right defaults to left. |
| `limit_style` | Pagination style. Values are `limit_offset` (default) and `offset_fetch`. |
| `bool_true` / `bool_false` | Boolean literal strings used by host row editing SQL. Defaults are `TRUE` and `FALSE`. |
| `explain_template` | Explain SQL template. Default behavior is `EXPLAIN {sql}`. Empty/whitespace disables template output. |
| `table_reference_schema_mode` | Controls host-generated table qualification. `auto` follows capabilities; `prefer_schema` uses the request schema as the qualifier when present. |
| `row_id_column` | Optional physical row identifier expression included by host-generated table-data SQL. |
| `row_id_alias` | Optional alias for `row_id_column`; defaults to `__rowid__` when `row_id_column` is set. |
| `default_order_by` | Optional default `ORDER BY` expression for host-generated paginated table-data SQL when the request has no sort. |
| `compatible_database_type` | Existing host built-in database type used for fallback SQL builders. |
| `supports_schema` | Legacy dialect capability; prefer top-level capabilities when available. |
| `supports_sequences` | Legacy dialect capability. |
| `uses_schema_as_database` | Legacy dialect capability. |

Examples:

```json
{
  "dialect": {
    "identifier_quote_left": "`",
    "identifier_quote_right": "`",
    "limit_style": "limit_offset",
    "bool_true": "TRUE",
    "bool_false": "FALSE"
  }
}
```

```json
{
  "dialect": {
    "identifier_quote_left": "[",
    "identifier_quote_right": "]",
    "limit_style": "offset_fetch",
    "bool_true": "1",
    "bool_false": "0",
    "explain_template": "EXPLAIN QUERY PLAN {sql}"
  }
}
```

Use `identifier_quote_left/right` when a database uses bracket or paired quoting. Do not make the driver hand-roll host SQL quoting if the dialect field can express it.

Identifier quote rules:

- Set both quote fields for symmetric quoting, for example `` ` `` / `` ` `` or `"` / `"`.
- If `identifier_quote_left` is `[` and `identifier_quote_right` is omitted, host treats the pair as `[` / `]`.
- If right quote is non-empty, host escapes embedded right quote by doubling it: `has]quote` -> `[has]]quote]`.
- If both effective quote strings are empty, host returns the identifier unchanged.

## Host SQL Generation

The external plugin uses dialect fields in these places:

| Host behavior | Dialect fields |
| --- | --- |
| `quote_identifier` for host-generated database, table, column, index, and DDL fallback fragments | `identifier_quote_left`, `identifier_quote_right` |
| Pagination for table data queries | `limit_style` |
| Boolean values in host row editing SQL | `bool_true`, `bool_false` |
| Explain command/fallback SQL | `explain_template` |
| Table reference qualification | `table_reference_schema_mode`, `supports_schema`, `uses_schema_as_database` |
| Table-data row identifiers and default ordering | `row_id_column`, `row_id_alias`, `default_order_by` |
| Built-in DDL fallback after external method returns NotSupported | `compatible_database_type` |

Pagination rules:

- `limit_offset` produces `LIMIT <limit> OFFSET <offset>`.
- `offset_fetch` produces `OFFSET <offset> ROWS FETCH NEXT <limit> ROWS ONLY`.
- For `offset_fetch`, if the caller has no `ORDER BY`, host injects `ORDER BY (SELECT NULL)` before `OFFSET` because many dialects require an order clause for this syntax.
- `build_limit_clause()` returns `LIMIT` for `limit_offset` and an empty string for `offset_fetch`.

Boolean literal rules:

- Host treats input values `1` and case-insensitive `true` as true.
- All other values are false.
- The emitted SQL literal is `bool_true` or `bool_false`.

Explain rules:

- `explain_template` is trimmed before use.
- If it contains `{sql}`, host replaces `{sql}` with the target SQL.
- If it does not contain `{sql}`, host appends a space and the target SQL.
- Empty template returns no direct explain statement.
- For external explain routing, host may wrap a `sql/explain` wire request and include the formatted template as `fallback_sql`.

## Connection Lifecycle

`connection` describes physical connection behavior the host cannot infer from generic SQL metadata. Use it for local embedded engines or file-backed drivers where concurrent physical opens can fail or leave file locks behind.

Host source of truth:

- `crates/db/src/ipc/registry.rs`: `IpcDriverConnection`
- `crates/db/src/ipc/plugin.rs`: `ExternalDatabasePlugin::connection_lifecycle`
- `crates/db/src/plugin.rs`: `ConnectionLifecycle`
- `crates/db/src/manager.rs`: session creation, physical open serialization, and close-on-release handling

Supported connection fields:

| Field | Meaning |
| --- | --- |
| `single_file` | The connection targets a local file path rather than a network/server endpoint. |
| `single_connection` | The engine cannot safely open multiple physical connections to the same file at the same time. |
| `close_on_release` | The host should close the physical connection when the session is released instead of keeping it idle in the pool. |
| `path_fields` | Ordered config fields used to derive the file lock key, for example `host`, `database`, or `extra_params.path`. |

Example for a single-file embedded database:

```json
{
  "connection": {
    "single_file": true,
    "single_connection": true,
    "close_on_release": true,
    "path_fields": ["host", "database", "extra_params.path"]
  }
}
```

Behavior:

- If `single_file` and `single_connection` are both true, the host derives a physical-open lock key from `path_fields` and serializes physical opens for the same resolved file.
- `close_on_release` tells the host to drop the physical connection when a UI/session operation ends. This prevents idle external driver processes from retaining file locks.
- `path_fields` must match the driver's own path resolution order in `conn/open`. Do not use catalog names, display names, or connection names unless the driver actually opens that value as the file path.
- A `file:` prefix is treated as a path prefix and should not create a distinct lock identity from the same bare path.
- The host manager should read this lifecycle policy from the plugin/manifest. Do not write special branches such as `if driver_id == "duckdb"` in host session or IPC connection code.

Use this block when:

- The database is embedded or local-file backed.
- The engine reports lock errors when two driver processes open the same file.
- The driver should not keep idle sessions alive because the database file must be reusable immediately.

Do not use this block for normal server databases. Server-backed drivers should usually keep the defaults so the host can pool connections normally.

## Capabilities

Capabilities drive UI visibility and optional metadata calls. Typical fields include:

- `supports_schema`
- `uses_schema_as_database`
- `supports_sequences`
- `supports_functions`
- `supports_procedures`
- `supports_triggers`
- `supports_table_engine`
- `supports_table_charset`
- `supports_table_collation`
- `supports_auto_increment`
- `supports_tablespace`
- `supports_unsigned`
- `supports_enum_values`
- `show_charset_in_column_detail`
- `show_collation_in_column_detail`
- `table_engines`

Keep capabilities honest. A capability set to true tells the host it can expose workflows and call related methods.

Note for the shared Go dbipc runtime (`internal/dbipc`): the Go `DriverSpec` has a `SupportsComments` bool that gates `COMMENT ON TABLE` / `COMMENT ON COLUMN` emission in `ddl.go`. When a Go driver's backend supports comments, set `SupportsComments: true` in `spec.go` **and** map the comment column in `schema/objects` / `schema/columns` handlers; the switch alone is not enough to make designer comment echo work.

## Category

`category` is top-level manifest metadata used by the host to group external database drivers in the new connection UI.

Supported values:

| Value | Meaning |
| --- | --- |
| `domestic_database` | Put the external driver under the 国产数据库 sidebar category. |

Rules:

- Declare `"category": "domestic_database"` for国产数据库 drivers such as DM, KingbaseES, and GBase 8s.
- Leave `category` absent for ordinary external database drivers so they remain in the normal 数据库 category.
- Do not put `category` under `ui`; host grouping reads the top-level manifest field.
- Do not hardcode concrete external driver ids in host UI grouping. Classification must come from manifest metadata.

## UI Form

The optional `ui.show_in_new_connection` flag controls whether the external driver is
listed as a card in the host's New Connection view. It defaults to `true` for backward
compatibility. Set it to `false` when the driver is exposed through a separate built-in
connection entry but must remain available to the registry, runtime, and MCP tools.

This flag only controls the New Connection UI. It does not disable driver loading or
runtime operations, and it does not change the `connections.list_kinds` MCP capability
enumeration.

`ui.form` defines connection form fields. Keep field ids aligned with the config shape consumed by `conn/open`.

Common field ids:

- `name`
- `host`
- `port`
- `username`
- `password`
- `database`
- `service_name`
- `sid`
- driver-specific extra params

Connection form field ids map directly into `DbConnectionConfig`: the basic fields above become first-class connection fields, and all other visible fields are stored in `extra_params` by their raw id. For driver-specific extra params, use the raw key, for example `GBASEDBTSERVER` or `PROTOCOL`; do not prefix form field ids with `extra_params.`.

Use locale keys when the driver ships `locales/`. Package verification should confirm `locales_dir` exists when declared.

## Compatibility Fallback

`dialect.compatible_database_type` is a host hint for external drivers. It lets the host fall back to built-in database plugin methods when the external driver returns not-supported for a wire method such as DDL building.

Example:

```json
{
  "dialect": {
    "identifier_quote_left": "\"",
    "identifier_quote_right": "\"",
    "compatible_database_type": "PostgreSQL"
  }
}
```

Use it when:

- The external database accepts SQL close to an existing built-in type.
- Host-generated DDL is better than the generic external fallback.
- Differences are acceptable for the host workflow being enabled.

Do not use it when:

- The target database has incompatible DDL semantics.
- The driver should implement its own SQL builder.
- Compatibility would hide a correctness bug.

Current intended values are existing host `DatabaseType` variants such as `MySQL`, `PostgreSQL`, `SQLite`, `MSSQL`, `Oracle`, and `ClickHouse`. Treat built-in DuckDB as temporary if the host is planning to remove it.
