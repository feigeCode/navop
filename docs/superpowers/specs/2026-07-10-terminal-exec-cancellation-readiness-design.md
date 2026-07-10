# Terminal Exec Cancellation And Readiness Design

**Date:** 2026-07-10  
**Status:** Approved for implementation planning

## Summary

`terminal.exec` writes commands into an existing visible terminal. Its lifecycle must not be defined by PTY/stdout/stderr EOF, and cancelling an Agent turn must not stop the terminal command. Before writing a command, the tool must prove that the terminal is at an interactive shell prompt, discard any unsubmitted line safely, and confirm a fresh input prompt before submitting the requested command.

This design separates three lifecycles:

1. Agent turn lifecycle: reaches `Cancelled` within a bounded UI/runtime deadline.
2. Tool-call lifecycle: stops waiting for a cancelled turn and suppresses late results.
3. Terminal command lifecycle: continues independently after submission; its observer is cleaned up in the background without killing the terminal process.

The initial implementation scope is the visible SSH terminal path currently exposed by `terminal.exec`. Core readiness and supervision types should remain backend-neutral so a future local-terminal provider can reuse them.

## Existing Behavior And Root Causes

The current implementation captures SSH terminal output from the PTY stream and can complete on OSC 133 `CommandFinished`, a prompt-text fallback, or timeout. It does not directly wait for PTY EOF. Four architectural gaps remain:

- `exec_in_ssh_terminal` writes command bytes without proving that the shell can accept a new command or clearing an existing edited line.
- `PromptStart`, `InputStart`, `CommandStart`, and `CommandFinished` are emitted but not maintained as an authoritative readiness state.
- `terminal.exec` waits through a synchronous capture bridge in `spawn_blocking`; dropping the async waiter does not stop the blocking capture task.
- Agent cancellation is represented as a failed turn, and the UI waits for a later turn terminal event before leaving its running state.

Terminal readiness, tool cancellation, turn terminal state, and background observer cleanup are therefore coupled through one wait.

## Goals

- Never infer tool completion from PTY/stdout/stderr EOF or inherited descriptor closure.
- Never inject a shell command while a foreground task may own terminal stdin.
- Clear an unsubmitted shell line before every Agent/MCP `terminal.exec` submission.
- Cancel an Agent turn promptly without interrupting an already submitted terminal task.
- Prevent cancelled or stale tool results from mutating conversation history.
- Support `command &` and `nohup command &` without waiting for the background process.
- Serialize automated writes, detect concurrent human input, and fail closed when readiness is unknown.

## Non-Goals

- Killing or signalling a process launched through a visible terminal.
- Replacing structured `ssh.exec` or `ssh.command.*` operations.
- Making prompt-text heuristics authoritative without shell integration.
- Disabling normal human terminal input while an Agent runs.
- Adding unsafe injection or multi-terminal fan-out.

## Core Principles

1. **Shell-command readiness is not byte writability.** A terminal can accept stdin while `vim`, `top`, `npm`, or another foreground process is running, but it cannot safely accept a new shell command.
2. **Readiness precedes clearing.** No control character is sent until the supervisor atomically proves that the terminal is at a shell input prompt.
3. **Submission is the ownership boundary.** Before submission, cancellation prevents the command. After submission, cancellation only detaches the tool waiter.
4. **A cancelled turn is terminal immediately.** Worker joins, captures, and cleanup continue under supervisors and cannot delay the conversation state.
5. **Shell protocol events beat display heuristics.** OSC 133 epochs define readiness and completion; terminal-grid or prompt-text inspection is diagnostic only.

## Shell Command Readiness

The terminal input supervisor owns this state:

```rust
enum ShellCommandReadiness {
    Initializing, PromptRendering,
    Ready { prompt_epoch: u64 },
    ClearingInput { lease_id: u64, prompt_epoch: u64 },
    SubmissionPending { command_epoch: u64 },
    CommandRunning { command_epoch: u64 },
    AwaitingPrompt { command_epoch: u64 },
    Unknown, Unsupported, Disconnected,
}
```

Transitions:

```text
connect -> Initializing
PromptStart -> PromptRendering
InputStart -> Ready(new prompt_epoch)
Ready + outbound Enter -> SubmissionPending
CommandStart -> CommandRunning
CommandFinished -> AwaitingPrompt
next PromptStart -> PromptRendering
next InputStart -> Ready(new prompt_epoch)
disconnect -> Disconnected
missing/untrusted integration -> Unknown or Unsupported
```

`CommandFinished` does not make the terminal ready because shell hooks and prompt rendering may still be running. Only a fresh `InputStart` returns to `Ready`.

`Ready` means the shell owns stdin and is editing a command line; it does not mean that line is empty. Partial human input remains `Ready` and is deliberately discarded by the safe-replace handshake.

