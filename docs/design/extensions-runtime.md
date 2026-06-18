# Extensions Runtime Architecture

This document describes the current OnetCli extension runtime, including the
process-based database IPC driver path and the Wasm Component path used by
composite extensions.

For the lower-level IPC driver runtime and packaging contract, see
[`docs/design/ipc-drivers.md`](ipc-drivers.md).

## Goals

The extension system has three current integration points:

- **Language extensions** load Tree-sitter and syntax assets into the shared
  GPUI highlighter registry.
- **Database driver extensions** provide process-isolated database drivers over
  the IPC protocol.
- **Composite extensions** contribute commands, menus, toolbars, keybindings,
  and Wasm-backed database tree actions.

The important architectural boundary is that extensions cross stable data
contracts, not GPUI internals. Database IPC drivers speak framed JSON-RPC.
Composite Wasm extensions speak WIT interfaces and return declarative UI data.

## Startup And Discovery

`main/src/main.rs` initializes the extension runtime after core app
initialization:

```rust
onetcli_app::init(cx);
extension_runtime::init(cx);
```

`extension_runtime::init` in `crates/extension-runtime/src/extension/mod.rs`
does the runtime setup:

1. Resolves the extension root from the app config directory:
   `<config-dir>/extensions`.
2. Creates the global `ExtensionRegistry`.
3. Registers built-in providers:
   - `LanguageExtensionProvider`
   - `DatabaseDriverExtensionProvider`
   - `CompositeExtensionProvider`
4. Loads installed language extensions.
5. Refreshes the global composite runtime catalog.
6. Refreshes database tree menu contributions.
7. Registers the database tree extension action handler.

Extension directories are partitioned by kind:

```text
<config-dir>/extensions/
  languages/
  database_drivers/
  composite/
```

The generic provider abstraction is `ExtensionProvider` in
`crates/extension-runtime/src/extension/provider.rs`. Providers implement
installed listing, install-from-directory, and uninstall behavior for a specific
`ExtensionKind`.

## Composite Manifest Model

Composite extensions use `extension.json`. The schema lives in
`crates/extension-runtime/src/extension/manifest/schema.rs`.

The top-level manifest contains identity, compatibility, permissions, runtime
declarations, and contribution points:

```rust
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub engines: Engines,
    pub api: ApiVersions,
    pub permissions: Vec<String>,
    pub runtime: RuntimeSection,
    pub contributes: ContributesManifest,
    pub manifest_dir: PathBuf,
}
```

`RuntimeSection` currently models both process and Wasm runtimes:

```rust
pub struct RuntimeSection {
    pub ipc: Vec<IpcRuntime>,
    pub wasm: Vec<WasmRuntime>,
}
```

The Wasm runtime path is wired into command execution today. The IPC runtime
section is structurally defined and security-checked, but the production
database-driver path currently uses `driver.json` and `IpcDriverRegistry`.

The manifest loader validates:

- required identity fields,
- SemVer `version`,
- duplicate runtime ids across `runtime.ipc` and `runtime.wasm`,
- permission string syntax,
- Wasm module paths do not use absolute paths and do not escape the extension
  directory,
- IPC command paths do not escape the extension directory, and absolute IPC
  commands are restricted to the `/usr/bin/` allowlist.

## IPC Database Driver Runtime

IPC database drivers are process-isolated database plugins. The host starts a
driver executable, establishes a local socket transport, performs `init`
negotiation, and then calls database protocol methods over JSON-RPC.

The main crates are:

- `crates/extension-protocol`: method names, lifecycle types, envelope types,
  row/query/schema payloads, and length-prefixed JSON framing.
- `crates/extension-host`: host-side process management, local socket setup,
  JSON-RPC client, cancellation, timeout, and lifecycle negotiation.
- `crates/extension-driver`: driver-side runtime that routes requests to
  connection workers.
- `crates/db/src/ipc`: database-layer adapter that exposes IPC drivers through
  the existing `DatabasePlugin` and `DbConnection` traits.
- External extension repositories: concrete database driver implementations,
  such as the DuckDB IPC driver.

### Driver Manifest

Database drivers use `driver.json`. The schema is `IpcDriverManifest` in
`crates/db/src/ipc/registry.rs`:

```rust
pub struct IpcDriverManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: IpcDriverEntry,
    pub transport: IpcDriverTransport,
    pub dialect: IpcDriverDialect,
    pub capabilities: Option<DatabaseCapabilities>,
    pub methods: Vec<String>,
    pub ui: IpcDriverUi,
    pub manifest_dir: PathBuf,
}
```

The DuckDB driver manifest is the canonical example:

```json
{
  "id": "duckdb",
  "name": "DuckDB",
  "entry": {
    "command": "./duckdb_driver",
    "args": [],
    "working_dir": null
  },
  "transport": {
    "name": "duckdb-driver.sock",
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
    "exec/run",
    "exec/batch",
    "schema/databases",
    "schema/schemas",
    "schema/objects",
    "ddl/build_create_table"
  ]
}
```

Manifest method declarations are checked against
`extension_protocol::method::ALL_METHODS`. Driver-private methods are allowed
only under the `x/...` namespace.

### Discovery

`IpcDriverRegistry::load_default()` scans driver roots in this order:

1. User config directory: `<config-dir>/extensions/database_drivers`.

The registry accepts either a root containing multiple driver directories or a
single direct directory containing `driver.json`. The first manifest for a
driver id wins.

### Host-Side Connection Flow

The host-side adapter path is:

```text
DB UI
  -> GlobalDbState / DbManager
  -> ExternalDatabasePlugin
  -> ExternalDbConnection
  -> db::ipc::JsonRpcClient
  -> extension-host process + local socket
  -> extension-driver serve
  -> concrete driver connection worker
```

`DbManager` registers one `ExternalDatabasePlugin` per external driver id from
`IpcDriverRegistry`. `DatabaseType::External { driver_id }` resolves directly to
that plugin. Each plugin owns its concrete `IpcDriverManifest` and creates
`ExternalDbConnection` for that driver only.

`DatabaseType::DuckDB` remains user-facing as a built-in database type, but the
manager can route it to the `duckdb` IPC driver when that manifest is available.
This DuckDB bridge is not the generic external-driver identity contract.

`db::ipc::JsonRpcClient::start` then:

1. Builds a `SpawnConfig` from the driver manifest.
2. Calls `extension_host::process::spawn`.
3. Receives the connected local socket stream.
4. Splits the stream into reader and writer.
5. Starts the generic JSON-RPC client reader task.
6. Sends `init` through `extension_host::negotiation::negotiate`.
7. Stores the negotiated `ExtensionSession`.

`extension-host` creates the local socket listener and passes the socket name to
the child process through `ONETCLI_EXT_SOCKET`. The child driver connects back to
the host. `ProcessHandle` owns the child process and kills it on drop as a
cleanup fallback.

The wire transport is 4-byte little-endian length prefix plus JSON. Envelope
messages are JSON-RPC 2.0 requests, responses, and notifications.

### Driver-Side Runtime

Driver authors implement two traits from `crates/extension-driver/src/lib.rs`:

```rust
pub trait DriverConnection: Send {
    fn call(&mut self, method: &str, params: &Value)
        -> Result<Value, ProtocolError>;

    fn interrupt_hook(&self) -> Option<InterruptHook> {
        None
    }

    fn close(&mut self) {}
}

pub trait Driver: Send + Sync + 'static {
    fn init(&self, params: &Value) -> Result<Value, ProtocolError>;
    fn open_connection(&self, params: &Value) -> Result<OpenedConnection, ProtocolError>;
    fn call_connless(&self, method: &str, params: &Value) -> Result<Value, ProtocolError>;
    fn shutdown(&self) {}
}
```

`extension_driver::serve` owns the concurrency model:

- The reader task only reads frames and routes requests. It does not block on
  synchronous database APIs.
- `$/ping`, `shutdown`, and `$/cancelRequest` are handled directly on the
  reader path.
- `conn/open` runs in a background blocking task.
- Requests with `conn_id` are routed to that connection's dedicated worker
  thread. Each worker is FIFO.
- `cursor_id`, `stream_id`, and `import_id` are routed back to the worker that
  created the resource.
- Connectionless requests run in a background blocking task.

This keeps cancel, ping, and shutdown responsive even when a connection is busy
running a long query.

### Query And Execution Mapping

`ExternalDbConnection` maps the app's `DbConnection` trait to wire protocol
methods:

- `conn/open` returns a `conn_id`; subsequent connection-bound requests inject
  that id.
