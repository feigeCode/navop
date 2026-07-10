# Terminal Exec Cancellation And Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make visible terminal execution readiness-aware and asynchronously cancellable while ensuring Agent cancellation never stops an already submitted terminal command.

**Architecture:** Add a pure terminal execution supervisor that owns shell readiness, automation leases, command epochs, output capture, and detached cleanup. Drive it from the existing SSH channel actor, propagate cancellation through `tool_runtime`, and make Agent turns enter `TurnCancelled` immediately with stale-turn write guards.

**Tech Stack:** Rust 2021, Tokio channels/oneshot/cancellation tokens, OSC 133 shell integration, GPUI event handling, Cargo tests.

---

## File Structure

- Create `crates/terminal/src/exec_supervisor.rs`: pure readiness/operation state machine and unit tests.
- Modify `crates/terminal/src/exec_capture.rs`: retain output sanitization as a non-blocking helper; remove blocking capture ownership.
- Modify `crates/terminal/src/types.rs`: asynchronous `TerminalExecHandle`, cancellation context, readiness errors, ready timeout.
- Modify `crates/terminal/src/ssh_backend.rs`: route writes/events through the supervisor and execute supervisor effects.
- Modify `crates/terminal/src/lib.rs`: export new terminal execution types.
- Modify `crates/tool_runtime/src/registry.rs`: add cancellation to `ToolContext`.
- Modify `crates/tool_runtime/Cargo.toml`: add `tokio-util`.
- Modify `crates/agent_runtime/src/tools/runtime_adapter.rs`: forward invocation cancellation.
- Modify `crates/public_mcp/src/registry.rs`: make terminal execution async and cancellation-aware.
- Modify `crates/public_mcp/src/tools/terminal_exec.rs`: remove `spawn_blocking`, parse `ready_timeout_ms`, await registry execution.
- Modify `crates/public_mcp/src/terminal_exec.rs`: expose the ready timeout in the public request.
- Modify `crates/terminal_view/src/public_mcp.rs`: await the core async handle and map structured failures.
- Modify `crates/agent_runtime/src/runtime/{active_turn,event,mod,session}.rs`: immediate cancellation and stale-turn guards.
- Modify `crates/ai_chat_view/src/agent_view.rs`: leave running state on cancellation acknowledgement/event.
- Update focused tests in the same crates and `docs/agent-tools-current-state.md`.

### Task 1: Build The Pure Terminal Execution Supervisor

**Files:** Create/Test `crates/terminal/src/exec_supervisor.rs`; modify `crates/terminal/src/lib.rs`.

- [x] **Step 1: Write failing readiness and safe-replace tests**

```rust
#[test]
fn ready_exec_clears_then_submits_after_fresh_input_start() {
    let mut state = ExecSupervisor::ready_for_test(7);
    assert_eq!(vec![Effect::WriteEtx], state.start(request("df -h"), 11));
    assert_eq!(
        vec![Effect::WriteCommand(b"df -h\n".to_vec())],
        state.on_osc(OscEvent::InputStart),
    );
}

#[test]
fn running_terminal_rejects_without_writing() {
    let mut state = ExecSupervisor::running_for_test(3);
    assert_eq!(vec![Effect::Fail(TerminalExecError::Busy)], state.start(request("pwd"), 12));
}
```
- [x] **Step 2: Run the focused tests and confirm red state**

Run: `rtk cargo test -p terminal exec_supervisor -- --nocapture`  
Expected: FAIL because `exec_supervisor` and its types do not exist.

- [x] **Step 3: Implement readiness, leases, phases, and effects**

```rust
pub(crate) enum Readiness { Initializing, PromptRendering, Ready { prompt_epoch: u64 }, Clearing, SubmissionPending, Running, AwaitingPrompt, Unknown, Disconnected }
pub(crate) enum Effect { WriteEtx, WriteCommand(Vec<u8>), Complete(TerminalExecOutput), Fail(TerminalExecError), ArmTimeout { id: u64, phase: ExecPhase, duration: Duration } }
pub(crate) struct ExecSupervisor { readiness: Readiness, event_seq: u64, input_seq: u64, prompt_epoch: u64, active: Option<ActiveExec> }
```
Implement `start`, `on_user_write`, `on_terminal_chunk`, `cancel`, `timeout`, and `disconnect`. `start` writes nothing unless readiness is `Ready`; safe replace emits ETX first; a newer `InputStart` submits the command; human writes invalidate pre-submit automation; post-submit cancellation removes only the result sender.

- [x] **Step 4: Add command-boundary tests**

