# Unified Tool Runtime Phase 6 Multi-Resource Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow one Agent turn to execute multiple explicit resource-targeted tool calls safely, with parallel execution only for tools that declare `supports_parallel`.

**Architecture:** Keep Agent planning and approval semantics unchanged. The Agent turn already collects all valid tool calls from a model response; Phase 6 adds a small batching layer that groups adjacent parallel-safe calls, dispatches safe batches concurrently, and keeps serial or approval-gated calls ordered. UI result grouping remains a later display layer over existing per-call `resource_id` observations.

**Tech Stack:** Rust, Tokio, `agent_runtime`, `ai_chat_view`, existing `Tool::supports_parallel` and `ToolRouter::supports_parallel`.

Current status: Task 1 verified; ready to commit.

---

## File Structure

Modify:

- `crates/agent_runtime/src/tasks/agent.rs`
  - Add a pure batching helper for executable tool calls.
  - Later dispatch each batch concurrently when every call supports parallel execution.
  - Keep approval behavior fail-closed: any call requiring approval pauses before dispatch.

- `crates/agent_runtime/tests/integration.rs`
  - Add regression coverage for multi-call ordering and safe parallel dispatch.

- `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Track Phase 6 status and verification.

Out of scope for the first checkpoint:

- Batched high-risk approvals.
- UI target-grouped result cards.
- Parallel execution for tools that do not explicitly opt in.
- Changing model prompt behavior beyond existing resource-pool semantics.

## Task 1: Batch Executable Calls By Parallel Safety

**Files:**
- Modify: `crates/agent_runtime/src/tasks/agent.rs`

- [x] **Step 1: Write failing batching tests**

Add pure tests near existing `agent.rs` tests:

```rust
#[test]
fn executable_call_batches_group_adjacent_parallel_safe_calls() {
    let calls = vec![
        ToolCall::new("read_a", serde_json::json!({})),
        ToolCall::new("read_b", serde_json::json!({})),
        ToolCall::new("write_a", serde_json::json!({})),
        ToolCall::new("read_c", serde_json::json!({})),
    ];
    let batches = executable_call_batches(calls, |call| call.tool_name.as_str().starts_with("read"));

    assert_eq!(batches.len(), 3);
    assert!(batches[0].parallel);
    assert_eq!(batches[0].calls.len(), 2);
    assert!(!batches[1].parallel);
    assert_eq!(batches[1].calls.len(), 1);
    assert!(batches[2].parallel);
    assert_eq!(batches[2].calls.len(), 1);
}

#[test]
fn executable_call_batches_keep_serial_calls_separate() {
    let calls = vec![
        ToolCall::new("write_a", serde_json::json!({})),
        ToolCall::new("write_b", serde_json::json!({})),
    ];
    let batches = executable_call_batches(calls, |_| false);

    assert_eq!(batches.len(), 2);
    assert!(batches.iter().all(|batch| !batch.parallel));
}
```

Expected red result:

```text
cannot find function `executable_call_batches` in this scope
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test -p agent_runtime executable_call_batches
```

Expected: fail because the batching helper is missing.

- [x] **Step 3: Add batching helper**

Add a private struct and helper:

```rust
struct ExecutableCallBatch {
    parallel: bool,
    calls: Vec<ToolCall>,
}