- Queries use `query/start` followed by repeated `cursor/fetch` and a final
  `cursor/close`.
- Non-query execution uses `exec/run`.
- Batch execution uses `exec/batch` when the driver declares support.
- Schema and metadata calls use the corresponding `schema/*` methods.

If the underlying reader task closes, `ExternalDbConnection` evicts the broken
client and clears `conn_id` so later calls fail as `NotConnected` and can be
recovered by higher layers.

## Wasm Component Runtime

Wasm Component extensions are loaded from composite manifests and executed with
Wasmtime. They are used for commands and database-tree actions, not for
arbitrary GPUI rendering.

### Runtime Registration

`ExtensionRuntimeCatalog` is built from installed composite manifests. Its
registration path in `crates/extension-runtime/src/registration.rs` is:

1. Register Wasm runtimes.
2. Register commands whose handler kind is `wasm`.
3. Register menu slots.
4. Register toolbar slots.
5. Register keybindings.
6. Register database tree menu contributions.

Each runtime is keyed as:

```text
<extension_id>::<runtime_id>
```

`WasmRuntimeBinding` stores:

- extension id,
- runtime key,
- runtime kind,
- resolved component module path,
- Wasm runtime config,
- manifest permissions.

Command contributions only bind to Wasm runtimes when
`command.handler.kind == "wasm"`. The handler's `runtime_id` must point to a
declared runtime in the same manifest.

### WIT Contract

The WIT world is in `crates/extension-api/wit/extension.wit`:

```wit
world extension {
    import db;
    import ui;
    import task;

    export activate: func();
    export run-action: func();
    export handle-view-action: func(event: view-action-event);
    export deactivate: func();
}
```

The current execution chain calls `run-action` and `handle-view-action`.
`activate` and `deactivate` are part of the contract but are not currently a
startup/shutdown execution path.

The UI contract returns declarative view data:

```wit
record view-spec {
    id: string,
    title: string,
    mode: view-mode,
    nodes: list<ui-node>,
    actions: list<ui-action>,
    window: option<view-window-options>,
}

open-view: func(view: view-spec);
```

The DB contract exposes controlled database access:

```wit
list-connections: func() -> result<list<connection-info>, db-error>;
open-session: func(connection-id: string, database: option<string>)
    -> result<session, db-error>;

resource session {
    execute: func(sql: string, options: exec-options)
        -> result<row-batch, db-error>;
    list-databases: func() -> result<list<string>, db-error>;
    list-schemas: func(database: string) -> result<list<string>, db-error>;
    close: func() -> result<_, db-error>;
}
```

### Runtime Execution

`ComponentRuntime` in `crates/extension-wasm/src/component.rs` loads the
component file with Wasmtime, enables the component model, async support, and
fuel consumption, and registers imports for:

- `onet:extension/db`
- `onet:extension/ui`
- `onet:extension/task`
- WASI Preview 2

Running an action follows this path:

```text
DbTreeExtensionActionHandler
  -> ExtensionRuntimeCatalog::run_db_tree_component_action
  -> ComponentRuntime::from_file
  -> ComponentHostState + ExtensionDbGateway
  -> Wasm export run-action
  -> Wasm imports db/ui/task
  -> ui.open-view(ViewSpec)
  -> ExtensionWidgetView popup
```

`ui.open-view` does not directly create a window from inside the Wasm call.
Instead, `ComponentHostState` records the `ViewSpec`. After `run-action`
returns, the host validates the views, resolves database selector options, and
opens `ExtensionWidgetView` popups.

When the user clicks an action in the extension view, the form state is packed
as `ViewActionEvent` and sent back to the Wasm component through
`handle-view-action`.

### Database Host Gateway

Wasm database imports are backed by `ExtensionDbGateway` in
`crates/extension-runtime/src/extension_db_gateway.rs`. It adapts Wasm resource
operations to `GlobalDbState`:

- `list_connections` returns connection summaries.
- `open_session` creates a direct database session.
- `execute` executes SQL through the existing DB session manager.
- `list_databases` and `list_schemas` call direct metadata helpers.
- `close_session` closes the underlying session.

`DbSessionResource` is tied to the calling extension id. The gateway rejects
foreign session resources and closed resources before executing operations.

## Declarative Extension UI

Composite extensions do not return GPUI objects. They return
`extension_component::ViewSpec`, which is rendered by `db_view`.

