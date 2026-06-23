# Public MCP Structured Remote Ops Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade Public MCP from terminal-only SSH control to a structured remote operations channel that can run commands, track long jobs, write remote scripts safely, and still preserve the existing interactive terminal bridge.

**Architecture:** Replace the legacy terminal paste/snapshot tools with a structured remote ops provider backed by active SSH sessions. The provider returns structured command results with stdout, stderr, exit code, timing, and command state, while long-running commands are tracked in an in-process command registry with polling and cancellation. File write and session diagnostics share the same permission, approval, and audit path. The legacy `terminal_snapshot` and `terminal_write` tools are removed entirely; `list_sessions` is migrated to the remote ops provider.

**Tech Stack:** Rust, rmcp, public_mcp, terminal_view, terminal, ssh, sftp, tokio, serde_json

---

## Context

The current Public MCP surface is intentionally small:

- `public_mcp.list_sessions`
- `public_mcp.terminal_snapshot`
- `public_mcp.terminal_write`

This works for lightweight interactive operations, but it is not reliable enough for heavy remote ops such as moving `/var/lib/docker` to `/data/docker`, stopping services, running long copy jobs, or executing multi-line scripts. The primary gaps are:

- terminal writes return only `{ "ok": true }`
- snapshots expose only visible terminal text
- command exit code and completion state are unavailable
- long-running commands cannot be polled or cancelled structurally
- large script input is pasted into an interactive shell instead of uploaded and executed
- unavailable session errors do not provide recovery state
- destructive operations have only coarse write-terminal approval

This plan adds a structured path for automation while keeping raw terminal control for interactive recovery.

---

## File Structure

### Existing Files To Modify

- `crates/public_mcp/src/permissions.rs`
  - Add operation kinds for remote command execution, command cancellation, and remote file writes.

- `crates/public_mcp/src/registry.rs`
  - Extend the registered session handle contract or add a sibling remote-session contract for structured SSH operations.
  - Preserve existing terminal snapshot/write behavior.

- `crates/public_mcp/src/tools/registry.rs`
  - Register the new remote ops tool provider alongside the existing terminal provider.

- `crates/public_mcp/src/tools/terminal.rs`
  - Keep existing tools compatible.
  - Optionally extend snapshot arguments for scrollback without changing default visible-text behavior.

- `crates/public_mcp/src/tools/mod.rs`
  - Export the new remote ops provider module.

- `crates/public_mcp/src/runtime.rs`
  - Start the MCP runtime with both terminal and remote ops providers.

- `crates/terminal_view/src/public_mcp.rs`
  - Bridge active SSH terminal sessions to the new structured operations.
  - Add command requests for exec, poll, cancel, and file write.

- `crates/terminal_view/src/view.rs`
  - Handle new Public MCP commands on the GPUI thread where terminal/session state is owned.

- `docs/design/agent-cli-extension.md`
  - Document the new remote ops tools and explain the boundary between terminal bridge and structured execution.

### New Files To Create

- `crates/public_mcp/src/tools/remote_ops.rs`
  - MCP tool definitions and argument/result serialization for structured remote operations.

- `crates/public_mcp/src/remote_ops.rs`
  - Shared remote ops data models such as command state, output chunks, file write result, and session diagnostic result.

- `crates/public_mcp/tests/remote_ops.rs`
  - Protocol-level tests for tool schemas, permission behavior, successful calls, validation errors, polling, cancellation, and file write results.

- `crates/terminal_view/src/public_mcp_remote_ops.rs`
  - UI/session-side command registry and adapters from active SSH sessions to remote ops execution.

---

## Tool Surface

### Tool Surface

`list_sessions` is migrated to the remote ops provider. The legacy `terminal_snapshot` and `terminal_write` tools are **removed** to avoid misleading AI into using terminal paste as an execution channel. The structured remote ops tools are now the only execution path:

```text
public_mcp.list_sessions
```

### Structured Tools

Add these names:

```text
public_mcp.remote_exec
public_mcp.remote_command_poll
public_mcp.remote_command_output
public_mcp.remote_command_cancel
public_mcp.remote_file_write
public_mcp.session_diagnostics
```

