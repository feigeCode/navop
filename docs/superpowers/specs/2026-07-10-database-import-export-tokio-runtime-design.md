# Database Import/Export Tokio Runtime Design

**Date:** 2026-07-10
**Status:** Implemented and verified in the isolated issue branch

## Summary

OnetCli database import and export operations must always execute on the application-owned Tokio runtime. The current SQL dump, table export, and table import views start Tokio-dependent database futures with GPUI `background_spawn`; on macOS this polls them on a GCD queue without a Tokio runtime context. MySQL connection setup then panics in `tokio::time::timeout`, and the panic crosses GPUI's `extern "C"` dispatcher trampoline, causing `abort()` and terminating the application.

The redesign makes the safe execution path the only public progress API. `GlobalDbState` will return GPUI `Task` handles whose database futures are internally scheduled with `one_core::gpui_tokio::Tokio::spawn_result`. The direct runtime-bound implementations become private, explicitly named methods with a defensive Tokio-context check. All three affected views use the task-returning APIs while retaining their existing progress-channel and UI-update loops.

## Goals

- Fix GitHub issue #103 so starting a MySQL SQL dump cannot terminate OnetCli because the task lacks a Tokio runtime.
- Fix the same executor mismatch in table export and table import.
- Make public import/export progress APIs safe by construction so callers do not choose between GPUI and Tokio executors.
- Convert an unexpected database-task panic into a returned `JoinError`/`anyhow::Error` instead of allowing it to cross a macOS C ABI boundary.
- Preserve progress events, file streaming, cancellation-on-task-drop, and existing UI success/error behavior.
- Add deterministic regression coverage for the Tokio runtime contract without depending on a real database or nondeterministic GPUI/Tokio worker timing.

## Non-Goals

- Changing SQL dump formatting, pagination, file naming, or database query behavior.
- Reworking the connection pool or session lifecycle.
- Adding retry behavior for failed imports or exports.
- Changing the UI layout or progress presentation.
- Catching arbitrary panics around GPUI's platform dispatcher.

## Existing Failure Path

The affected views currently create a progress channel and then run the database future with GPUI:

```text
SqlDumpView / DataExportView / TableImportView
  -> cx.background_spawn(..._with_progress_sync(...))
  -> ConnectionManager::create_session
  -> MySqlPlugin::create_connection
  -> MysqlDbConnection::connect
  -> tokio::time::timeout
  -> Tokio Handle::current panic
  -> async_task::Runnable::run
  -> gpui_macos extern "C" trampoline
  -> abort / SIGABRT
```

The `_sync` suffix is misleading: these methods are asynchronous and require a Tokio runtime, but their public API does not express that contract.

## Architecture

### Safe Public Task APIs

`GlobalDbState` owns the executor decision and exposes task-returning progress methods:

```rust
pub struct ExportProgressRequest {
    pub connection_id: String,
    pub config: ExportConfig,
    pub progress_tx: Option<ExportProgressSender>,
}

pub struct ImportProgressRequest {
    pub connection_id: String,
    pub config: ImportConfig,
    pub data: String,
    pub file_name: String,
    pub progress_tx: Option<ImportProgressSender>,
}

pub fn export_data_with_progress<C: AppContext>(
    &self,
    cx: &C,
    request: ExportProgressRequest,
) -> Task<anyhow::Result<ExportResult>>;

pub fn import_data_with_progress<C: AppContext>(
    &self,
    cx: &C,
    request: ImportProgressRequest,
) -> Task<anyhow::Result<ImportResult>>;
```

Each method clones `GlobalDbState` and calls `Tokio::spawn_result`. Returning the `Task` instead of awaiting it lets the views consume progress events concurrently and await the final result after the sender closes.

The request types live in a focused `crates/db/src/import_export/task.rs` module. This keeps method parameter counts within the repository limit and avoids pushing the existing `import_export/mod.rs` beyond its file-size ceiling.

The existing `export_data` and `import_data` convenience methods remain asynchronous. They use the same Tokio-backed core path and therefore do not duplicate session/export logic.

### Private Runtime-Bound Core

Session acquisition, plugin calls, and session release move into private methods:

```rust
async fn export_data_with_progress_on_tokio(...) -> anyhow::Result<ExportResult>;
async fn import_data_with_progress_on_tokio(...) -> anyhow::Result<ImportResult>;
```