Any outbound user write containing CR/LF moves `Ready` to `SubmissionPending` before its bytes are sent. This closes the race between a human pressing Enter and the remote `CommandStart` event arriving.

## Terminal Input Supervisor

All SSH terminal writes pass through one actor instead of independent raw `SshCommand::Write(Vec<u8>)` sends.

```rust
enum TerminalInputSource {
    User, AgentPreflight, AgentCommand, TerminalResponse, InitCommand,
}

struct TerminalWrite {
    source: TerminalInputSource,
    bytes: Vec<u8>,
    write_seq: u64,
}
```

The supervisor owns readiness and connection epochs, terminal-event and input-write sequences, at most one automation lease, the active execution observer, and detachment bookkeeping. Human input remains enabled; if it races with automation preflight, the human write wins and invalidates the lease.

## Safe Replace Preflight

Agent- and MCP-facing `terminal.exec` always uses `SafeReplace`; callers cannot disable the clear-input invariant.

1. Resolve the target and check the connection.
2. Atomically require `Ready` and acquire the only automation lease.
3. Record prompt epoch, terminal-event sequence, and input-write sequence.
4. Send ETX (`Ctrl+C`, byte `0x03`) tagged as `AgentPreflight`.
5. Enter `ClearingInput` and wait for a newer `InputStart`.
6. Abort without submitting on cancellation, disconnect, clear timeout, or human input-write sequence change.
7. Submit command bytes and Enter atomically, then enter `SubmissionPending`.

ETX is chosen over `Ctrl+U` because it discards the whole pending shell line regardless of cursor position or common readline/zle editing modes. Visual line-clearing escapes are forbidden because they do not clear the shell editing buffer.

ETX may only be sent while holding a lease acquired from `Ready`. It is never sent from `CommandRunning`, `SubmissionPending`, `AwaitingPrompt`, `Unknown`, or `Unsupported`.

For `submit: false`, the same readiness and clear handshake runs, but the command is inserted without Enter and no completion observer is created.

## Busy And Unknown Behavior

The default policy is fail-fast:

| Readiness | Result | Terminal bytes written |
| --- | --- | --- |
| `Ready` | Run safe-replace preflight | ETX, then command after fresh `InputStart` |
| busy states | `terminal_busy` | None |
| `Unknown` | `readiness_unknown` | None |
| `Unsupported` | `readiness_unsupported` | None |
| `Disconnected` | `terminal_disconnected` | None |

Optional `ready_timeout_ms` waits for `Ready`, defaults to zero, and is capped at 10 seconds. Waiting sends no control characters. It remains separate from clear-input and command-completion deadlines so the tool cannot queue a surprise command indefinitely.

## Terminal Execution Operation

Replace the synchronous execute-and-wait handle with an asynchronous operation:

```rust
enum TerminalExecPhase {
    WaitingForReady, ClearingInput, Submitted, Observing, Completed,
    CancelledBeforeSubmit, DetachedAfterSubmit, Failed, TimedOut,
}

struct TerminalExecContext {
    operation_id: String,
    cancellation: CancellationToken,
}
```

```rust
trait TerminalExecSessionHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot;
    fn start_exec(
        &self,
        request: TerminalExecRequest,
        context: TerminalExecContext,
    ) -> TerminalExecFuture;
}
```

The operation waits on supervisor notifications without a blocking `Condvar` or `spawn_blocking`. Dropping or cancelling its caller unregisters that waiter without stopping the terminal supervisor.

## Completion Semantics

Completion is scoped to the submitted command epoch:

1. Matching `CommandFinished` is authoritative and yields `ShellIntegrationExit`.
2. A fresh `InputStart` after matching `CommandStart` may yield `PromptReturned` if the finish marker is unavailable but the protocol still proves readiness.
3. Otherwise the operation reaches `TimedOut` at its bounded deadline.

EOF, inherited descriptor closure, output quiet periods, and child-process exit are not completion signals.

For `npm run dev &` and `nohup command &`, the shell invocation completes after launching the background job. The result closes at that command boundary. The background process may remain alive and continue writing to the terminal; later output belongs to the terminal session and is not appended to the completed tool result.

## Tool Cancellation And Ownership

Extend tool execution context with audit identity, cancellation, and supervision:

```rust
struct ToolExecutionContext {
    adapter: ToolAdapter,
    audit: AuditContext,
    cancellation: CancellationToken,
    supervisor: ToolCallSupervisorHandle,
}

enum ToolCancellationPolicy {
    Cooperative, DetachExternalWork, TerminateOwnedWorkInBackground,
}
```

`terminal.exec` uses `DetachExternalWork`:

- Before submission, cancellation releases the lease and submits nothing.
- After submission, cancellation marks the tool call cancelled, detaches the Agent waiter, and suppresses its result from that turn.
- It never sends ETX, SIGINT, SIGTERM, SIGKILL, closes the terminal channel, or shuts down the PTY because the Agent was cancelled.
- The observer remains supervised until command completion, disconnect, or timeout, then releases its slot and discards output not consumed by an active turn.

Only work created and owned by another tool may use `TerminateOwnedWorkInBackground`. A process running in the user's visible terminal is not owned by `terminal.exec`.

## Agent Turn Cancellation

Add explicit events and stop mapping cancellation to failure:

```rust
RuntimeEvent::TurnCancelRequested { session_id, turn_id }
RuntimeEvent::TurnCancelled { session_id, turn_id }
```

`Runtime::interrupt` atomically transitions the turn to `Cancelled`, emits the event, cancels the turn token, notifies the tool supervisor, and hands the old worker to a run supervisor. It does not wait for worker or tool cleanup.

The UI leaves running state after a successful cancel acknowledgement; the event pump applies the same state idempotently. Runtime acknowledgement should occur within 100 ms under normal load and the UI must visibly stop within 500 ms.

A new turn may start immediately. History mutations and terminal outcomes use `finish_if_running(turn_id, ...)` or an equivalent generation check. Late deltas, observations, results, and outcomes from a cancelled turn are discarded or retained only as supervisor diagnostics; they cannot mutate the transcript or clear a newer active turn.

## Result And Error Contract

Keep existing request fields and add optional `ready_timeout_ms`; `timeout_ms` becomes the command-completion timeout. Clear-input timeout is an internal bounded constant.

Structured errors:

```text
terminal_busy
readiness_unknown
readiness_unsupported
terminal_disconnected
ready_timeout
clear_input_timeout
concurrent_user_input
capture_already_active
cancelled_before_submit
submission_failed
```

Cancellation after submission is lifecycle state `DetachedAfterSubmit`, not `TimedOut` and not a fabricated command result.

## Shell Integration Boundary

Automatic injection requires trusted shell-integration readiness. Without it, the tool fails closed instead of treating text ending in `$`, `#`, `%`, or `>` as a prompt; REPLs, database clients, and program output can contain the same text.

This supersedes the earlier assumption that prompt-text fallback can authorize automatic execution. Prompt parsing may remain for diagnostics or output sanitization, not for clearing or submission decisions.

## Testing Strategy

This cross-module behavioral and concurrency change requires TDD. Unit and contract tests cover:

- readiness transitions and pessimistic Enter transition;
- busy/unknown/disconnected requests writing zero bytes;
- `Ready -> ETX -> fresh InputStart -> command` ordering;
- cancellation during ready wait or clear handshake submitting no command;
- concurrent human input invalidating the lease;
- cancellation after submission sending no control character or process signal;
- command epochs ignoring stale finish events;
- `command &` and `nohup command &` completing without EOF;
- post-completion output exclusion and observer cleanup;
- immediate `TurnCancelled` despite a blocked tool handler;
- stale results not mutating a cancelled or newer turn;
- a new turn starting while old cleanup continues.

Manual smoke:

1. Type a partial shell line, approve `terminal.exec`, and verify it is discarded before the Agent command appears.
2. Run a foreground task and verify `terminal.exec` returns busy without changing it.
3. Submit a long foreground command, cancel the Agent, and verify the command continues.
4. Run `npm run dev &` and common `nohup` forms; verify tool completion at shell return while the process remains alive.
5. Cancel and immediately send a new Agent message; verify no stale event corrupts it.

## Acceptance Criteria

- Cancellation reaches `TurnCancelled` and restores UI input within 500 ms.
- No `terminal.exec` cancellation path interrupts or terminates a submitted command.
- A non-ready terminal receives no preflight or command bytes.
- Every submitted command follows a confirmed safe-replace handshake.
- Background/nohup completion is independent of inherited PTY descriptors.
- A cancelled turn receives no late history or transcript mutations.
- Observers release on completion, disconnect, or bounded timeout.
- Targeted terminal, tool-runtime, agent-runtime, public-MCP, and UI tests pass, followed by relevant checks, clippy, and manual terminal smoke verification.

## Superseded Decisions

This document supersedes conflicting earlier assumptions that:

- synchronous `spawn_blocking` is an acceptable long-term execution bridge;
- prompt-text fallback can authorize automatic injection;
- cancellation may wait for tool execution before the turn becomes terminal;
- terminal command cleanup belongs to Agent cancellation.

The existing product distinction remains: `terminal.exec` is visible terminal execution, while `ssh.exec` and `ssh.command.*` remain structured SSH operations.