The names intentionally stay under `public_mcp.*` for compatibility with the current provider style. A later `tool_runtime` migration can alias them to names such as `remote.exec`, `remote.command.poll`, and `remote.file.write`.

---

## Result Shapes

### `public_mcp.remote_exec`

Arguments:

```json
{
  "session_id": "ssh-terminal-123-...",
  "command": "docker info --format '{{.DockerRootDir}}'",
  "cwd": "/root",
  "env": {
    "SYSTEMD_PAGER": "cat",
    "PAGER": "cat",
    "LESS": "-F -X"
  },
  "timeout_ms": 30000,
  "mode": "foreground"
}
```

Foreground result:

```json
{
  "status": "exited",
  "stdout": "/var/lib/docker\n",
  "stderr": "",
  "exit_code": 0,
  "duration_ms": 128,
  "timed_out": false
}
```

Background result:

```json
{
  "status": "running",
  "command_id": "cmd_01j...",
  "started_at_ms": 1782150000000
}
```

### `public_mcp.remote_command_poll`

Arguments:

```json
{
  "command_id": "cmd_01j..."
}
```

Result:

```json
{
  "command_id": "cmd_01j...",
  "status": "running",
  "exit_code": null,
  "duration_ms": 42100,
  "stdout_bytes": 16384,
  "stderr_bytes": 0
}
```

### `public_mcp.remote_command_output`

Arguments:

```json
{
  "command_id": "cmd_01j...",
  "stdout_offset": 0,
  "stderr_offset": 0,
  "limit_bytes": 65536
}
```

Result:

```json
{
  "command_id": "cmd_01j...",
  "stdout": "copying layer...\n",
  "stderr": "",
  "next_stdout_offset": 17,
  "next_stderr_offset": 0,
  "truncated": false
}
```

### `public_mcp.remote_command_cancel`

Arguments:

```json
{
  "command_id": "cmd_01j...",
  "signal": "sigint"
}
```

Result:

```json
{
  "command_id": "cmd_01j...",
  "status": "cancel_requested"
}
```

### `public_mcp.remote_file_write`

Arguments:

```json
{
  "session_id": "ssh-terminal-123-...",
  "path": "/data/onetcli-mcp/migrate-docker-root.sh",
  "content": "#!/usr/bin/env bash\nset -euo pipefail\n...",
  "mode": 493,
  "overwrite": false
}
```

Result:

```json
{
  "path": "/data/onetcli-mcp/migrate-docker-root.sh",
  "bytes_written": 42,
  "sha256": "..."
}
```

### `public_mcp.session_diagnostics`

Arguments:

```json
{
  "session_id": "ssh-terminal-123-..."
}
```

Result:

```json
{
  "session_id": "ssh-terminal-123-...",
  "connection_id": 123,
  "host_label": "性能环境",
  "state": "connected",
  "connection_kind": "ssh",
  "last_error": null,
  "recoverable": true,
  "suggested_action": null
}
```

---

### Task 1: Add Remote Ops Data Models

**Files:**

- Create: `crates/public_mcp/src/remote_ops.rs`
- Modify: `crates/public_mcp/src/lib.rs`

- [ ] **Step 1: Define command mode and command status models**

