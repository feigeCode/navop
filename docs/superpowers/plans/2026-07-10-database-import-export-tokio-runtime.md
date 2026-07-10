# Database Import/Export Tokio Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make SQL dump, table export, and table import execute database futures on OnetCli's Tokio runtime and return structured errors instead of aborting the macOS process.

**Architecture:** `GlobalDbState` exposes task-returning progress APIs that internally call `Tokio::spawn_result`; private Tokio-bound core methods own session/plugin work and enforce a small runtime contract. The three GPUI views keep their progress loops but obtain task handles only through `GlobalDbState`, removing the unsafe public `_sync` methods.

**Tech Stack:** Rust, GPUI `Task`/`AppContext`, Tokio runtime, `anyhow`, existing `db` and `db_view` unit-test infrastructure.

---

### Task 1: Add the Tokio runtime contract with TDD

**Files:**
- Create: `crates/db/src/runtime_contract.rs`
- Modify: `crates/db/src/lib.rs`

- [x] **Step 1: Register the module and write tests before the helper exists**

Add this module declaration to `crates/db/src/lib.rs` near the other private support modules:

```rust
mod runtime_contract;
```

Create `crates/db/src/runtime_contract.rs` containing only the tests initially:

```rust
#[cfg(test)]
mod tests {
    use super::require_tokio_runtime;

    #[test]
    fn rejects_database_operation_outside_tokio_runtime() {
        let error = require_tokio_runtime("database export")
            .expect_err("operation outside Tokio should fail");

        assert!(error.to_string().contains("database export"));
        assert!(error.to_string().contains("Tokio runtime"));
    }

    #[tokio::test]
    async fn accepts_database_operation_inside_tokio_runtime() {
        require_tokio_runtime("database export")
            .expect("operation inside Tokio should succeed");
    }
}
```

- [x] **Step 2: Run the tests and verify RED**

Run:

```bash
rtk cargo test -p db runtime_contract --lib
```

Expected: compilation fails because `require_tokio_runtime` is not defined. This proves the tests require the new runtime contract.

- [x] **Step 3: Implement the minimal runtime contract**

Add above the test module in `crates/db/src/runtime_contract.rs`:

```rust
pub(crate) fn require_tokio_runtime(operation: &str) -> anyhow::Result<()> {
    tokio::runtime::Handle::try_current()
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("{operation} requires the application Tokio runtime"))
}
```

- [x] **Step 4: Run the tests and verify GREEN**

Run:

```bash
rtk cargo test -p db runtime_contract --lib
```

Expected: both runtime-contract tests pass.

- [ ] **Step 5: Commit the runtime contract**

```bash
git add crates/db/src/lib.rs crates/db/src/runtime_contract.rs
git commit -m "test(db): define database Tokio runtime contract"
```

### Task 2: Add a failing structural regression test for all three UI paths

**Files:**
- Modify: `crates/db_view/src/import_export/mod.rs`

- [x] **Step 1: Add source-contract tests for export and import task creation**

Append to `crates/db_view/src/import_export/mod.rs`:

```rust
#[cfg(test)]
mod runtime_tests {
    const SQL_DUMP_VIEW: &str = include_str!("sql_dump_view.rs");
    const TABLE_EXPORT_VIEW: &str = include_str!("table_export_view.rs");
    const TABLE_IMPORT_VIEW: &str = include_str!("table_import_view.rs");
    const UNSAFE_EXPORT_API: &str = concat!("export_data_with_progress_", "sync");
    const UNSAFE_IMPORT_API: &str = concat!("import_data_with_progress_", "sync");
    const GPUI_BACKGROUND_SPAWN: &str = concat!("background_", "spawn");

    fn assert_safe_export_task(source: &str) {
        assert!(!source.contains(UNSAFE_EXPORT_API));
        assert!(!source.contains(GPUI_BACKGROUND_SPAWN));
        assert!(source.contains(".export_data_with_progress("));
    }

    #[test]
    fn database_transfer_views_delegate_runtime_to_global_state() {
        assert_safe_export_task(SQL_DUMP_VIEW);
        assert_safe_export_task(TABLE_EXPORT_VIEW);
        assert!(!TABLE_IMPORT_VIEW.contains(UNSAFE_IMPORT_API));
        assert!(!TABLE_IMPORT_VIEW.contains(GPUI_BACKGROUND_SPAWN));
        assert!(TABLE_IMPORT_VIEW.contains(".import_data_with_progress("));
    }
}
```

- [x] **Step 2: Run the test and verify RED**

Run:

```bash
rtk cargo test -p db_view database_transfer_views_delegate_runtime_to_global_state --lib
```

Expected: the test fails because all three views still contain `_with_progress_sync`, use `cx.background_spawn`, and do not call the safe task APIs.

