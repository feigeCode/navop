# AI Production Readiness Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the AI Agent feature safe enough for production by closing persistence, log-safety, provider/tool readiness, and release-verification gaps.

**Architecture:** Reuse `chat_sessions` for AI history by adding agent/ask metadata fields and storing Agent runtime sessions as opaque JSON snapshots, keeping the runtime schema owned by `agent_runtime` and the storage schema owned by `one-core`. Redact sensitive values before any AI request/tool-call logging. Preserve existing runtime, UI, and MCP tool boundaries.

**Tech Stack:** Rust, GPUI, SQLite/rusqlite, `agent_runtime`, `ai_chat_view`, `one-core`, `public_mcp`, `cargo test`.

---

## Scope

- Persist and restore local AI Agent sessions, including history, tool calls, observations, current plan, resources, and system instruction.
- Keep existing `chat_sessions` history visible; legacy records without Agent snapshots load as Ask-mode history.
- Prevent AI runtime logs from exposing raw sensitive arguments or model payloads.
- Add tests around production-critical behavior.
- Do not change model prompts, tool schemas, or public MCP protocols unless a test exposes a required compatibility issue.

## Risks

- Snapshot JSON may contain sensitive business data. It must be stored locally like other app data, not logged.
- Storage migration must be backward compatible for existing installs.
- Log redaction must not break debugging completely; previews should remain structurally useful.
- Provider-specific real-model behavior remains partly manual unless credentials are available.

## Tasks

### Task 1: Agent Session Persistence

**Files:**
- Create: `crates/core/migrations/20260630000001_agent_sessions.sql`
- Modify: `crates/core/src/llm/chat_history.rs`
- Modify: `crates/core/src/llm/storage.rs`
- Modify: `crates/ai_chat_view/src/persistence.rs`

- [x] Add `chat_sessions` metadata fields: `session_kind`, `uid`, `snapshot_json`, `archived`.
- [x] Add `AgentSession` and `AgentSessionRepository` over the existing `chat_sessions` table.
- [x] Register the repository during LLM storage init.
- [x] Implement `save_session`, `list_summaries`, `list_archived_summaries`, `load_snapshot`, `delete_session`, `rename_session`, and `set_archived`.
- [x] Load legacy `chat_sessions` + `chat_messages` as Ask-mode `SessionSnapshot`.
- [x] Test save/list/load/rename/archive/delete, snapshot round-trip, and legacy Ask history fallback.

### Task 2: AI Log Redaction

**Files:**
- Modify: `crates/agent_runtime/src/tasks/agent.rs`
- Add tests near agent log helper functions.

- [x] Redact sensitive keys in logged model messages, tools, and tool-call arguments.
- [x] Preserve truncation behavior.
- [x] Cover nested objects and arrays.

### Task 3: Provider And Tool Readiness Verification

**Files:**
- Modify tests only unless issues are found.

- [x] Run `agent_runtime`, `ai_chat_view`, public MCP tool registry, ACP approval tests.
- [x] Run `cargo check -p main`.
- [x] Document ignored real-model tests and required manual smoke commands.

### Task 4: Release Decision

- [x] Summarize remaining risks.
- [x] Decide whether AI Agent can move from beta to production.

## Verification Results

Executed with `rtk`:

```bash
rtk cargo test -p ai_chat_view persistence
rtk cargo test -p agent_runtime tasks::agent::tests
rtk cargo test -p agent_runtime
rtk cargo test -p ai_chat_view
rtk cargo test -p main public_mcp_runtime::tool_registry
rtk cargo test -p main ai_chat_acp
rtk cargo test -p main ai_chat_acp_approval
rtk cargo check -p main
rtk cargo fmt -p agent_runtime -p ai_chat_view -p one-core -- --check
```

Results:

- `ai_chat_view persistence`: 5 passed.
- `agent_runtime tasks::agent::tests`: 4 passed.
- `agent_runtime`: 50 passed.
- `ai_chat_view`: 162 passed, 2 ignored.
- `main public_mcp_runtime::tool_registry`: 7 passed.
- `main ai_chat_acp`: 5 passed.
- `main ai_chat_acp_approval`: 3 passed.
- `main` check: 0 errors, 1 existing future-incompat warning from `block v0.1.6`.
- Targeted fmt check for changed packages passed.

Full-workspace `rtk cargo fmt --check` currently reports formatting diffs in unrelated files:

- `crates/terminal_view/src/view.rs`
- `crates/extension-runtime/src/extension_downloader.rs`
- `crates/extension-runtime/src/extension_downloader_network_tests.rs`

Those diffs were intentionally not included in this AI production-readiness change.

## Release Decision

AI local Agent/Ask history persistence and AI runtime log-safety blockers are closed for the tested local paths.

Remaining production caveats:

- Real-provider smoke tests remain credential-dependent; `ai_chat_view` has ignored real-model smoke tests that should be run with provider credentials before a public production rollout.
- Log redaction now protects runtime tracing for model messages/tool-call arguments, but persisted local snapshots still intentionally contain conversation and tool history. Treat the local database as sensitive application data.
- `block v0.1.6` future-incompat warning is unrelated to the AI changes but should be tracked before a toolchain upgrade.

## Verification Commands

```bash
cargo test -p agent_runtime
cargo test -p ai_chat_view
cargo test -p main public_mcp_runtime::tool_registry
cargo test -p main ai_chat_acp
cargo check -p main
```
