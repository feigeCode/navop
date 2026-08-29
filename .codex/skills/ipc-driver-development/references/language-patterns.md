# Language Implementation Patterns

## Table of Contents

- [Choose a Runtime](#choose-a-runtime)
- [Rust Pattern](#rust-pattern)
- [Node.js Pattern](#nodejs-pattern)
- [Python Pattern](#python-pattern)
- [Cross-Language Checklist](#cross-language-checklist)

## Choose a Runtime

Any language is acceptable if it can:

- Start as a long-lived process from `driver.json.entry.command`.
- Read/write the host IPC transport required by the runtime.
- Parse JSON-RPC request envelopes and return matching response envelopes.
- Preserve request ids and typed error objects.
- Manage connection state keyed by `conn_id`.
- Support cancellation or interruption for long-running queries, or clearly return unsupported where the protocol permits it.

Prefer Rust when extending this repository's current driver style because `extension-driver` and `extension-protocol` remove most framing and type-shape risk.

Choose another language when the database SDK is materially better there, but compensate with contract tests against captured JSON fixtures.

## Rust Pattern

Use:

- `extension-protocol` for typed params/results and method constants.
- `extension-driver` for runtime, driver trait, connection trait, and interruption hook.
- `serde_json::from_value` / `serde_json::to_value` for all wire conversions.

Recommended module split:

```text
src/
  main.rs              # runtime entry
  driver.rs            # Driver / DriverConnection routing
  state.rs             # conn_id -> session state
  session.rs           # database SDK wrapper
  handlers.rs          # protocol handlers
  ddl.rs               # pure DDL builders
```

Routing rule:

- `Driver::call_connless`: `conn/test`, pure `ddl/build*`
- `DriverConnection::call`: live connection methods and duplicate `ddl/build*` acceptance

Test handlers directly with JSON params. Add integration tests through the host IPC client when behavior depends on runtime framing.

## Node.js Pattern

Use Node.js when the database client library is strongest in JavaScript/TypeScript.

Recommended module split:

```text
src/
  index.ts             # process bootstrap
  rpc.ts               # JSON-RPC framing and dispatch
  manifest.ts          # method list assertions
  connections.ts       # conn_id state
  handlers/
    conn.ts
    query.ts
    schema.ts
    ddl.ts
```

Practical rules:

- Use TypeScript types that mirror `extension-protocol` structs.
- Validate incoming params with a schema library or narrow parser.
- Keep result row metadata stable: column names, type strings, nullability when available.
- Stream large results through cursor methods rather than returning huge arrays.
- Make cancellation explicit with `AbortController` or the database client's cancellation API.

Package the built JS and runtime launcher so `entry.command` works from the archive root.

## Python Pattern

Use Python when the database API is mature and deployment size is acceptable.

Recommended module split:

```text
driver/
  __main__.py          # process bootstrap
  rpc.py               # JSON-RPC framing and dispatch
  connections.py       # conn_id state
  handlers/
    conn.py
    query.py
    schema.py
    ddl.py
```

Practical rules:

- Use dataclasses or pydantic-style validation for params/results.
- Normalize database exceptions into protocol errors at handler boundaries.
- Avoid global singletons for live connections; keep explicit `conn_id` state.
- For packaging, produce a stable executable entry: zipapp, PyInstaller binary, or a small launcher with vendored dependencies.
- Test on the same target platforms declared in `extension.build.json`.

## Cross-Language Checklist

For every language:

- Keep manifest method names generated or tested from the dispatcher.
- Add JSON fixture tests for `conn/test`, `conn/open`, at least one query, and at least one schema method.
- Quote identifiers with a dialect-aware helper.
- Distinguish database/catalog, schema, and table in data structures.
- Return `NotSupported` instead of silent empty success for unimplemented optional behavior.
- Ensure process shutdown closes all database sessions.
- Keep import/export streams bounded in memory.
- Log enough context for diagnosis without leaking passwords, tokens, or connection strings.