fn executable_call_batches(
    calls: Vec<ToolCall>,
    supports_parallel: impl Fn(&ToolCall) -> bool,
) -> Vec<ExecutableCallBatch> {
    let mut batches = Vec::new();
    let mut current_parallel = Vec::new();

    for call in calls {
        if supports_parallel(&call) {
            current_parallel.push(call);
        } else {
            if !current_parallel.is_empty() {
                batches.push(ExecutableCallBatch {
                    parallel: true,
                    calls: std::mem::take(&mut current_parallel),
                });
            }
            batches.push(ExecutableCallBatch {
                parallel: false,
                calls: vec![call],
            });
        }
    }

    if !current_parallel.is_empty() {
        batches.push(ExecutableCallBatch {
            parallel: true,
            calls: current_parallel,
        });
    }

    batches
}
```

- [x] **Step 4: Run tests to verify they pass**

Run:

```bash
rtk cargo test -p agent_runtime executable_call_batches
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agent_runtime/src/tasks/agent.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-6-multi-resource-execution.md
rtk git commit -m "feat(agent): batch executable tool calls"
```

## Task 2: Dispatch Parallel-Safe Batches Concurrently

**Files:**
- Modify: `crates/agent_runtime/src/tasks/agent.rs`
- Modify: `crates/agent_runtime/tests/integration.rs`

- [ ] **Step 1: Add integration test with delayed parallel tools**

Create two fake tools that opt into `supports_parallel`, block on a barrier or delayed channel,
and prove one turn can start both before either finishes.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
rtk cargo test -p agent_runtime parallel_tool_calls_start_before_first_finishes
```

Expected: fail under current serial dispatch.

- [ ] **Step 3: Dispatch each batch**

Replace the serial `for call in executable_calls` dispatch loop with:

1. Record all tool calls before dispatch as today.
2. Build batches using `ctx.services.tools.supports_parallel(call)`.
3. Serial batch: dispatch one call and record observation.
4. Parallel batch: spawn one future per call with cloned `Arc<RuntimeServices>`, `Arc<Session>`,
   `ToolDispatchContext`, goal, turn id, and cancellation token.
5. Join all futures and record observations in original call order.

- [ ] **Step 4: Run tests**

Run:

```bash
rtk cargo test -p agent_runtime parallel_tool_calls_start_before_first_finishes
rtk cargo test -p agent_runtime integration
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agent_runtime/src/tasks/agent.rs crates/agent_runtime/tests/integration.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-6-multi-resource-execution.md
rtk git commit -m "feat(agent): dispatch parallel-safe tool batches"
```

## Task 3: Preserve Approval And Serial Semantics

**Files:**
- Modify: `crates/agent_runtime/tests/high_risk_approval.rs`
- Modify: `crates/agent_runtime/tests/integration.rs`

- [ ] **Step 1: Add regression tests**

Cover:

1. A manual-mode call still pauses before any dispatch.
2. A non-parallel call between two parallel calls creates three batches.
3. Observation order remains the original tool-call order.

- [ ] **Step 2: Run tests and fix gaps**

Run:

```bash
rtk cargo test -p agent_runtime high_risk_approval
rtk cargo test -p agent_runtime executable_call_batches
rtk cargo test -p agent_runtime observation_order
```

- [ ] **Step 3: Commit**

```bash
rtk git add crates/agent_runtime/src/tasks/agent.rs crates/agent_runtime/tests/high_risk_approval.rs crates/agent_runtime/tests/integration.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-6-multi-resource-execution.md
rtk git commit -m "test(agent): preserve approval and observation ordering"
```

## Task 4: Phase 6 Tracking

**Files:**
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
- Modify: `docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-6-multi-resource-execution.md`

- [ ] **Step 1: Run verification**

Run:

```bash
rtk cargo test -p agent_runtime executable_call_batches
rtk cargo test -p agent_runtime parallel_tool_calls_start_before_first_finishes
rtk cargo test -p agent_runtime high_risk_approval
rtk cargo check -p agent_runtime
rtk git diff --check
```

- [ ] **Step 2: Update tracking**

Update Phase 6 in the design doc with commit hashes and verification results.

- [ ] **Step 3: Commit tracking**

```bash
rtk git add docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-6-multi-resource-execution.md
rtk git commit -m "docs: track multi-resource execution checkpoint"
```

## Self-Review

Spec coverage:

1. Uses existing `supports_parallel` as the safety switch.
2. Keeps approval fail-closed before dispatch.
3. Keeps observation ordering deterministic even when execution is concurrent.

Placeholder scan:

1. UI result grouping and batched approvals are explicitly deferred.
2. Every implementation task has concrete files and verification commands.

Type consistency:

1. Batches hold existing `ToolCall` values.
2. No new public API is required for Task 1.