The renderer path is:

- `crates/db_view/src/extension_widget_view.rs`
- `crates/db_view/src/extension_widget.rs`
- `crates/db_view/src/extension_widget_view_controls.rs`
- `crates/db_view/src/extension_selector.rs`
- `crates/db_view/src/extension_selector_parts.rs`

Supported view content includes:

- text blocks,
- forms,
- text fields,
- text areas,
- password fields,
- checkboxes,
- selects,
- database selectors,
- action buttons.

The database selector source can be expanded into connection, database, schema,
table, and column selector parts. Options are loaded by the host with the same
permission set used for the Wasm action.

This keeps the plugin UI surface predictable and prevents extensions from
injecting arbitrary native UI code.

## Permissions And Safety

Composite permissions are declared in `extension.json` and validated during
manifest loading.

Database permission strings are parsed by `PermissionSet` in
`crates/extension-component/src/permissions.rs`.

Supported database permissions are:

```text
db:connections:list
db:read:<connection_id|*>
db:write:<connection_id|*>
db:schema:<connection_id|*>
db:admin:<connection_id|*>
```

Database permission levels are ordered:

```text
read < write < schema < admin
```

A higher level grants lower-risk access for the same connection scope. The `*`
connection id matches all connections.

UI and storage permissions are parsed as `ui:*` strings. Some UI host imports
are currently placeholders, so the permission model is ahead of parts of the
runtime implementation.

Important safety boundaries:

- Composite Wasm modules cannot escape the extension directory.
- Composite IPC command paths cannot escape the extension directory.
- IPC driver method declarations reject unknown standard method names.
- IPC transport readers remain responsive while connection workers run blocking
  database APIs.
- Wasm DB access is mediated by `ExtensionDbGateway` and `PermissionSet`.
- Wasm session resources are extension-owned.
- Wasmtime fuel is set per component invocation.

## Extension Manager Integration

The extension manager UI talks to `extension_view::ExtensionViewHost`.
`MainExtensionViewHost` in `crates/extension-runtime/src/extension_view_host.rs`
implements the host side.

It supports:

- listing installed extensions,
- loading marketplace entries,
- reviewing marketplace downloads,
- reviewing local tarballs,
- installing confirmed staging directories,
- uninstalling extensions,
- refreshing runtime contributions after install or uninstall.

Composite extension installs run permission review. If high-risk permissions are
present, installation returns `NeedsPermission` and requires explicit user
confirmation before moving the staged extension into the installed extension
root.

After install or uninstall, the runtime refreshes:

- `GlobalExtensionRuntimeCatalog`,
- database tree menu contributions.

## Current State And Known Gaps

The mature runtime paths are:

- process-based IPC database drivers,
- DuckDB IPC driver integration,
- language extension loading,
- composite Wasm command registration,
- database tree menu contributions,
- Wasm database actions,
- declarative extension view rendering,
- database permissions for Wasm DB access.

Areas that are defined but not fully wired yet:

- `runtime.ipc` in composite manifests is schema-defined and security-checked,
  but command execution currently wires Wasm handlers, not composite IPC
  handlers.
- WIT exports `activate` and `deactivate`, but the main execution path currently
  calls `run-action` and `handle-view-action`.
- `WasmRuntimeConfig.timeout_ms` is modeled, but the current component execution
  path relies on fuel and does not visibly wrap calls in a timeout.
- Some UI host imports, such as notify, refresh tree, result view, and progress
  updates, are placeholders or partial implementations.
- The Wasm `cursor` resource is modeled, but current DB gateway results are
  returned as a single `RowBatch` and generally do not expose a follow-up cursor.

## Verification Entry Points

Useful targeted checks when editing IPC runtime behavior:

```bash
cargo test -p extension-driver -- --nocapture
cargo test -p db ipc:: -- --nocapture
cargo check -p main -p db_view
```

Useful targeted checks when editing composite Wasm behavior:

```bash
cargo test -p extension-runtime extension_runtime_wasm_contract_tests -- --nocapture
cargo test -p extension-wasm -- --nocapture
cargo test -p extension-component -- --nocapture
cargo check -p extension-runtime -p extension-wasm -p extension-component -p db_view
```

Always include a direct check for the module being changed and report any
blocked command rather than claiming success without evidence.
