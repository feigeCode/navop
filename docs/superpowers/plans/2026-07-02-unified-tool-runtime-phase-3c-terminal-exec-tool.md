# Unified Tool Runtime Phase 3c Terminal Exec Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new `terminal.exec` tool that executes through an existing visible terminal session, while preserving the current structured SSH tools and all legacy aliases.

**Architecture:** Keep `ssh.exec` as the structured non-interactive SSH command tool. Add a separate terminal execution bridge backed by `terminal_view`/`PublicMcpRegistry` terminal sessions. The new tool writes the command into the live terminal PTY, submits it, and returns an observed output snapshot or submission status without pretending to have a structured exit code when the terminal cannot prove one.

**Tech Stack:** Rust 2024, `tool_runtime`, `public_mcp`, `terminal_view`, `ai_chat_view`, GPUI tests where needed.

---

## Decision Record

The user explicitly wants the "terminal execution effect":

1. The command appears in the terminal like a user typed it.
2. The terminal pane shows the command output from the same session.
3. Existing structured command tools must remain available.

Therefore:

1. Do not mutate `ssh.exec` into a terminal-surface tool.
2. Keep `ssh.exec` / `ssh.remote_exec` / `ssh.command.*` as structured remote execution tools.
3. Add `terminal.exec` for live terminal execution.
4. Agent prompt should prefer `terminal.exec` only when the user asks for visible terminal execution, side-panel current terminal execution, or "like typing in the terminal".

## Tool Contract

Canonical id:

```text
terminal.exec
```

Input schema:

```json
{
  "type": "object",
  "properties": {
    "target": {
      "type": "string",
      "description": "Active terminal resource id, label, or alias."
    },
    "command": {
      "type": "string",
      "description": "Command text to insert into the terminal."
    },
    "submit": {
      "type": "boolean",
      "default": true,
      "description": "When true, press Enter after inserting the command."
    },
    "wait_for_output": {
      "type": "boolean",
      "default": true,
      "description": "When true, wait for a bounded terminal output delta."
    },
    "timeout_ms": {
      "type": ["integer", "null"],
      "default": 60000
    }
  },
  "required": ["target", "command"]
}
```

Output schema shape:

```json
{
  "target": "terminal-ssh-prod-a",
  "command": "df -h",
  "submitted": true,
  "completion": "observed_output | shell_integration_exit | submitted_only | timed_out",
  "exit_code": null,
  "output": "...",
  "duration_ms": 1200
}
```

Rules:

1. `target` resolves only to `ResourceKind::Terminal`.
2. `command` must be sent to the terminal input unchanged.
3. `submit=true` appends Enter; `submit=false` stages the text without execution.
4. If shell integration can prove exit status, set `completion=shell_integration_exit` and include `exit_code`.
5. If shell integration is not available, return `observed_output`, `submitted_only`, or `timed_out`; do not invent `exit_code`.
6. Permission risk is `High` and `open_world=true`.

## File Structure

- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Track `terminal.exec` as an additive terminal-surface tool.
- Modify/Create: `crates/public_mcp/src/tools/terminal_exec.rs`
  - Runtime tool descriptor, schema, alias-free canonical tool handler.
- Modify: `crates/public_mcp/src/tools.rs`
  - Export terminal exec registry/provider if housed in `public_mcp`.
- Modify: `crates/public_mcp/src/tools/registry.rs`
  - Include `terminal.exec` alongside terminal tool registry when terminal execution provider is available.
- Modify: `crates/public_mcp/src/registry.rs`
  - Add a terminal execution trait or extend terminal session handle only if a minimal injection/output contract can be tested without UI.
- Modify: `terminal_view` public MCP bridge files
  - Register live terminal sessions with a `TerminalExecSessionHandle` capable of writing input and observing output.
- Modify: `crates/ai_chat_view` prompt/resource wiring if needed
  - Prefer `terminal.exec` when current side-panel terminal is the default target and the requested behavior implies visible terminal execution.
- Tests:
  - `crates/public_mcp/tests/terminal_exec.rs`
  - Existing `public_mcp` protocol tests
  - Targeted `terminal_view` bridge tests if a fake terminal session abstraction exists

## Task 1: Contract Tests For `terminal.exec`

- [ ] **Step 1: Add failing registry/list test**

Create `crates/public_mcp/tests/terminal_exec.rs` with a fake terminal execution handle. Assert:

```rust
assert!(names.contains(&"terminal.exec".to_string()));
```

Expected red result: `terminal.exec` is not registered.

- [ ] **Step 2: Add failing descriptor test**

Assert:

```rust
assert_eq!(json!(["target", "command"]), tool.input_schema["required"]);
assert_eq!("string", tool.input_schema["properties"]["target"]["type"]);
assert_eq!("string", tool.input_schema["properties"]["command"]["type"]);
assert_eq!(false, tool.annotations.read_only);
assert_eq!(true, tool.annotations.open_world);
```

