# Unified Tool Runtime Phase 4 Public MCP Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the app Public MCP runtime build one unified `tool_runtime::ToolRegistry` per enabled toolset group, then expose it through `ToolRuntimeMcpProvider`.

**Architecture:** Keep `public_mcp` as the MCP protocol adapter and `tool_runtime` as the execution catalog. `main/src/public_mcp_runtime/tool_registry.rs` should collect enabled tool runtime registries, merge them, and create a single `ToolRuntimeMcpProvider`; legacy provider traits remain only as adapter scaffolding during migration. The first checkpoint fixes the real terminal path so the terminal toolset exposes both structured SSH tools and the new live `terminal.exec` tool.

**Tech Stack:** Rust 2024, `tool_runtime`, `public_mcp`, `terminal_view`, GPUI tests.

---

## Tracking Status

Last updated: 2026-07-02

Current status: App runtime registry merge checkpoint verified.

## File Structure

- Modify: `main/src/public_mcp_runtime/tool_registry.rs`
  - Collect `tool_runtime::ToolRegistry` values instead of pushing many `ToolRuntimeMcpProvider` values.
  - Merge collected registries with `tool_runtime::ToolRegistry::merge`.
  - Expose the merged registry through one `ToolRuntimeMcpProvider`.
  - Include both `remote_ops_tool_registry` and `terminal_exec_tool_registry` when terminal tools are enabled.
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Update Phase 4 tracking when the checkpoint is verified.
- Modify: this plan file
  - Mark completed steps and record verification evidence.

## Task 1: Terminal Toolset Uses Unified Runtime Registry

- [x] **Step 1: Write the failing test**

Add a GPUI test in `main/src/public_mcp_runtime/tool_registry.rs`:

```rust
#[gpui::test]
fn build_tool_registry_terminal_toolset_includes_terminal_exec(cx: &mut TestAppContext) {
    let toolsets = McpToolsetSettings {
        terminal: true,
        connections: false,
        internal_functions: false,
        ..Default::default()
    };

    let tools = cx.update(|cx| {
        terminal_view::public_mcp::init(cx);
        build_tool_registry(cx, &toolsets)
            .expect("terminal registry should build")
            .tools()
    });

    assert!(tools.iter().any(|tool| tool.name == "ssh.exec"));
    assert!(tools.iter().any(|tool| tool.name == "terminal.exec"));
}
```

- [x] **Step 2: Run the test to verify red**

Run:

```bash
rtk cargo test -p main build_tool_registry_terminal_toolset_includes_terminal_exec
```

Expected before implementation: fails because `terminal.exec` is absent from the app registry path.

Observed red result:

```text
assertion failed: tools.iter().any(|tool| tool.name == "terminal.exec")
```

- [x] **Step 3: Implement the unified registry collection**

In `main/src/public_mcp_runtime/tool_registry.rs`:

1. Import `terminal_exec_tool_registry`.
2. Replace repeated provider pushes for runtime-backed tools with a `Vec<tool_runtime::ToolRegistry>`.
3. Push `remote_ops_tool_registry(registry.clone())` and `terminal_exec_tool_registry(registry)` for the terminal toolset.
4. At the end, merge the registries and create one `ToolRuntimeMcpProvider`.
5. Keep the existing empty-provider warning behavior.

- [x] **Step 4: Run the terminal toolset test green**

Run:

```bash
rtk cargo test -p main build_tool_registry_terminal_toolset_includes_terminal_exec
```

Expected: passes.

## Task 2: Regression Verification

- [x] **Step 1: Run existing Public MCP runtime tests**

Run:

```bash
rtk cargo test -p main public_mcp_runtime
```

Expected: existing app registry behavior remains compatible.

- [x] **Step 2: Run Public MCP crate tests**

Run:

```bash
rtk cargo test -p public_mcp
```

Expected: protocol adapter and runtime provider contracts still pass.

- [x] **Step 3: Run compile checks**

Run:

```bash
rtk cargo check -p main
rtk cargo check -p public_mcp
```

Expected: no new compile errors. Existing `block v0.1.6` future-incompat warning can remain.

- [x] **Step 4: Commit**

Run:

```bash
rtk git add main/src/public_mcp_runtime/tool_registry.rs docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-4-public-mcp-adapter.md
rtk git commit -m "feat(public_mcp): merge app runtime tool registries"
```

Verification completed:

```bash
rtk cargo test -p main build_tool_registry_terminal_toolset_includes_terminal_exec
rtk cargo test -p main public_mcp_runtime
rtk cargo test -p public_mcp
rtk cargo check -p main
rtk cargo check -p public_mcp
```

Known warning:

```text
block v0.1.6 future-incompat warning
```

## Out Of Scope

1. Removing `PublicMcpToolProvider`.
2. Replacing `PermissionMode` with full `PermissionPolicy`.
3. Rewriting Public MCP approval UI.
4. Migrating external MCP or ACP providers.
5. Manual visible terminal smoke for Phase 3c.

## Plan Self-Review

Spec coverage:

1. Moves the app Public MCP registry toward one runtime catalog.
2. Keeps MCP as an adapter over runtime descriptors.
3. Fixes the live terminal execution tool exposure through the real app registry path.

Marker scan:

1. No placeholder tasks remain.
2. Deferred broader Phase 4 work is explicit in Out Of Scope.

Type consistency:

1. Uses existing `tool_runtime::ToolRegistry::merge`.
2. Uses existing `ToolRuntimeMcpProvider`.
3. Uses existing `terminal_exec_tool_registry` and `remote_ops_tool_registry`.