```rust
#[test]
fn background_command_finishes_on_command_finished_without_eof() {
    let mut state = ExecSupervisor::observing_for_test(21, "npm run dev &");
    state.on_osc(OscEvent::CommandStart);
    assert!(matches!(state.on_osc(OscEvent::CommandFinished { exit_code: 0 }).as_slice(), [Effect::Complete(output)] if output.exit_code == Some(0)));
}
#[test]
fn cancel_after_submit_detaches_without_control_write() {
    let mut state = ExecSupervisor::observing_for_test(22, "sleep 300");
    assert!(state.cancel(22).is_empty() && state.is_detached(22));
}
#[test]
fn stale_command_finished_does_not_complete_new_epoch() {
    let mut state = ExecSupervisor::observing_for_test(23, "pwd");
    assert!(state.on_command_finished(22, 0).is_empty());
}
```
- [x] **Step 5: Run terminal supervisor tests**

Run: `rtk cargo test -p terminal exec_supervisor -- --nocapture`  
Expected: PASS.

- [x] **Step 6: Commit**

Run: `rtk git add crates/terminal/src/exec_supervisor.rs crates/terminal/src/lib.rs && rtk git commit -m "feat(terminal): add exec readiness supervisor"`

### Task 2: Drive The Supervisor From The SSH Actor

**Files:** Modify/Test `crates/terminal/src/{types,ssh_backend,exec_capture}.rs`.

- [x] **Step 1: Write failing async-handle and actor tests**

```rust
#[tokio::test]
async fn terminal_exec_handle_forwards_cancellation() {
    let token = CancellationToken::new();
    let handle = fake_async_handle();
    token.cancel();
    assert_eq!(TerminalExecError::CancelledBeforeSubmit, handle.exec(request(), token).await.unwrap_err());
}
```
- [x] **Step 2: Run and confirm failure**

Run: `rtk cargo test -p terminal terminal_exec_handle -- --nocapture`  
Expected: FAIL because the handle is synchronous.

- [x] **Step 3: Implement the async handle and SSH command variants**

```rust
pub type TerminalExecFuture = Pin<Box<dyn Future<Output = Result<TerminalExecOutput, TerminalExecError>> + Send>>;
pub struct TerminalExecHandle { exec_fn: Arc<dyn Fn(TerminalExecRequest, CancellationToken) -> TerminalExecFuture + Send + Sync> }
enum SshCommand { Write(TerminalWrite), StartExec { id: u64, request: TerminalExecRequest, result: oneshot::Sender<Result<TerminalExecOutput, TerminalExecError>> }, CancelExec { id: u64 }, ExecTimeout { id: u64, phase: ExecPhase }, Resize(TerminalSize), Shutdown }
```
The handle sends `StartExec` and selects between its oneshot and cancellation; on cancellation it sends `CancelExec`. The SSH actor converts OSC events and data into supervisor calls, executes returned writes/results, and schedules bounded timeout messages. Remove the blocking `session.wait()` path; keep output sanitization pure.

- [x] **Step 4: Verify SSH execution behavior**

Run: `rtk cargo test -p terminal terminal_exec -- --nocapture`  
Expected: PASS, including ready, busy, cancel-before-submit, detach-after-submit, background, nohup-equivalent, timeout, and disconnect cases.

- [x] **Step 5: Commit**

Run: `rtk git add crates/terminal/src/types.rs crates/terminal/src/ssh_backend.rs crates/terminal/src/exec_capture.rs crates/terminal/src/lib.rs && rtk git commit -m "refactor(terminal): make visible exec asynchronous"`

### Task 3: Propagate Cancellation Through Tool Runtime And Public MCP

**Files:** Modify `crates/tool_runtime/{Cargo.toml,src/registry.rs}`, `crates/agent_runtime/src/tools/runtime_adapter.rs`, `crates/public_mcp/src/{registry,terminal_exec}.rs`, `crates/public_mcp/src/tools/terminal_exec.rs`, and `crates/terminal_view/src/public_mcp.rs`; test `crates/public_mcp/tests/terminal_exec.rs` and `crates/agent_runtime/tests/tool_runtime_adapter.rs`.

- [x] **Step 1: Write failing cancellation propagation tests**

```rust
#[tokio::test]
async fn agent_adapter_forwards_cancelled_token_to_runtime_tool() {
    let token = CancellationToken::new();
    token.cancel();
    let observation = dispatch_runtime_tool_with(token).await;
    assert!(!observation.success);
    assert!(observation.summary.contains("cancel"));
}
```
- [x] **Step 2: Run and confirm failure**

