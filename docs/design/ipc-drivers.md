# IPC Driver Runtime And Packaging

This document records the current production contract for process-based database
IPC drivers.

## Runtime Model

The host must keep the transport reader responsive. A driver request is routed by
method and resource id:

- `$/ping`, `shutdown`, and `$/cancelRequest` are handled directly by the runtime
  reader path.
- `conn/open` runs in a background blocking task. Slow connection creation must
  not block ping, shutdown, cancel, or connectionless requests.
- Requests with a `conn_id` run on that connection's dedicated worker thread.
  Each connection worker is FIFO and may call synchronous database APIs.
- `cursor_id`, `stream_id`, and `import_id` are routed back to the connection
  worker that created the resource.
- Requests without `conn_id` or a routed resource id are connectionless and run
  in a background blocking task, not on a connection worker.

Driver methods that do not need a database connection must not require or inject
`conn_id`. Otherwise they will be serialized behind that connection worker and
can freeze user-facing workflows.

Timeout and cancellation are cooperative at the protocol boundary:

- `extension-host` sends `$/cancelRequest` on request timeout or cancellation
  token cancellation.
- `extension-driver` interrupts only the worker request that is currently running.
- Cancelled queued or running requests are normalized to `REQUEST_CANCELLED`.

## Driver Layout

A driver directory contains a `driver.json` manifest and the executable named by
the manifest entry:

```text
ipc-drivers/
  duckdb/
    driver.json
    duckdb_driver
    locales/
      en.yml
      zh-CN.yml
      zh-HK.yml
```

The DuckDB manifest uses a relative executable path:

```json
{
  "id": "duckdb",
  "name": "DuckDB",
  "entry": {
    "command": "./duckdb_driver",
    "args": []
  },
  "transport": {
    "name": "duckdb-driver.sock",
    "connect_timeout_ms": 5000
  }
}
```

Relative commands are resolved against the manifest directory. In development,
the driver package should be built and installed by the extension repository,
not by this application repository.

## Discovery Order

`IpcDriverRegistry::load_default()` scans installed database driver extensions
from the user config directory:

```text
<config-dir>/extensions/database_drivers
```

The registry accepts both a root containing driver subdirectories and a direct
single-driver directory containing `driver.json`.

## Packaging

The main application release workflow builds only `main`. IPC driver binaries
and marketplace manifests are published by extension-specific release pipelines,
currently in the external extensions repository. This repository owns the host
runtime and registry contracts, not DuckDB driver production.

## UI Behavior

DuckDB is still presented as the built-in `DatabaseType::DuckDB` connection type.
When the `duckdb` IPC driver is available, the DuckDB connection path uses it internally.
The new-connection UI filters that driver out of the generic external driver
list to avoid showing duplicate DuckDB entries.

Third-party drivers are shown as `ExternalDatabase` entries and persist the
selected driver id in `DatabaseType::External { driver_id }`. The database type
is the source of truth for external driver identity.

When the connection form edits an existing external connection, it resolves the
driver id from `DatabaseType::External { driver_id }` and asks
`create_external_connection_form_for(driver_id, ...)` for that driver's manifest
form. This prevents existing external connections from falling back to a generic
external form.

Driver connection forms are manifest-driven:

- `ui.form.tabs` maps directly to connection form tabs, so drivers can expose
  SSH, SSL, advanced, or database-specific groups without host UI changes.
- `Checkbox` and `FilePath` are supported connection field types.
- `visible_when` rules are preserved by the manifest bridge and enforced during
  rendering, validation, and `extra_params` construction. Invisible fields are
  not validated or saved.
- Driver-localized labels are resolved from `ui.locales_dir` first, then the app
  locale/raw fallback.

## Host Integration Contract

`DbManager` registers one `ExternalDatabasePlugin` per manifest driver id. Each
plugin instance owns its concrete `IpcDriverManifest` and serves only that
driver. `DatabaseType::External { driver_id }` is routed to the matching plugin.

