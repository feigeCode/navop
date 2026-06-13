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
if the manifest directory does not contain the executable and the current target
directory has a same-named binary, the registry rewrites the command to that
absolute target binary.

## Discovery Order

`IpcDriverRegistry::load_default()` scans driver roots in this order. The first
manifest for a driver id wins.

1. Paths from `ONETCLI_IPC_DRIVER_DIR`. Multiple paths can be supplied using the
   platform path separator.
2. User config directory: `<config-dir>/ipc-drivers`.
3. Bundled application directories:
   - macOS: `OnetCli.app/Contents/Resources/ipc-drivers`
   - Linux package: `/usr/share/onetcli/ipc-drivers`
   - Portable layout: `<executable-dir>/ipc-drivers`
4. Debug-only workspace fallback: `crates/duckdb_driver`, only when the built
   `duckdb_driver` binary exists beside the current debug executable.

The registry accepts both a root containing driver subdirectories and a direct
single-driver directory containing `driver.json`.

## Packaging

Use the shared packaging helper:

```bash
bash script/package-ipc-drivers.sh <target-triple> <destination-ipc-drivers-dir>
```

Examples:

```bash
bash script/package-ipc-drivers.sh aarch64-apple-darwin \
  target/OnetCli.app/Contents/Resources/ipc-drivers

bash script/package-ipc-drivers.sh x86_64-unknown-linux-gnu \
  package/usr/share/onetcli/ipc-drivers

bash script/package-ipc-drivers.sh x86_64-pc-windows-msvc \
  package/ipc-drivers
```

The release workflow builds both `main` and `duckdb_driver`, then packages the
driver manifest, binary, and locales into the platform discovery path.

## UI Behavior

DuckDB is still presented as the built-in `DatabaseType::DuckDB` connection type.
When the `duckdb` IPC driver is available, `DuckDbPlugin` uses it internally.
The new-connection UI filters that driver out of the generic external driver
list to avoid showing duplicate DuckDB entries.

Third-party drivers are shown as `ExternalDatabase` entries and must persist the
selected driver id in `extra_params[external_driver_id]`.

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
cargo test -p duckdb_driver -- --nocapture
cargo test -p db ipc:: -- --nocapture
cargo test -p db --test ipc_duckdb_driver -- --nocapture
cargo test -p main new_connection::connection_kind::tests::external_database_kinds_skip_builtin_duckdb_driver -- --nocapture
cargo check -p main -p db_view
git diff --check
```