Run: `rtk cargo test -p agent_runtime --test tool_runtime_adapter agent_adapter_forwards_cancelled_token_to_runtime_tool -- --nocapture`  
Expected: FAIL because `ToolContext` creates no invocation cancellation path.

- [x] **Step 3: Add cancellable `ToolContext` and async terminal registry**

```rust
#[derive(Clone)]
pub struct ToolContext { pub adapter: ToolAdapter, pub cancellation: CancellationToken }
impl ToolContext {
    pub fn for_adapter(adapter: ToolAdapter) -> Self { Self { adapter, cancellation: CancellationToken::new() } }
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self { self.cancellation = token; self }
}
```
Forward `ToolInvocation::cancellation` in the Agent adapter. Make `TerminalExecSessionHandle::exec_in_terminal` return a boxed future, make `PublicMcpRegistry::terminal_exec` async, remove `spawn_blocking`, pass `context.cancellation`, and parse optional `ready_timeout_ms`.

- [x] **Step 4: Run adapter/provider tests**

Run: `rtk cargo test -p tool_runtime -p public_mcp -p agent_runtime -p terminal_view terminal_exec -- --nocapture`  
Expected: PASS.

- [x] **Step 5: Commit**

Run: `rtk git add crates/tool_runtime crates/agent_runtime/src/tools/runtime_adapter.rs crates/public_mcp crates/terminal_view/src/public_mcp.rs && rtk git commit -m "feat(tools): propagate terminal exec cancellation"`

### Task 4: Make Agent Cancellation Immediately Terminal

**Files:** Modify `crates/agent_runtime/src/runtime/{active_turn,event,session,mod}.rs`; test `crates/agent_runtime/tests/integration.rs`.

- [x] **Step 1: Write failing immediate-cancel and stale-result tests**

```rust
#[tokio::test]
async fn interrupt_emits_cancelled_before_blocked_tool_returns() {
    let fixture = BlockingToolFixture::new();
    let turn_id = fixture.start_turn().await;
    fixture.runtime.interrupt(fixture.session.id()).unwrap();
    assert!(matches!(tokio::time::timeout(Duration::from_millis(100), fixture.next_event()).await.unwrap(), RuntimeEvent::TurnCancelled { turn_id: id, .. } if id == turn_id));
}
#[tokio::test]
async fn cancelled_turn_cannot_clear_or_append_to_new_turn() {
    let fixture = BlockingToolFixture::new();
    let stale = fixture.start_turn().await;
    fixture.runtime.interrupt(fixture.session.id()).unwrap();
    let current = fixture.start_second_turn().await;
    fixture.release(stale); assert_eq!(Some(current), fixture.session.active_turn_id());
}
```

Add `BlockingToolFixture` in the same test module with a oneshot release gate and event receiver so both tests are deterministic.

- [x] **Step 2: Run and confirm failure**

Run: `rtk cargo test -p agent_runtime interrupt_emits_cancelled_before_blocked_tool_returns -- --nocapture`  
Expected: FAIL because cancellation currently emits `TurnFailed` only after task return.

- [x] **Step 3: Implement turn-aware cancellation**

```rust
TurnCancelled { session_id: SessionId, turn_id: TurnId },
pub fn interrupt(&self, id: &SessionId) -> Result<TurnId, RuntimeError> {
    let session = self.session(id).ok_or_else(|| RuntimeError::SessionNotFound(id.clone()))?;
    let turn_id = session.cancel_and_detach_active_turn()?;
    session.emit(RuntimeEvent::TurnCancelled { session_id: id.clone(), turn_id: turn_id.clone() });
    Ok(turn_id)
}
```

Add `clear_active_turn_if(turn_id)` and `is_turn_writable(turn_id)`. Guard assistant deltas/messages, tool calls, observations, plans, and final outcomes. Background workers call `finish_if_active`; cancelled/stale workers cannot emit another terminal outcome or clear a newer turn.

- [x] **Step 4: Run Agent runtime tests**

Run: `rtk cargo test -p agent_runtime -- --nocapture`  
Expected: PASS.

- [x] **Step 5: Commit**

Run: `rtk git add crates/agent_runtime && rtk git commit -m "fix(agent): decouple turn cancellation from tool cleanup"`

### Task 5: Update UI Cancellation And Documentation

**Files:** Modify/Test `crates/ai_chat_view/src/{agent_view,agent_transcript}.rs`; modify `docs/agent-tools-current-state.md`.

- [x] **Step 1: Write failing UI event tests**