`DatabaseType::DuckDB` remains a built-in-facing type for users, but the manager
may back it with the `duckdb` IPC driver when the manifest is available. This is
a compatibility bridge for DuckDB only; future third-party databases should use
`DatabaseType::External { driver_id }`.

The connection config sent over IPC includes both:

- `database_type_key`: the storage key such as `DuckDB` or `External:iotdb`.
- `driver_id`: the concrete external driver id.

## Manifest SQL And Dialect Contract

The manifest dialect is the host SQL-generation contract for external drivers.
It currently includes:

- `identifier_quote_left` and `identifier_quote_right`, with right-quote
  escaping. This supports asymmetric quoting such as `[name]`.
- `limit_style`: `limit_offset` or `offset_fetch`.
- `bool_true` and `bool_false` literals.
- `explain_template`, used as the fallback SQL carried with `sql/explain`.

Host-side SQL builders must use the driver dialect uniformly. The external
plugin's local fallback covers column changes and index changes, including
`DROP INDEX IF EXISTS ...` and `CREATE [UNIQUE] INDEX ...`. When a driver
declares DDL methods such as `ddl/build_create_table` or
`ddl/build_alter_table`, async table designer paths ask the driver first and
fall back to local SQL only when the method is not supported.

`sql/explain` is connection-bound. The host builds a wire pseudo-SQL request
containing the original SQL plus dialect fallback SQL so the driver can either
execute native explain behavior or use the fallback template.

## Object View Header Customization

Drivers can customize the object-list table rendered by the database object
panel with the optional `schema/object_view` method. This method is
connection-bound and is called before the host falls back to the older
`schema/databases`, `schema/objects`, `schema/columns`, `schema/indexes`, and
other fixed mappings.

Declare the method in `driver.json` when the driver implements it:

```json
{
  "methods": [
    "schema/object_view",
    "schema/databases",
    "schema/objects",
    "schema/columns"
  ]
}
```

If the method is absent from `methods`, or the driver returns the standard
not-supported error for this method, the host keeps the legacy object view
behavior. This makes the method safe to add incrementally.

### Request

The host sends `schema/object_view` with these params:

```json
{
  "conn_id": 12,
  "view": "columns",
  "database": "main",
  "schema": "public",
  "table": "events"
}
```

Fields:

- `conn_id`: injected by the host for connection-bound routing.
- `view`: one of `databases`, `schemas`, `tables`, `columns`, `indexes`,
  `views`, `functions`, `procedures`, `triggers`, or `sequences`.
- `database`: present when the requested view is scoped to a database.
- `schema`: present when the requested view is scoped to a schema.
- `table`: present for table-scoped views such as `columns` and `indexes`.

### Response

Return the full table shape the host should render:

```json
{
  "title": "Columns",
  "columns": [
    { "key": "name", "name": "Field", "width_px": 220 },
    { "key": "type", "name": "Type", "width_px": 160 },
    { "key": "nullable", "name": "Null?", "width_px": 72, "align": "right" },
    { "key": "comment", "name": "Comment", "width_px": 260 }
  ],
  "rows": [
    ["id", "BIGINT", "false", "primary key"],
    ["payload", "JSON", "true", ""]
  ]
}
```

Response fields:

- `title`: optional. Empty or omitted uses the host default title for that view.
- `columns`: required for the custom view to be used. The order is the rendered
  order, so omit fields the driver does not want to display.
- `columns[].key`: stable column identifier, unique within the view.
- `columns[].name`: header label shown to the user.
- `columns[].width_px`: optional width in pixels. Omit it to use the host
  default width.
- `columns[].align`: optional text alignment: `left`, `center`, or `right`.
  Omit it for left alignment.
- `rows`: each row is an array of strings aligned with `columns`.

The host normalizes row length to the number of rendered columns: extra values
are ignored and missing values become empty strings. Drivers should still return
exactly one value per column because it is easier to inspect and test.

### Choosing Fields

The driver owns the display field set for `schema/object_view`. For example, a
time-series driver can render measurements with `name`, `tags`, `fields`, and
`retention_policy` instead of forcing the generic `Name` / `Comment` columns.
The same driver can render `columns` as `name`, `type`, `nullable`,
`encoding`, and `compression` if those fields are important for that database.