### Task 3: Make `GlobalDbState` task APIs safe by construction

**Files:**
- Create: `crates/db/src/import_export/task.rs`
- Modify: `crates/db/src/import_export/mod.rs`
- Modify: `crates/db/src/manager.rs:2548-2735`

- [x] **Step 1: Define request objects with bounded parameter counts**

Create `crates/db/src/import_export/task.rs`:

```rust
use super::{
    ExportConfig, ExportProgressSender, ImportConfig, ImportProgressSender,
};

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
```

Register and re-export the module from `crates/db/src/import_export/mod.rs`:

```rust
mod task;
pub use task::{ExportProgressRequest, ImportProgressRequest};
```

- [x] **Step 2: Import `Task`, request objects, and the runtime contract**

Update the imports in `crates/db/src/manager.rs`:

```rust
use crate::runtime_contract::require_tokio_runtime;
use crate::{ExportProgressRequest, ImportProgressRequest};
use gpui::{AppContext, AsyncApp, Global, Task};
```

- [x] **Step 3: Convert export progress into a task-returning API**

Replace the current async public `export_data_with_progress` and public `export_data_with_progress_sync` methods with:

```rust
pub fn export_data_with_progress<C: AppContext>(
    &self,
    cx: &C,
    request: ExportProgressRequest,
) -> Task<anyhow::Result<ExportResult>> {
    let clone_self = self.clone();
    Tokio::spawn_result(cx, async move {
        clone_self
            .export_data_with_progress_on_tokio(request)
            .await
    })
}

async fn export_data_with_progress_on_tokio(
    &self,
    request: ExportProgressRequest,
) -> anyhow::Result<ExportResult> {
    require_tokio_runtime("database export")?;
    let db_config = self
        .get_config(&request.connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", request.connection_id))?;
    let plugin = self.get_plugin(&db_config.database_type)?;
    let session_id = self
        .connection_manager
        .create_session(db_config.clone(), &self.db_manager)
        .await?;

    let result = {
        let mut guard = self
            .connection_manager
            .get_session_connection(&session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        plugin
            .export_data_with_progress(conn, &request.config, request.progress_tx)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))
    };

    self.connection_manager
        .release_session(&session_id)
        .await
        .map_err(|error| anyhow::anyhow!("{}", error))?;
    result
}
```

Keep `export_data` asynchronous and have it await the returned task:

```rust
self.export_data_with_progress(
    cx,
    ExportProgressRequest {
        connection_id,
        config,
        progress_tx: None,
    },
)
    .await
```

- [x] **Step 4: Add the task-returning import progress API**

Replace duplicated import/session logic with:

```rust
pub fn import_data_with_progress<C: AppContext>(
    &self,
    cx: &C,
    request: ImportProgressRequest,
) -> Task<anyhow::Result<ImportResult>> {
    let clone_self = self.clone();
    Tokio::spawn_result(cx, async move {
        clone_self.import_data_with_progress_on_tokio(request).await
    })
}

async fn import_data_with_progress_on_tokio(
    &self,
    request: ImportProgressRequest,
) -> anyhow::Result<ImportResult> {
    require_tokio_runtime("database import")?;
    let db_config = self
        .get_config(&request.connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", request.connection_id))?;
    let plugin = self.get_plugin(&db_config.database_type)?;
    let session_id = self
        .connection_manager
        .create_session(db_config.clone(), &self.db_manager)
        .await?;

    let result = {
        let mut guard = self
            .connection_manager
            .get_session_connection(&session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        plugin
            .import_data_with_progress(
                conn,
                &request.config,
                &request.data,
                &request.file_name,
                request.progress_tx,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))
    };

    self.connection_manager
        .release_session(&session_id)
        .await
        .map_err(|error| anyhow::anyhow!("{}", error))?;
    result
}
```

Update `import_data` to construct `ImportProgressRequest` with an empty file name and no progress sender, then call and await this API, removing its duplicated session/plugin implementation.

- [x] **Step 5: Format and compile the `db` crate**

Run:

```bash
rtk cargo fmt --all -- --check
rtk cargo check -p db
```

Expected: formatting and `db` compilation succeed. The structural `db_view` regression test remains red until Task 4.

Result: targeted `rustfmt --check` was used for the changed Rust files so unrelated
workspace formatting was preserved; `db`, `db_view`, and `main` compile successfully.

### Task 4: Migrate SQL dump, table export, and table import

**Files:**
- Modify: `crates/db_view/src/import_export/sql_dump_view.rs:261-272`
- Modify: `crates/db_view/src/import_export/table_export_view.rs:602-613`
- Modify: `crates/db_view/src/import_export/table_import_view.rs:494-508`

- [x] **Step 1: Migrate SQL dump and table export**

In both export views, replace the `cx.background_spawn` block with:

```rust
let export_handle = global_state_clone.export_data_with_progress(
    cx,
    db::ExportProgressRequest {
        connection_id: connection_id_clone,
        config: export_config,
        progress_tx: Some(progress_tx),
    },
);
```

Do not change the progress receiver loop, file-writing code, or final `export_handle.await` handling.

- [x] **Step 2: Migrate table import**

Replace the import `cx.background_spawn` block with:

```rust
let import_handle = global_state_clone.import_data_with_progress(
    cx,
    db::ImportProgressRequest {
        connection_id: connection_id_clone,
        config: import_config,
        data,
        file_name,
        progress_tx: Some(progress_tx),
    },
);
```

Do not change progress rendering or final result handling.

- [x] **Step 3: Run the structural test and verify GREEN**

Run:

```bash
rtk cargo test -p db_view database_transfer_views_delegate_runtime_to_global_state --lib
```

Expected: the regression test passes for all three views.

- [x] **Step 4: Prove the unsafe API and call pattern are gone**

Run:

```bash
rtk rg -n "export_data_with_progress_sync|import_data_with_progress_sync" crates
rtk rg -n "let (export|import)_handle = cx\.background_spawn" crates/db_view/src/import_export
```

Expected: both searches return no matches.

- [ ] **Step 5: Commit the runtime migration**

```bash
git add crates/db/src/import_export/mod.rs crates/db/src/import_export/task.rs crates/db/src/manager.rs crates/db_view/src/import_export/mod.rs crates/db_view/src/import_export/sql_dump_view.rs crates/db_view/src/import_export/table_export_view.rs crates/db_view/src/import_export/table_import_view.rs
git commit -m "fix(db-view): run import export tasks on Tokio"
```

### Task 5: Verification, review, and delivery gate

**Files:**
- Verify all files changed in Tasks 1-4.

- [x] **Step 1: Run targeted unit tests**

```bash
rtk cargo test -p db runtime_contract --lib
rtk cargo test -p db_view import_export --lib
```

Expected: all targeted tests pass with zero failures.

- [x] **Step 2: Run crate checks and lint**

```bash
rtk cargo check -p db -p db_view -p main
rtk cargo clippy -p db -p db_view -p main --all-targets -- -D warnings
```

Expected: all crates compile and clippy reports no warnings.

Result: `cargo check -p db -p db_view -p main` succeeds. Strict workspace-path
Clippy with `-D warnings` is blocked by pre-existing warnings in untouched code,
including `agent_runtime`, `ui`, and older `db` code. Normal scoped Clippy exits
successfully, with no diagnostics in the import/export lines changed by this plan.

- [x] **Step 3: Run formatting and diff validation**

```bash
rtk cargo fmt --all -- --check
rtk git diff --check
rtk git status --short
```

Expected: formatting and diff checks pass; status lists only the intentional design, plan, implementation, and test changes.

- [x] **Step 4: Review against the acceptance criteria**

Confirm from the diff and searches:

- All three UI paths use `GlobalDbState` task APIs.
- Only `GlobalDbState` selects `Tokio::spawn_result` for these operations.
- Runtime-bound core methods are private and guarded.
- Unsafe `_sync` APIs and direct GPUI database-task spawning are absent.
- Progress loops, file writing, and result rendering have no unrelated changes.

Review result: no Critical or Important findings. One Minor finding was fixed by
making the structural regression test assert architecture-level calls instead of
depending on local variable names; the affected test passes after the correction.

- [x] **Step 5: Run completion verification and report exact evidence**

Re-run any command affected by review fixes. Report command names, exit status, test counts, and any validation that could not be executed. Do not claim issue #103 fixed without fresh successful evidence.

Final evidence: `db` reports 632 passed; `db_view` reports 421 passed and one
credential-gated MySQL integration test ignored; `cargo check` succeeds for `db`,
`db_view`, and `main`; changed-file rustfmt and `git diff --check` succeed; scoped
Clippy exits successfully with no diagnostics in touched code; unsafe API and
affected-view `background_spawn` searches return no matches. The four focused
runtime tests cover the helper inside/outside Tokio plus both private cores outside
Tokio before connection lookup.

- [x] **Step 6: Record the executor boundary in project guidance**

Add an `AGENTS.md` experience entry distinguishing pure GPUI background I/O from
Tokio-bound database futures. Document the macOS `SIGABRT` trigger, root cause,
safe `Tokio::spawn_result` ownership boundary, verification approach, and scope.

- [x] **Step 7: Directly test both runtime-bound private cores**

Add `crates/db/src/manager_runtime_contract_tests.rs` and register it as a child
module of `manager`. Poll each private core exactly once without Tokio and assert
that export and import return their operation-specific runtime errors before any
connection lookup or asynchronous suspension.