The public `_with_progress_sync` methods are removed. No compatibility alias is retained because preserving the unsafe and misleading entry point would allow the bug to recur.

At the beginning of each private core method, a small runtime-contract helper verifies `tokio::runtime::Handle::try_current()`. A missing runtime becomes a descriptive `anyhow::Error`, providing defense in depth for future internal callers.

### Runtime Contract Module

A focused `crates/db/src/runtime_contract.rs` module contains:

```rust
pub(crate) fn require_tokio_runtime(operation: &str) -> anyhow::Result<()>;
```

This keeps runtime validation out of the already-large manager module and allows deterministic unit tests:

- A normal `#[test]` proves the helper returns an error outside Tokio.
- A `#[tokio::test]` proves it succeeds inside Tokio.

The error includes the operation name so logs identify whether export or import violated the contract.

## Data Flow

SQL dump and table export use this flow:

```text
GPUI view task
  -> create unbounded progress channel
  -> GlobalDbState::export_data_with_progress(cx, ...)
     -> Tokio::spawn_result
        -> private Tokio core
           -> create session
           -> plugin export
           -> send progress events
           -> release session
  -> GPUI view receives progress events and writes/updates UI
  -> progress sender drops
  -> view awaits returned Task
  -> render success or structured error
```

Table import follows the same pattern with `ImportProgressEvent`.

## Error And Panic Handling

- Missing Tokio context: return a descriptive error before connection setup.
- Database connection/query failure: preserve the existing `anyhow::Result` flow and UI error logging.
- Tokio task panic: `Tokio::spawn_result` observes it as `JoinError`; the GPUI task receives an error instead of unwinding through the macOS dispatcher.
- Dropped view/task: retain the current `Tokio::spawn_result` behavior, which aborts the Tokio task when the GPUI task handle is dropped.
- Progress receiver closure: exit the progress loop and await the task result, preserving current completion behavior.
- Session release: preserve current release behavior and error propagation; this change does not alter pool semantics.

## Affected Files

- `AGENTS.md`: record the executor boundary for Tokio-dependent database futures.
- `crates/db/src/lib.rs`: register the runtime-contract module.
- `crates/db/src/runtime_contract.rs`: implement and test the Tokio runtime contract.
- `crates/db/src/import_export/mod.rs`: register and re-export transfer task request types.
- `crates/db/src/import_export/task.rs`: define export/import progress request objects.
- `crates/db/src/manager.rs`: replace duplicated public/direct progress methods with safe task APIs and private Tokio-bound cores.
- `crates/db/src/manager_runtime_contract_tests.rs`: directly prove both private cores reject a missing Tokio runtime before connection lookup.
- `crates/db_view/src/import_export/sql_dump_view.rs`: use the safe export task API.
- `crates/db_view/src/import_export/table_export_view.rs`: use the safe export task API.
- `crates/db_view/src/import_export/table_import_view.rs`: use the safe import task API.

## Testing Strategy

The implementation follows TDD:

1. Add runtime-contract tests and verify the outside-Tokio test fails before the helper exists.
2. Implement the minimal helper and verify both outside/inside Tokio tests pass.
3. Directly poll both private cores without Tokio and prove they return operation-specific runtime errors before any connection lookup.
4. Refactor manager APIs so the runtime-bound core is private and guarded.
5. Migrate all three UI call sites to the task-returning public APIs.
6. Run searches proving no `_with_progress_sync` or affected `cx.background_spawn` call remains.
7. Run targeted `db` and `db_view` tests, then `cargo check -p main` and clippy for the changed crates.
8. Perform code review and completion verification before reporting the issue fixed.

No test will wait on a real MySQL server or depend on nondeterministic GPUI/Tokio worker completion timing. The runtime helper tests cover the contract directly; compilation and call-site searches prove the UI paths use the safe public API.

## Acceptance Criteria

- SQL dump, table export, and table import database futures are spawned by `Tokio::spawn_result`, owned through `GlobalDbState` APIs.
- No affected view directly drives a database import/export future with `cx.background_spawn`.
- No public `export_data_with_progress_sync` or `import_data_with_progress_sync` API remains.
- Runtime-bound core methods fail with a structured error when invoked without Tokio.
- Existing progress, file-writing, result, and cancellation flows remain intact.
- Runtime-contract tests pass both outside and inside Tokio.
- Relevant tests, checks, clippy, review, and final verification complete without unreported failures.
