# Unified Tool Runtime Phase 3a Agent DB Read Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Start business-tool migration by letting Agent DB read tools derive from `tool_runtime` canonical `db.query` / `db.schema`, while keeping existing Agent DB write and table helper tools available.

**Architecture:** Add a read-only database runtime registry in `onetcli_runtime::database_tools`, then bridge that registry into `agent_runtime::ToolRegistry` through the Phase 2b adapter. Because Agent function names are sanitized, runtime canonical `db.query` still appears to the model as `db_query`; this preserves the existing callable name while changing its backing execution path to the unified Tool Runtime.

**Tech Stack:** Rust 2024, `main`, `onetcli_runtime`, `agent_runtime`, `tool_runtime`, GPUI test support.

---

## File Structure

- Modify: `crates/agent_runtime/src/tools/registry.rs`
  - Adds a safe `extend` method to merge another Agent tool registry.
- Modify: `crates/onetcli_runtime/src/database_tools.rs`
  - Adds `database_read_tool_registry(repo)` returning only `db.schema` and `db.query`.
- Modify: `crates/onetcli_runtime/tests/database_tools.rs`
  - Adds test proving read registry excludes `db.exec`.
- Modify: `main/src/public_mcp_runtime.rs`
  - Registers legacy Agent DB tools, then extends with runtime bridged read DB tools so `db_query` is backed by `db.query`.
- Modify: `main/src/public_mcp_runtime/agent_db_registry_tests.rs`
  - Updates Agent DB registry expectations for runtime-backed `db_query` and canonical `db_schema`.

## Task 1: Add Read-Only Database Runtime Registry

**Files:**
- Modify: `crates/onetcli_runtime/src/database_tools.rs`
- Modify: `crates/onetcli_runtime/tests/database_tools.rs`

- [ ] **Step 1: Add failing read registry test**

Append to `crates/onetcli_runtime/tests/database_tools.rs`:

```rust
#[test]
fn database_read_tool_registry_exposes_only_schema_and_query() {
    let registry = onetcli_runtime::database_tools::database_read_tool_registry(repo());
    let tools = registry.list(ToolAdapter::FunctionCalling);
    let ids = tools.iter().map(|tool| tool.id.clone()).collect::<Vec<_>>();

    assert_eq!(vec!["db.schema".to_string(), "db.query".to_string()], ids);
    assert!(tools.iter().all(|tool| tool.annotations.read_only));
}
```

- [ ] **Step 2: Verify read registry test fails**

Run:

```bash
rtk cargo test -p onetcli_runtime --test database_tools database_read_tool_registry_exposes_only_schema_and_query
```

Expected: compile failure because `database_read_tool_registry` does not exist.

- [ ] **Step 3: Implement `database_read_tool_registry`**

Add to `crates/onetcli_runtime/src/database_tools.rs` next to `database_tool_registry`:

```rust
pub fn database_read_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(DatabaseToolHandler::new(repo.clone(), DatabaseTool::Schema)),
        Arc::new(DatabaseToolHandler::new(repo, DatabaseTool::Query)),
    ])
}
```

- [ ] **Step 4: Verify read registry test passes**

Run:

```bash
rtk cargo test -p onetcli_runtime --test database_tools database_read_tool_registry_exposes_only_schema_and_query
```

Expected: test passes.

## Task 2: Allow Agent ToolRegistry Extension

**Files:**
- Modify: `crates/agent_runtime/src/tools/registry.rs`
- Optional test: covered through main registry behavior in Task 3.

- [ ] **Step 1: Implement `ToolRegistry::extend`**

Add to `impl ToolRegistry`:

```rust
pub fn extend(&mut self, other: ToolRegistry) -> &mut Self {
    for (name, tool) in other.tools {
        self.tools.insert(name, tool);
    }
    self
}
```

This intentionally allows later registries to override earlier tools by `ToolName`. Phase 3a depends on this to replace legacy `db_query` with runtime-backed `db.query` while preserving the same Agent function name.

## Task 3: Register Runtime-Backed DB Read Tools For Agent

**Files:**
- Modify: `main/src/public_mcp_runtime.rs`
- Modify: `main/src/public_mcp_runtime/agent_db_registry_tests.rs`

- [ ] **Step 1: Add failing Agent DB registry expectations**

Update `agent_runtime_tool_registry_uses_native_database_tools` to assert:

```rust
assert!(names.contains(&"db_query".to_string()));
assert!(names.contains(&"db_schema".to_string()));
assert!(names.contains(&"db_execute_sql".to_string()));
assert!(names.contains(&"db_list_tables".to_string()));
assert!(
    !names.contains(&"db_exec".to_string()),
    "Agent registry must not expose write-capable db.exec through runtime bridge in Phase 3a"
);
```

Then add a spec assertion:

```rust
let registry = cx.update(|cx| {
    register_connection_repository(cx);
    let mut settings = AppSettings::default();
    settings.mcp.toolsets = McpToolsetSettings {
        terminal: false,
        connections: false,
        database: true,
        ..Default::default()
    };
    cx.set_global(settings);
    agent_runtime_tool_registry(cx).expect("agent registry should build")
});

let db_query = registry
    .get(&ToolName::new("db.query"))
    .expect("runtime-backed db.query should be registered");
let spec = db_query.spec(&ResourceContext::new());
assert_eq!(RiskLevel::Read, spec.risk);
assert!(
    spec.description.contains("Run read-only SQL through a saved database connection"),
    "db_query should be backed by tool_runtime db.query descriptor"
);
assert_eq!(serde_json::json!(["connection", "sql"]), spec.parameters["required"]);
```

- [ ] **Step 2: Verify Agent DB registry test fails**

Run:

```bash
rtk cargo test -p main agent_runtime_tool_registry_uses_native_database_tools
```

Expected: failure because `db_schema` is not registered and `db_query` still uses the old native Agent descriptor.

- [ ] **Step 3: Register runtime read bridge after legacy DB tools**

Modify `main/src/public_mcp_runtime.rs` inside `if agent_database_enabled`:

```rust
onetcli_runtime::agent_db_tools::register_agent_db_tools(repo.clone(), &mut agent_registry);
let runtime_db_read_registry = onetcli_runtime::database_tools::database_read_tool_registry(repo);
let runtime_agent_db_read_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
    runtime_db_read_registry,
    tool_runtime::ToolAdapter::FunctionCalling,
);
agent_registry.extend(runtime_agent_db_read_registry);
```

Registering the runtime bridge after legacy tools means `ToolName::new("db.query")` replaces old `db_query`, while old write/table helper tools remain.

- [ ] **Step 4: Verify Agent DB registry test passes**

Run:

```bash
rtk cargo test -p main agent_runtime_tool_registry_uses_native_database_tools
```

Expected: test passes.

## Task 4: Full Phase 3a Verification

**Files:**
- Modified files above.

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt -p agent_runtime -p onetcli_runtime -p main
```

Expected: formatting completes.

- [ ] **Step 2: Run targeted tests**

Run:

```bash
rtk cargo test -p onetcli_runtime --test database_tools
rtk cargo test -p main agent_runtime_tool_registry_uses_native_database_tools
rtk cargo test -p agent_runtime --test tool_runtime_adapter
```

Expected: all targeted tests pass.

- [ ] **Step 3: Compile integration crates**

Run:

```bash
rtk cargo check -p main
rtk cargo check -p ai_chat_view
rtk cargo check -p public_mcp
```

Expected: all compile.

- [ ] **Step 4: Commit Phase 3a**

Run:

```bash
rtk git add crates/agent_runtime crates/onetcli_runtime main docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-3a-agent-db-read-tools.md
rtk git commit -m "feat(agent): bridge database read tools through tool runtime"
```

Expected: commit includes Phase 3a code, tests, and plan.

## Plan Self-Review

Spec coverage:

1. Starts business tool migration with DB read tools only.
2. Keeps write-capable `db.exec` out of the runtime bridge for Agent in this phase.
3. Preserves old Agent helper tools such as `db_execute_sql` and `db_list_tables`.
4. Uses the Phase 2b registry bridge, proving Agent derives DB read specs from `tool_runtime`.

Marker scan:

1. No unfinished markers are present.
2. Each task has concrete files, code, and verification commands.

Type consistency:

1. `database_read_tool_registry` returns `tool_runtime::ToolRegistry`.
2. `tool_runtime_agent_tool_registry` converts it to `agent_runtime::ToolRegistry`.
3. `ToolRegistry::extend` resolves the `db_query` name collision by letting runtime-backed read tools override legacy tools.