Expected red result: descriptor does not exist.

- [ ] **Step 3: Add failing execution test**

Call:

```json
{
  "target": "terminal-1",
  "command": "df -h",
  "submit": true,
  "wait_for_output": true
}
```

Fake handle should record inserted text exactly as `df -h\n` and return a known output delta.

Expected red result: unknown tool.

## Task 2: Add Runtime Handler Without UI Dependency

- [ ] **Step 1: Define terminal execution trait**

Add a small trait in `public_mcp::registry` or a nearby module:

```rust
pub trait TerminalExecSessionHandle: Send + Sync + 'static {
    fn snapshot(&self) -> TerminalSessionSnapshot;
    fn exec_in_terminal(&self, request: TerminalExecRequest) -> anyhow::Result<TerminalExecResult>;
}
```

Keep this separate from `RemoteOpsSessionHandle` so structured SSH execution and live terminal execution remain distinct.

- [ ] **Step 2: Define request/result types**

Use fields from the tool contract:

```rust
pub struct TerminalExecRequest {
    pub target: String,
    pub command: String,
    pub submit: bool,
    pub wait_for_output: bool,
    pub timeout_ms: Option<u64>,
}
```

Result must include completion state and optional exit code.

- [ ] **Step 3: Register fake handles in `PublicMcpRegistry`**

Add registry storage and lookup for terminal exec handles. Only expose connected terminal sessions.

- [ ] **Step 4: Implement `terminal.exec` runtime tool**

Descriptor:

```text
id = terminal.exec
title = Execute in terminal
read_only = false
open_world = true
risk = High
target = ResourceKind::Terminal required
```

- [ ] **Step 5: Run green contract tests**

Run:

```bash
rtk cargo test -p public_mcp --test terminal_exec
```

Expected: contract tests pass.

## Task 3: Wire Terminal View As Provider

- [ ] **Step 1: Locate terminal session registration**

Find where `terminal_view::public_mcp::registry(cx)` registers terminal sessions.

- [ ] **Step 2: Implement live terminal handle**

The live handle must:

1. Insert command text into the terminal PTY/input buffer.
2. Append Enter when `submit=true`.
3. Capture output delta from the same terminal buffer when possible.
4. Return `submitted_only` if reliable output capture is not available yet.

- [ ] **Step 3: Add targeted fake/contract tests**

Use fake terminal components if available. Do not require a real SSH server or GUI window for unit tests.

## Task 4: Agent/UI Integration

- [ ] **Step 1: Prompt/resource language**

Update Agent prompt rules:

1. Use `terminal.exec` when user asks to execute in the visible terminal.
2. Use `ssh.exec` for structured background/non-interactive command execution.
3. Never claim a `terminal.exec` result has an exit code unless the result contains one.

- [ ] **Step 2: Tool cards**

Display:

```text
terminal.exec
target: <terminal label>
command: <exact command>
completion: <state>
```

If output exists, show terminal-style monospace output in the tool card.

- [ ] **Step 3: Approval details**

Approval card must show the exact command and target terminal before writing into the live terminal.

## Task 5: Verification

- [ ] **Step 1: Targeted tests**

Run:

```bash
rtk cargo test -p public_mcp --test terminal_exec
rtk cargo test -p public_mcp
rtk cargo test -p main public_mcp_runtime
rtk cargo test -p agent_runtime
```

- [ ] **Step 2: Compile checks**

Run:

```bash
rtk cargo check -p public_mcp
rtk cargo check -p main
rtk cargo check -p ai_chat_view
```

- [ ] **Step 3: Manual smoke after UI wiring**

Scenario:

1. Open an SSH terminal tab.
2. Ask Agent to run `df -h` in the terminal.
3. Verify the command appears in the terminal pane.
4. Verify terminal output appears in that same pane.
5. Verify Agent tool card says `terminal.exec` and shows the same command/output summary.

## Out Of Scope

1. Removing `ssh.exec`.
2. Removing `ssh.remote_exec`.
3. Replacing structured command output with terminal-only output.
4. Multi-terminal fan-out execution.
5. Reliable exit code support when shell integration is unavailable.

## Plan Self-Review

Spec coverage:

1. Adds a new tool instead of mutating existing SSH tools.
2. Preserves previous tools and aliases.
3. Tracks the terminal execution effect requested by the user.
4. Separates structured SSH command execution from live terminal execution.

Marker scan:

1. No placeholder tasks remain.
2. Risky unknowns are explicit and bounded by result states.

Type consistency:

1. `terminal.exec` targets `ResourceKind::Terminal`.
2. `ssh.exec` remains the structured SSH command tool.
3. `TerminalExecSessionHandle` is separate from `RemoteOpsSessionHandle`.