```rust
#[gpui::test]
fn stop_ack_immediately_clears_running_state(cx: &mut TestAppContext) {
    let view = running_agent_view(cx);
    view.update(cx, |this, cx| this.stop(cx)); assert!(!view.read(cx).is_running);
}
#[test]
fn cancelled_event_is_not_rendered_as_failure() {
    let mut transcript = AgentTranscript::new();
    transcript.apply(&cancelled_event()); assert_eq!(Some("cancelled"), transcript.last_status_code());
}
```

Add `running_agent_view` and `cancelled_event` helpers in the same test module using the existing runtime/view fixtures.

- [x] **Step 2: Run and confirm failure**

Run: `rtk cargo test -p ai_chat_view stop_ack_immediately_clears_running_state -- --nocapture`  
Expected: FAIL because local stop waits for a later runtime event.

- [x] **Step 3: Implement UI cancellation handling**

On successful `runtime.interrupt`, immediately call `set_running(false, cx)`. Treat `TurnCancelled` as a terminal event, persist once, and render cancellation separately from failure. Document readiness, safe replace, fail-closed behavior, and detached command ownership.

- [x] **Step 4: Run UI tests**

Run: `rtk cargo test -p ai_chat_view -- --nocapture`  
Expected: PASS.

- [x] **Step 5: Commit**

Run: `rtk git add crates/ai_chat_view docs/agent-tools-current-state.md && rtk git commit -m "fix(ai-chat): finish cancelled turns immediately"`

### Task 6: Full Verification And Review

**Files:** Modify only files required by verified findings.

- [x] **Step 1: Run formatting and focused verification**

Run: `rtk cargo fmt --all -- --check`  
Run: `rtk cargo test -p terminal -p tool_runtime -p agent_runtime -p public_mcp -p terminal_view -p ai_chat_view`  
Run: `rtk cargo check -p terminal -p tool_runtime -p agent_runtime -p public_mcp -p terminal_view -p ai_chat_view -p main`  
Expected: all exit 0; only the existing `block v0.1.6` future-incompatibility warning may remain.

- [x] **Step 2: Run clippy and repository checks**

Run: `rtk cargo clippy -p terminal -p tool_runtime -p agent_runtime -p public_mcp -p terminal_view -p ai_chat_view --all-targets -- -D warnings`  
Run: `rtk git diff --check`  
Expected: exit 0.

- [x] **Step 3: Perform manual terminal smoke**

Verify partial-line replacement, foreground busy rejection, Agent cancellation without command interruption, `npm run dev &`, and `nohup` with/without explicit stdio redirection. Record any environment limitation rather than fabricating a pass.

- [x] **Step 4: Request code review and address findings**

Use `superpowers:requesting-code-review`, apply verified findings with `superpowers:receiving-code-review`, then rerun affected tests.

## Verification Results (2026-07-10)

- Related test matrix: `876 passed, 2 ignored` across terminal, tool runtime, Agent runtime,
  Public MCP, terminal view, and AI chat view.
- Cross-crate check including `main`: `0 errors`; only the existing `block v0.1.6`
  future-incompatibility warning remains.
- `git diff --check`: passed; feature worktree clean after commits.
- Full `cargo fmt --all -- --check` was executed but is blocked by pre-existing formatting in
  untouched `terminal_element.rs`, `sql_editor.rs`, and `results_delegate.rs`; no task file was
  reported by rustfmt.
- Full strict clippy was executed. After fixing the task-adjacent `result_large_err`, it remains
  blocked by existing warnings in unchanged/shared code (`core`, `terminal`, `ai_chat_view`, and
  UI dependencies). The first complete run exposed 105 dependency warnings; `--no-deps` still
  exposed 18 pre-existing warnings in selected crates. These were not suppressed or folded into
  this lifecycle change.
- Manual native GPUI + live SSH smoke could not be performed in the non-interactive test
  environment. The same lifecycle cases are covered by deterministic supervisor/runtime/UI tests:
  partial input clearing, busy zero-write, pre-cancel zero-start, submitted command detach,
  background/nohup command-boundary completion, and stale-turn write rejection.

## Plan Self-Review

- Spec coverage: readiness, safe replace, fail-closed behavior, background/nohup completion, cancellation ownership, stale-turn guards, UI terminal state, and verification each map to a task.
- Placeholder scan: no deferred implementation decisions remain; test comments describe fixtures whose concrete helpers are created in the same task.
- Type consistency: `TerminalExecContext` becomes the core handle cancellation argument; `ToolContext` owns adapter-level cancellation; `TerminalExecError`, `ExecPhase`, and `TurnCancelled` names remain stable across tasks.
- Execution choice: the user selected inline execution in the current session.
