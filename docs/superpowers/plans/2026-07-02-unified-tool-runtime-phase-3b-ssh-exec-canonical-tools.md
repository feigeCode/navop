# Unified Tool Runtime Phase 3b SSH Exec Canonical Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make existing SSH remote command runtime tools expose canonical `ssh.exec` / `ssh.command.*` ids and terminal-like `target + command` input, while preserving old `ssh.remote_*` names as aliases.

**Architecture:** Reuse the existing `public_mcp::tools::remote_ops_tool_registry` because it already implements `tool_runtime::ToolRegistry` over active SSH terminal sessions. Change only the tool descriptors, aliases, schemas, and argument parsing so Agent/MCP list canonical ids, alias calls still work, and `ssh.exec` accepts a terminal-style command string against a target resource/session.

**Tech Stack:** Rust 2024, `public_mcp`, `tool_runtime`, `agent_runtime` adapter compatibility tests.

---

## File Structure

- Modify: `crates/public_mcp/src/tools/remote_ops.rs`
  - Canonicalize remote ops tool ids.
  - Add aliases for previous names.
  - Change `ssh.exec` schema to require `target` and `command`, with `session_id` retained as compatibility input.
  - Parse target precedence `target > connection > connection_id > session_id`.
- Modify: `crates/public_mcp/tests/remote_ops.rs`
  - Assert listed tools use canonical names.
  - Assert legacy names are not listed.
  - Assert aliases still call the canonical handler.
  - Assert `ssh.exec` with `target` executes.
- Modify as needed: `crates/public_mcp/tests/protocol.rs`, `crates/public_mcp/tests/runtime.rs`, `crates/public_mcp/tests/tool_registry.rs`, `main/src/public_mcp_approval/*`
  - Update list/approval expectations only where canonical list output changes. Keep legacy call compatibility tests where useful.

## Task 1: Red Tests For Canonical SSH Tools

- [x] **Step 1: Update `remote_ops_tools_are_registered`**

Expected listed names:

```rust
assert!(names.contains(&"ssh.exec".to_string()));
assert!(names.contains(&"ssh.list_sessions".to_string()));
assert!(names.contains(&"ssh.session_diagnostics".to_string()));
assert!(names.contains(&"ssh.command.poll".to_string()));
assert!(names.contains(&"ssh.command.output".to_string()));
assert!(names.contains(&"ssh.command.cancel".to_string()));
assert!(!names.contains(&"ssh.remote_exec".to_string()));
assert!(!names.contains(&"ssh.remote_command_poll".to_string()));
assert!(!names.contains(&"ssh.remote_command_output".to_string()));
assert!(!names.contains(&"ssh.remote_command_cancel".to_string()));
```

- [x] **Step 2: Add `ssh_exec_schema_uses_terminal_target_input`**

Assert listed `ssh.exec` requires `["target", "command"]`, includes optional `session_id`, and does not require `session_id`.

- [x] **Step 3: Add call tests**

Add one test that calls runtime registry with canonical `ssh.exec` and `{ "target": "ssh-1", "command": "pwd" }`.

Add one test that calls alias `ssh.remote_exec` with `{ "session_id": "ssh-1", "command": "pwd" }`.

- [x] **Step 4: Run red tests**

Run:

```bash
rtk cargo test -p public_mcp --test remote_ops remote_ops_tools_are_registered ssh_exec_schema_uses_terminal_target_input ssh_exec_accepts_target_argument ssh_remote_exec_alias_still_accepts_session_id
```

Expected: fails because descriptors and schema still use old `ssh.remote_*` names and require `session_id`.

## Task 2: Canonicalize Remote Ops Runtime Descriptors

- [x] **Step 1: Add aliases to `RemoteOpsToolSpec`**

Add `aliases: &'static [&'static str]` and implement `ToolHandler::aliases`.

- [x] **Step 2: Change canonical ids**

Map:

```text
ssh.remote_exec -> ssh.exec
ssh.remote_command_poll -> ssh.command.poll
ssh.remote_command_output -> ssh.command.output
ssh.remote_command_cancel -> ssh.command.cancel
```

Keep aliases:

```text
ssh.remote_exec
ssh.remote_command_poll
ssh.remote_command_output
ssh.remote_command_cancel
```

- [x] **Step 3: Update dispatch matches**

Dispatch on canonical ids and keep alias resolution in `tool_runtime::ToolRegistry`.

- [x] **Step 4: Update `ssh.exec` schema**

Use `target` and `command` as required fields. Keep `session_id`, `connection`, and `connection_id` optional compatibility fields.

- [x] **Step 5: Update argument parsing**

Resolve target with precedence:

```text
target > connection > connection_id > session_id
```

Use the resolved target as the active session id for the existing `PublicMcpRegistry::remote_exec` call.

- [x] **Step 6: Run green remote ops tests**

Run:

```bash
rtk cargo test -p public_mcp --test remote_ops
```

Expected: all remote ops tests pass.

## Task 3: Update Integration Expectations

- [x] **Step 1: Search old listed names**

Run:

```bash
rtk rg -n "ssh\\.remote_exec|ssh\\.remote_command_poll|ssh\\.remote_command_output|ssh\\.remote_command_cancel" crates/public_mcp/tests main/src/public_mcp_approval
```

- [x] **Step 2: Update canonical list expectations**

Where tests inspect `tools/list` output or Agent exposed tool names, expect `ssh.exec` / `ssh.command.*`.

- [x] **Step 3: Preserve alias call tests**

Where tests call `ssh.remote_exec`, keep or add expectations proving legacy calls still resolve.

- [x] **Step 4: Run targeted integration tests**

Run:

```bash
rtk cargo test -p public_mcp --test protocol
rtk cargo test -p public_mcp --test runtime
rtk cargo test -p public_mcp --test tool_registry
rtk cargo test -p main public_mcp_approval
rtk cargo test -p agent_runtime --test tool_runtime_adapter
```

Expected: targeted tests pass.

## Task 4: Verification And Commit

- [x] **Step 1: Format**

Run:

```bash
rtk cargo fmt -p public_mcp -p main -p agent_runtime
```

- [x] **Step 2: Compile checks**

Run:

```bash
rtk cargo check -p public_mcp
rtk cargo check -p main
```

- [x] **Step 3: Commit Phase 3b**

Run:

```bash
rtk git add crates/public_mcp main docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-3b-ssh-exec-canonical-tools.md
rtk git commit -m "feat(public_mcp): canonicalize ssh command tools"
```

Expected: commit contains canonical SSH command tool ids, alias compatibility, tests, and plan progress.

## Plan Self-Review

Spec coverage:

1. Agent/MCP should derive from canonical runtime ids.
2. SSH command surface should match terminal input with `target + command`.
3. Old names remain callable as aliases.
4. No UI, approval card redesign, or multi-resource execution is included in this phase.

Marker scan:

1. No placeholder implementation steps remain.
2. Each code-changing task has concrete tests and commands.

Type consistency:

1. Alias support uses existing `tool_runtime::ToolAlias`.
2. Public MCP listing uses `ToolRegistry::list`, so only canonical ids are listed.
3. Public MCP calls use `ToolRegistry::get/call`, so aliases remain callable.