Add serializable enums:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandMode {
    Foreground,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandStatus {
    Running,
    Exited,
    Failed,
    CancelRequested,
    Cancelled,
    TimedOut,
}
```

- [ ] **Step 2: Define request and result structs**

Add models for:

- `RemoteExecRequest`
- `RemoteExecResult`
- `RemoteCommandPollRequest`
- `RemoteCommandPollResult`
- `RemoteCommandOutputRequest`
- `RemoteCommandOutputResult`
- `RemoteCommandCancelRequest`
- `RemoteCommandCancelResult`
- `RemoteFileWriteRequest`
- `RemoteFileWriteResult`
- `SessionDiagnosticsRequest`
- `SessionDiagnosticsResult`

All request structs should use optional fields for optional JSON arguments. All result structs should serialize with `snake_case` field names.

- [ ] **Step 3: Export the module**

Modify `crates/public_mcp/src/lib.rs` so integration tests and runtime code can use the shared types:

```rust
pub mod remote_ops;
```

- [ ] **Step 4: Run formatting**

Run: `cargo fmt --all`

Expected: command exits 0 and formats the new module.

---

### Task 2: Add Permission Kinds For Structured Remote Ops

**Files:**

- Modify: `crates/public_mcp/src/permissions.rs`
- Modify: `crates/public_mcp/tests/permissions.rs`

- [ ] **Step 1: Add operation kinds**

Add these variants to `PublicMcpOperationKind`:

```rust
ReadSessionDiagnostics,
ExecuteRemoteCommand,
CancelRemoteCommand,
WriteRemoteFile,
ReadRemoteCommandOutput,
```

- [ ] **Step 2: Keep read-only operations allowed**

Update `decide_permission` so the following are read-only and allowed regardless of permission mode:

- `ReadTerminal`
- `ReadSessionDiagnostics`
- `ReadRemoteCommandOutput`

Remote command execution, cancellation, remote file write, terminal write, and internal function calls should continue to respect `Deny`, `Ask`, and `Allow`.

- [ ] **Step 3: Add permission tests**

Add tests that verify:

- read-only remote ops are allowed under `Deny`
- `ExecuteRemoteCommand` is denied under `Deny`
- `ExecuteRemoteCommand` asks under `Ask`
- `WriteRemoteFile` asks under `Ask`
- `CancelRemoteCommand` asks under `Ask`
- all structured write operations are allowed under `Allow`

- [ ] **Step 4: Run targeted tests**

Run: `cargo test -p public_mcp permissions -- --nocapture`

Expected: all permission tests pass.

---

### Task 3: Add Remote Ops Tool Provider

**Files:**

- Create: `crates/public_mcp/src/tools/remote_ops.rs`
- Modify: `crates/public_mcp/src/tools/mod.rs`
- Modify: `crates/public_mcp/src/tools/registry.rs`
- Modify: `crates/public_mcp/tests/tool_registry.rs`
- Create: `crates/public_mcp/tests/remote_ops.rs`

- [ ] **Step 1: Define MCP tool schemas**

Create tools for:

```text
public_mcp.remote_exec
public_mcp.remote_command_poll
public_mcp.remote_command_output
public_mcp.remote_command_cancel
public_mcp.remote_file_write
public_mcp.session_diagnostics
```

Set annotations as:

- `session_diagnostics`: read-only, non-destructive, idempotent, not open-world
- `remote_command_poll`: read-only, non-destructive, idempotent, not open-world
- `remote_command_output`: read-only, non-destructive, idempotent, not open-world
- `remote_exec`: write, potentially destructive, not idempotent, open-world
- `remote_command_cancel`: write, potentially destructive, not idempotent, open-world
- `remote_file_write`: write, potentially destructive, not idempotent, open-world

- [ ] **Step 2: Implement argument parsing**

Use typed request structs from `crates/public_mcp/src/remote_ops.rs`.

Validation rules:

- `session_id` is required where a session is needed
- `command` is required for `remote_exec`
- `mode` defaults to `foreground`
- `timeout_ms` defaults to 30 seconds for foreground commands
- `limit_bytes` defaults to 64 KiB for output reads
- `path` and `content` are required for file writes
- `overwrite` defaults to `false`

- [ ] **Step 3: Add provider dispatch**

Add `RemoteOpsToolProvider` with `tools()` and `call_tool()` implementations following the existing `TerminalToolProvider` pattern.

Read-only tools should execute directly. Write-like tools should call `decide_permission` with the new operation kinds and request approval when permission mode is `Ask`.

- [ ] **Step 4: Register the provider**

Modify `PublicMcpToolRegistry::terminal` so it registers both:

- `TerminalToolProvider`
- `RemoteOpsToolProvider`

Keep duplicate tool-name protection unchanged.

- [ ] **Step 5: Add schema and dispatch tests**

In `crates/public_mcp/tests/remote_ops.rs`, verify:

- `list_tools` includes all six new tool names
- read-only tools are callable under `PermissionMode::Deny`
- write-like tools return structured permission errors under `PermissionMode::Deny`
- `PermissionMode::Ask` creates approval requests for remote exec, cancellation, and file write

- [ ] **Step 6: Run targeted tests**

Run: `cargo test -p public_mcp remote_ops -- --nocapture`

Expected: remote ops protocol tests pass.

Run: `cargo test -p public_mcp tool_registry -- --nocapture`

Expected: tool registry tests pass and no duplicate tool names exist.

---

### Task 4: Extend Registry Contract For Structured Sessions

**Files:**

- Modify: `crates/public_mcp/src/registry.rs`
- Modify: `crates/public_mcp/tests/registry.rs`

- [ ] **Step 1: Add a structured remote ops handle**

Add a trait that can be implemented by active SSH terminal sessions:

```rust
pub trait RemoteOpsSessionHandle: Send + Sync + 'static {
    fn snapshot(&self) -> TerminalSessionSnapshot;
    fn exec(&self, request: RemoteExecRequest) -> Result<RemoteExecResult>;
    fn poll(&self, request: RemoteCommandPollRequest) -> Result<RemoteCommandPollResult>;
    fn output(&self, request: RemoteCommandOutputRequest) -> Result<RemoteCommandOutputResult>;
    fn cancel(&self, request: RemoteCommandCancelRequest) -> Result<RemoteCommandCancelResult>;
    fn write_file(&self, request: RemoteFileWriteRequest) -> Result<RemoteFileWriteResult>;
    fn diagnostics(&self, request: SessionDiagnosticsRequest) -> Result<SessionDiagnosticsResult>;
}
```

If sharing one handle with `TerminalSessionHandle` is cleaner in implementation, keep terminal methods and remote ops methods on a single registered handle, but preserve the existing terminal API.

- [ ] **Step 2: Add registry methods**

Add:

- `remote_exec`
- `remote_command_poll`
- `remote_command_output`
- `remote_command_cancel`
- `remote_file_write`
- `session_diagnostics`

The methods should:

- look up the session by `session_id` where applicable
- ensure it is an exposed connected SSH session before write-like actions
- return richer diagnostics for disconnected/unavailable sessions where possible
- keep current `list_sessions` behavior compatible

- [ ] **Step 3: Add registry tests**

Cover:

- connected SSH session accepts structured remote ops
- local and serial sessions are rejected for write-like remote ops
- disconnected SSH session returns a diagnostic result instead of only a generic error for `session_diagnostics`
- unknown session returns a clear unknown-session error
- existing `terminal_snapshot` and `terminal_write` tests still pass

- [ ] **Step 4: Run targeted tests**

Run: `cargo test -p public_mcp registry -- --nocapture`

Expected: registry tests pass.

---

### Task 5: Implement Session Diagnostics

**Files:**

- Modify: `crates/public_mcp/src/registry.rs`
- Modify: `crates/public_mcp/src/tools/remote_ops.rs`
- Modify: `crates/public_mcp/tests/remote_ops.rs`
- Modify: `crates/public_mcp/tests/registry.rs`

- [ ] **Step 1: Add diagnostic state output**

Return:

- `session_id`
- `connection_id`
- `host_label`
- `cwd`
- `rows`
- `cols`
- `connection_kind`
- `state`
- `last_error`
- `recoverable`
- `suggested_action`

For connected SSH sessions, `recoverable` should be `true` and `suggested_action` should be `null`.

For unknown sessions, return a structured error with code `unknown_session`.

For known but disconnected sessions, return `state = disconnected`, preserve the last error when available, and set `suggested_action = "reconnect_in_onetcli"`.

- [ ] **Step 2: Add tests for diagnostic recovery messages**

Test connected, disconnected, and unknown-session behavior.

- [ ] **Step 3: Run targeted tests**

Run: `cargo test -p public_mcp session_diagnostics -- --nocapture`

Expected: diagnostic tests pass.

---

### Task 6: Implement Foreground Remote Exec

**Files:**

- Modify: `crates/terminal_view/src/public_mcp.rs`
- Create: `crates/terminal_view/src/public_mcp_remote_ops.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/lib.rs`
- Modify: `crates/terminal_view/Cargo.toml` if new dependencies are needed
- Modify: `crates/public_mcp/tests/remote_ops.rs`

- [ ] **Step 1: Add terminal-view command variants**

Add Public MCP command variants for foreground exec and diagnostics. Keep the existing visible-text/write commands unchanged.

- [ ] **Step 2: Implement foreground command execution**

Execute commands through the active SSH connection instead of pasting into the visible terminal.

The command execution should:

- set default environment values:
  - `SYSTEMD_PAGER=cat`
  - `PAGER=cat`
  - `LESS=-F -X`
- respect caller-provided `cwd`
- respect caller-provided `env`
- capture stdout and stderr separately
- return exit code
- enforce `timeout_ms`
- return `timed_out = true` when timeout is exceeded

- [ ] **Step 3: Keep terminal UI behavior unchanged**

Foreground exec should not type into the visible shell and should not require the terminal prompt to be idle.

- [ ] **Step 4: Add fake-handle tests**

Use fake handles in `crates/public_mcp/tests/remote_ops.rs` to verify result serialization and permission behavior. If terminal-view has unit-testable adapters, add tests there for environment defaults and timeout mapping.

- [ ] **Step 5: Run targeted tests**

Run: `cargo test -p public_mcp remote_exec -- --nocapture`

Expected: remote exec protocol tests pass.

Run: `cargo test -p terminal_view public_mcp -- --nocapture`

Expected: terminal-view Public MCP tests pass.

---

### Task 7: Implement Background Command Registry

**Files:**

- Create or modify: `crates/terminal_view/src/public_mcp_remote_ops.rs`
- Modify: `crates/terminal_view/src/public_mcp.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/public_mcp/tests/remote_ops.rs`

- [ ] **Step 1: Add command registry state**

Track each background command by `command_id` with:

- session id
- command string
- status
- started time
- finished time
- exit code
- stdout buffer
- stderr buffer
- cancellation handle

Use bounded buffers or offset-based output storage so one command cannot grow memory without limit.

- [ ] **Step 2: Add background exec path**

When `remote_exec.mode = "background"`, start the command and immediately return:

- `status = running`
- `command_id`
- `started_at_ms`

- [ ] **Step 3: Add poll path**

`remote_command_poll` should return:

- `running` while process is active
- `exited` with `exit_code` when complete
- `failed` when setup failed
- `timed_out` when timeout kills or abandons the command
- output byte counters

- [ ] **Step 4: Add output path**

`remote_command_output` should return stdout and stderr slices from requested offsets and include next offsets.

If output is truncated because of `limit_bytes`, set `truncated = true`.

- [ ] **Step 5: Add cancellation path**

`remote_command_cancel` should support at least:

- `sigint`
- `sigterm`

If the backend cannot send a real signal yet, map cancellation to the closest supported channel close/abort behavior and return `status = cancel_requested`.

- [ ] **Step 6: Add lifecycle tests**

Test:

- background exec returns command id
- polling a running command reports running
- completed command reports exit code
- output can be read incrementally by offset
- cancelling a running command changes command state
- unknown command id returns structured error

- [ ] **Step 7: Run targeted tests**

Run: `cargo test -p terminal_view public_mcp_remote_ops -- --nocapture`

Expected: command registry tests pass.

Run: `cargo test -p public_mcp remote_command -- --nocapture`

Expected: MCP command lifecycle tests pass.

---

### Task 8: Implement Remote File Write

**Files:**

- Modify: `crates/terminal_view/src/public_mcp_remote_ops.rs`
- Modify: `crates/terminal_view/src/public_mcp.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/public_mcp/tests/remote_ops.rs`

- [ ] **Step 1: Implement safe file write**

Write `content` to the requested remote path through the active SSH/SFTP capability.

Rules:

- fail when `overwrite = false` and the file already exists
- create parent directories only when the implementation explicitly supports that behavior
- apply `mode` after write when provided
- compute and return SHA-256 of written content
- return byte count

- [ ] **Step 2: Add path risk classification**

Classify paths before writing:

- low risk: `/tmp/*`
- medium risk: `/data/*`, `/var/tmp/*`
- high risk: `/etc/*`, `/var/lib/*`, `/usr/*`, `/bin/*`, `/sbin/*`, `/root/*`

For the first implementation, high-risk paths should still be possible only through approval mode, but the approval payload must include risk classification and target path.

- [ ] **Step 3: Add file write tests**

Test:

- writes content and returns bytes plus hash
- refuses overwrite when `overwrite = false`
- accepts overwrite when `overwrite = true`
- applies mode when provided
- approval payload includes path and risk classification

- [ ] **Step 4: Run targeted tests**

Run: `cargo test -p public_mcp remote_file_write -- --nocapture`

Expected: file write protocol tests pass.

Run: `cargo test -p terminal_view public_mcp_remote_ops -- --nocapture`

Expected: terminal-view remote file tests pass.

---

### Task 9: Add Terminal Scrollback And Resize-Friendly Snapshot Support

**Files:**

- Modify: `crates/public_mcp/src/tools/terminal.rs`
- Modify: `crates/public_mcp/src/registry.rs`
- Modify: `crates/terminal_view/src/public_mcp.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/public_mcp/tests/protocol.rs`
- Modify: `crates/public_mcp/tests/registry.rs`

- [ ] **Step 1: Extend snapshot arguments compatibly**

Allow optional arguments:

```json
{
  "session_id": "ssh-terminal-123-...",
  "mode": "visible",
  "lines": 200
}
```

Defaults:

- `mode = visible`
- `lines = null`

Existing calls with only `session_id` must continue returning visible text.

- [ ] **Step 2: Add scrollback mode**

Support `mode = scrollback` when terminal-view can provide recent history.

If the terminal backend cannot provide scrollback yet, return a structured unsupported error instead of silently returning visible text.

- [ ] **Step 3: Add logical-line metadata**

Add metadata fields where available:

- `rows`
- `cols`
- `wrapped_lines`
- `mode`

Do not break existing `visible_text` consumers.

- [ ] **Step 4: Add tests**

Test:

- old snapshot arguments still work
- scrollback mode validates `lines`
- unsupported scrollback returns a structured error
- visible mode remains read-only

- [ ] **Step 5: Run targeted tests**

Run: `cargo test -p public_mcp terminal_snapshot -- --nocapture`

Expected: terminal snapshot tests pass.

---

### Task 10: Improve Approval Payloads And Audit Safety

**Files:**

- Modify: `crates/public_mcp/src/tools/remote_ops.rs`
- Modify: `crates/public_mcp/src/tools/terminal.rs`
- Modify: `main/src/public_mcp_approval/queue.rs`
- Modify: `main/src/public_mcp_approval/channel.rs`
- Modify: `main/src/public_mcp_approval/protocol_tests.rs`
- Modify: `crates/public_mcp/tests/remote_ops.rs`

- [ ] **Step 1: Add richer approval payloads**

For `remote_exec`, include:

- session id
- command preview
- cwd
- env keys
- mode
- timeout
- destructive risk classification

For `remote_file_write`, include:

- session id
- path
- bytes
- sha256
- overwrite
- mode
- path risk classification

For cancellation, include:

- command id
- original command preview when available
- signal

- [ ] **Step 2: Add simple destructive command detection**

Classify commands containing these tokens as high risk:

- `rm`
- `mkfs`
- `dd`
- `systemctl stop`
- `systemctl restart`
- `docker system prune`
- `docker rm`
- `mv /var/lib`
- `chmod -R`
- `chown -R`

This is a conservative helper for approval/audit messaging, not a security boundary.

- [ ] **Step 3: Add approval tests**

Test that approval requests include risk metadata for:

- `rm -rf /var/lib/docker.before-data-root-test`
- `systemctl stop docker`
- `remote_file_write` to `/data/onetcli-mcp/script.sh`
- `remote_file_write` to `/etc/systemd/system/example.service`

- [ ] **Step 4: Run targeted tests**

Run: `cargo test -p public_mcp remote_ops -- --nocapture`

Expected: remote ops approval tests pass.

Run: `cargo test -p main public_mcp_approval -- --nocapture`

Expected: approval bridge tests pass.

---

### Task 11: Document Public MCP Remote Ops Usage

**Files:**

- Modify: `docs/design/agent-cli-extension.md`
- Create: `docs/public-mcp-remote-ops.md`

- [ ] **Step 1: Update architecture documentation**

In `docs/design/agent-cli-extension.md`, document:

- terminal bridge remains for interactive control
- structured remote ops is the preferred path for automation
- future `tool_runtime` migration should alias tool names without breaking current MCP clients

- [ ] **Step 2: Add user-facing examples**

Create `docs/public-mcp-remote-ops.md` with examples for:

- checking Docker root dir with foreground exec
- running a long `rsync`/copy job in background
- polling output by offset
- writing a script to `/data/onetcli-mcp/*.sh`
- cancelling a long command
- diagnosing an unavailable session

- [ ] **Step 3: Add safety guidance**

Document:

- prefer `remote_exec` over `terminal_write` for automation
- prefer `remote_file_write` over heredoc for scripts
- default pager env values
- use `/data` or `/tmp` for generated scripts when root filesystem may be full
- verify before destructive cleanup

- [ ] **Step 4: Run docs check**

Run: `rg -n "public_mcp.remote_exec|remote_file_write|terminal_write" docs`

Expected: docs contain examples for the new tools and still mention terminal write as interactive bridge only.

---

### Task 12: Full Verification

**Files:**

- No source edits in this task.

- [ ] **Step 1: Run public MCP tests**

Run: `cargo test -p public_mcp -- --nocapture`

Expected: all public MCP tests pass.

- [ ] **Step 2: Run terminal-view targeted tests**

Run: `cargo test -p terminal_view public_mcp -- --nocapture`

Expected: terminal-view Public MCP tests pass.

- [ ] **Step 3: Run main approval tests**

Run: `cargo test -p main public_mcp_approval -- --nocapture`

Expected: approval tests pass.

- [ ] **Step 4: Run compile checks**

Run: `cargo check -p public_mcp`

Expected: check exits 0.

Run: `cargo check -p terminal_view`

Expected: check exits 0.

- [ ] **Step 5: Manual smoke test with a real SSH terminal**

Use an exposed SSH terminal session and verify:

1. `public_mcp.list_sessions` shows the SSH session.
2. `public_mcp.remote_exec` runs `pwd` and returns exit code 0.
3. `public_mcp.remote_exec` runs `docker info --format '{{.DockerRootDir}}'` and returns complete stdout.
4. `public_mcp.remote_file_write` writes a short script to `/tmp/onetcli-mcp-smoke.sh`.
5. `public_mcp.remote_exec` runs `bash /tmp/onetcli-mcp-smoke.sh`.
6. Background `remote_exec` returns `command_id`.
7. `remote_command_poll` shows running and then exited state.
8. `remote_command_output` returns output by offset.
9. `remote_command_cancel` can cancel a long `sleep 300`.
10. Existing `public_mcp.terminal_snapshot` and `public_mcp.terminal_write` still work for interactive fallback.

---

## Rollout Strategy

1. Replace legacy terminal paste/snapshot tools with structured remote ops.
2. Add structured remote ops behind the existing Public MCP runtime.
3. Keep all new tools opt-in by name; do not change existing MCP client flows.
4. Use approval metadata to make high-risk operations visible before execution.
5. Document that automation should prefer `remote_exec` and `remote_file_write`, while `terminal_write` remains for interactive recovery.

---

## Risks And Mitigations

- **Risk:** Active terminal sessions may not expose enough SSH internals for non-interactive exec.
  - **Mitigation:** Reuse the tab-scoped SSH session manager if available; otherwise add a session-backed adapter in terminal-view before exposing the tool as production-ready.

- **Risk:** Long command output can grow without bound.
  - **Mitigation:** Use bounded buffers, byte offsets, and explicit truncation metadata.

- **Risk:** Command cancellation semantics differ by backend.
  - **Mitigation:** Start with best-effort cancellation and make returned status explicit.

- **Risk:** Destructive command detection can miss shell constructs.
  - **Mitigation:** Treat detection as approval/audit assistance only. Do not claim it is a security sandbox.

- **Risk:** Changing terminal snapshot output could break existing clients.
  - **Mitigation:** Preserve default `visible_text` behavior and add optional arguments only.

---

## Completion Criteria

The plan is complete when:

- legacy terminal paste/snapshot tools are removed; structured remote ops is the only execution path
- structured foreground exec returns stdout, stderr, exit code, duration, and timeout state
- background exec supports command id, polling, output reads, and cancellation
- remote file write supports path, content, overwrite policy, mode, byte count, and SHA-256
- session diagnostics distinguish connected, disconnected, and unknown sessions
- approval requests include enough context for remote exec, file write, and cancellation
- docs explain when to use terminal bridge versus structured remote ops
- targeted tests and compile checks pass
- a real SSH terminal smoke test verifies the core workflow