Keep the first column as the object name when the row represents a clickable
database object. The current object panel uses the first cell for the row label
and icon pairing.

### Rust Driver Example

A driver using `extension-driver` can handle the method in
`DriverConnection::call`:

```rust
use extension_protocol::{error_codes, method, schema, ProtocolError};
use serde_json::Value;

fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
    match method_name {
        method::SCHEMA_OBJECT_VIEW => {
            let params: schema::ObjectViewParams = serde_json::from_value(params.clone())
                .map_err(|error| ProtocolError::new(
                    error_codes::INVALID_PARAMS,
                    error.to_string(),
                ))?;
            let view = match params.view {
                schema::ObjectViewKind::Columns => schema::ObjectView {
                    title: "Columns".into(),
                    columns: vec![
                        schema::ObjectViewColumn {
                            key: "name".into(),
                            name: "Field".into(),
                            width_px: Some(220.0),
                            align: None,
                        },
                        schema::ObjectViewColumn {
                            key: "nullable".into(),
                            name: "Null?".into(),
                            width_px: Some(72.0),
                            align: Some(schema::ObjectViewColumnAlign::Right),
                        },
                    ],
                    rows: load_column_rows(&params.database, &params.schema, &params.table)?,
                },
                _ => {
                    return Err(ProtocolError::new(
                        error_codes::METHOD_NOT_FOUND,
                        format!("unsupported object view: {}", params.view.as_str()),
                    ));
                }
            };
            serde_json::to_value(view).map_err(|error| ProtocolError::new(
                error_codes::INTERNAL_ERROR,
                error.to_string(),
            ))
        }
        _ => Err(ProtocolError::new(
            error_codes::METHOD_NOT_FOUND,
            format!("unknown method: {method_name}"),
        )),
    }
}
```

Unsupported `view` values should return `METHOD_NOT_FOUND` or another error
that the driver adapter maps to `DbError::NotSupported`; the host treats that as
the signal to fall back to legacy rendering for that object view.

## Display And Resources

Driver display metadata resolves from the manifest:

- driver id
- driver name
- icon asset path

Built-in icon names map to app assets. Relative custom driver icons become
`driver://{driver_id}/icon` or `driver://{driver_id}/icon_color`. The main app
asset source tries `DriverAssetSource` first and falls back to bundled GPUI
assets.

Connection tree nodes store external driver metadata for display. Extension menu
`when` contexts keep `connection.kind == "external"` for broad compatibility and
add `connection.driver_id` for driver-specific menu filtering.

## Extensibility Rules

Manifest `methods` should declare supported protocol methods. Unknown standard
method names are rejected to catch typos. Driver-private methods are allowed
under the `x/<driver-or-vendor>/...` namespace.

When adding a new protocol method:

- Add it to `extension-protocol::method`.
- Decide whether it is connection-bound, resource-bound, or connectionless.
- Add route handling in `extension-driver` only if it creates or consumes a
  routed resource id.
- Add method gating in `db::ipc::MethodSet` if the DB adapter calls it.
- Add a manifest declaration test in the driver.

## Verification

Useful targeted checks:

```bash
cargo test -p extension-driver -- --nocapture
cargo test -p db ipc:: -- --nocapture
cargo test -p db manager::tests -- --nocapture
cargo test -p db_view common:: -- --nocapture
cargo test -p db_view database_view_plugin::tests -- --nocapture
cargo test -p db_view db_tree_view::tests -- --nocapture
cargo test -p db_view connection_form_window::tests -- --nocapture
cargo test -p db_view extension_menu -- --nocapture
cargo test -p db_view table_designer_tab::tests -- --nocapture
cargo test -p extension-runtime database_driver_install -- --nocapture
cargo test -p main new_connection::connection_kind::tests::external_database_kinds_skip_builtin_duckdb_external_driver -- --nocapture
cargo check -p one-core -p db -p db_view -p extension-runtime -p main
cargo fmt --check
git diff --check
```
