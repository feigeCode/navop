# Unified Tool Runtime Phase 4 Public MCP Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the app Public MCP runtime build one unified `tool_runtime::ToolRegistry` per enabled toolset group, then expose it through `ToolRuntimeMcpProvider`.

**Architecture:** Keep `public_mcp` as the MCP protocol adapter and `tool_runtime` as the execution catalog. `main/src/public_mcp_runtime/tool_registry.rs` should collect enabled tool runtime registries, merge them, and create a single `ToolRuntimeMcpProvider`; legacy provider traits remain only as adapter scaffolding during migration. The first checkpoint fixes the real terminal path so the terminal toolset exposes both structured SSH tools and the new live `terminal.exec` tool.

**Tech Stack:** Rust 2024, `tool_runtime`, `public_mcp`, `terminal_view`, GPUI tests.

---

## Tracking Status

Last updated: 2026-07-02

Current status: App runtime registry merge, runtime-backed permission policy, and settings profile wording checkpoints verified.

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

## Task 3: Runtime-backed MCP Tools Use Unified PermissionPolicy

- [x] **Step 1: Write failing permission mapping test**

Add `permission_modes_map_to_unified_runtime_profiles` in `crates/public_mcp/tests/permissions.rs`.

Expected red result:

```text
no `permission_policy_for_mode` in `permissions`
```

- [x] **Step 2: Write failing high-risk runtime tool test**

Add `tool_runtime_provider_asks_for_high_risk_tools_in_allow_mode` in
`crates/public_mcp/tests/tool_runtime_adapter.rs`.

Expected before implementation: high-risk runtime tool runs directly in `PermissionMode::Allow`
without recording an approval request.

- [x] **Step 3: Add compatibility mapping**

Add:

```rust
PermissionMode::Deny  -> PermissionProfile::Safe
PermissionMode::Ask   -> PermissionProfile::Confirm
PermissionMode::Allow -> PermissionProfile::Auto
```

- [x] **Step 4: Use PermissionPolicy in ToolRuntimeMcpProvider**

Runtime-backed MCP calls now use:

```rust
permission_policy_for_mode(context.permission_mode)
    .decide(&descriptor.tool_id(), None, &call_annotations)
```

This keeps read-only calls automatic, allows low/medium mutating calls in Auto, and asks for
high-risk/destructive/open-world calls.

- [x] **Step 5: Update legacy expectations**

Update tests that previously treated `PermissionMode::Allow` as unrestricted for `ssh.exec`
and `redis.execute_command`. These tools are high-risk/open-world or mutating, so `Allow`
now maps to Auto and still asks for approval.

- [x] **Step 6: Verify**

Run:

```bash
rtk cargo test -p public_mcp
rtk cargo test -p main public_mcp_runtime
rtk cargo check -p public_mcp
rtk cargo check -p main
```

Expected: all pass. Existing `block v0.1.6` future-incompat warning can remain.

## Task 4: Settings Use Unified Permission Profile Wording

- [x] **Step 1: Write failing core settings test**

Add `mcp_permission_modes_expose_unified_profile_ids` in `crates/core/src/settings.rs`.

Expected red result:

```text
no method named `profile_id` found for enum `McpPermissionMode`
```

- [x] **Step 2: Add compatibility profile ids**

Keep persisted values unchanged, but expose unified profile ids:

```text
deny  -> safe
ask   -> confirm
allow -> auto
```

- [x] **Step 3: Expose profile id in runtime config**

`PublicMcpStartConfig` keeps `permission_mode` for protocol compatibility and adds
`permission_profile` for unified status/product semantics.

- [x] **Step 4: Update settings UI labels**

The dropdown still saves `deny`, `ask`, and `allow`, but displays:

```text
Safe
Confirm
Auto
```

This avoids a storage migration while moving the product concept away from raw MCP
deny/ask/allow.

- [x] **Step 5: Verify**

Run:

```bash
rtk cargo test -p one-core mcp_permission_modes_expose_unified_profile_ids
rtk cargo test -p main runtime_config_reads_global_mcp_settings
rtk cargo test -p main mcp_permission_mode_options_match_persisted_values
rtk cargo test -p one-core settings
rtk cargo test -p main public_mcp_runtime
rtk cargo check -p main
```

Expected: all pass. Existing `block v0.1.6` future-incompat warning can remain.

## Out Of Scope

1. Removing `PublicMcpToolProvider`.
2. Removing `PermissionMode` from settings/protocol storage. It is now a compatibility input that
   maps to `PermissionPolicy` for runtime-backed tools and to profile wording in settings UI.
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
