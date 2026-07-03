# Unified Tool Runtime Phase 2b Agent Registry Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow Agent sessions to derive executable Agent tools from a `tool_runtime::ToolRegistry`, without removing the existing `agent_runtime::Tool` compatibility system.

**Architecture:** Extend `agent_runtime::tools::runtime_adapter` with a registry bridge. The bridge lists `tool_runtime::RuntimeToolDescriptor` values for a selected `tool_runtime::ToolAdapter`, converts each descriptor to an Agent `ToolSpec`, and wraps each runtime handler as an `agent_runtime::Tool`. Execution calls the canonical runtime tool id while exposing the sanitized Agent function name to the model. Results are converted to `ToolObservation`.

**Tech Stack:** Rust 2024, `agent_runtime`, `tool_runtime`, `async-trait`, existing `cargo test -p agent_runtime`.

---

## File Structure

- Modify: `crates/agent_runtime/src/tools/runtime_adapter.rs`
  - Adds `tool_runtime_agent_tool_registry` and the adapter tool implementation.
- Modify: `crates/agent_runtime/tests/tool_runtime_adapter.rs`
  - Adds async tests for listing and executing runtime tools through Agent registry.

## Task 1: Add Agent Registry Bridge Tests

**Files:**
- Modify: `crates/agent_runtime/tests/tool_runtime_adapter.rs`

- [ ] **Step 1: Add failing registry bridge tests**

Append to `tool_runtime_adapter.rs`:

```rust
use std::sync::{Arc, Mutex};
use tool_runtime::{ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolRegistry, ToolResult};

#[tokio::test]
async fn runtime_registry_exposes_agent_specs_with_canonical_runtime_id() {
    let registry = ToolRegistry::new(vec![Arc::new(RuntimeEchoTool::new("db.query"))]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );

    let specs = agent_registry.specs(&ResourceContext::new());

    assert_eq!(1, specs.len());
    assert_eq!("db_query", specs[0].name.as_str());
    assert_eq!("Echo input", specs[0].description);
    assert_eq!(json!({ "type": "object" }), specs[0].parameters);
}

#[tokio::test]
async fn runtime_registry_agent_tool_executes_canonical_runtime_tool() {
    let handler = Arc::new(RuntimeEchoTool::new("db.query"));
    let registry = ToolRegistry::new(vec![handler.clone()]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry
        .get(&ToolName::new("db.query"))
        .expect("runtime tool should be exposed to agent");

    let observation = tool
        .execute(agent_invocation(
            "db.query",
            json!({ "message": "hello" }),
        ))
        .await
        .expect("runtime tool call should execute");

    assert!(observation.success);
    assert_eq!("db_query", observation.tool_name.as_str());
    assert_eq!(json!({ "message": "hello" }), handler.last_input());
    assert_eq!(json!({ "message": "hello" }), observation_data_json(&observation));
}
```

Include helpers in the same test file:

```rust
#[derive(Clone)]
struct RuntimeEchoTool {
    descriptor: ToolDescriptor,
    last_input: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RuntimeEchoTool {
    fn new(id: &str) -> Self {
        Self {
            descriptor: ToolDescriptor {
                id: id.to_string(),
                title: "Echo".to_string(),
                description: "Echo input".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                permissions: Vec::new(),
                mode: ToolMode::Deterministic,
                adapters: vec![tool_runtime::ToolAdapter::FunctionCalling],
                annotations: ToolAnnotations::read_only("Echo"),
            },
            last_input: Arc::new(Mutex::new(None)),
        }
    }

    fn last_input(&self) -> serde_json::Value {
        self.last_input
            .lock()
            .expect("last input lock")
            .clone()
            .expect("runtime tool should receive input")
    }
}

impl ToolHandler for RuntimeEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        *self.last_input.lock().expect("last input lock") = Some(input.clone());
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}

fn agent_invocation(tool_name: &str, arguments: serde_json::Value) -> agent_runtime::tools::ToolInvocation {
    agent_runtime::tools::ToolInvocation {
        session_id: SessionId::from_string("session-1"),
        turn_id: TurnId::from_string("turn-1"),
        call_id: ToolCallId::from_string("call-1"),
        tool_name: ToolName::new(tool_name),
        arguments,
        resource_id: None,
        resources: ResourceContext::new(),
        cancellation: tokio_util::sync::CancellationToken::new(),
    }
}

fn observation_data_json(observation: &agent_runtime::tools::ToolObservation) -> serde_json::Value {
    match &observation.data {
        agent_runtime::tools::ObservationData::Json(value) => value.clone(),
        other => panic!("expected json observation data, got {other:?}"),
    }
}
```

- [ ] **Step 2: Verify registry bridge tests fail**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter runtime_registry
```

Expected: compile failure because `tool_runtime_agent_tool_registry` does not exist.

## Task 2: Implement Registry Bridge

**Files:**
- Modify: `crates/agent_runtime/src/tools/runtime_adapter.rs`

- [ ] **Step 1: Implement adapter tool**

Add to `runtime_adapter.rs`:

```rust
use async_trait::async_trait;
use crate::error::ToolError;
use crate::tools::{ObservationData, Tool, ToolName, ToolObservation, ToolRegistry, ToolSpec};

pub fn tool_runtime_agent_tool_registry(
    registry: tool_runtime::ToolRegistry,
    adapter: tool_runtime::ToolAdapter,
) -> ToolRegistry {
    let mut agent_registry = ToolRegistry::new();
    for descriptor in registry.list_runtime(adapter) {
        agent_registry.register(std::sync::Arc::new(ToolRuntimeAgentTool {
            name: ToolName::new(descriptor.id.as_str()),
            runtime_id: descriptor.id.as_str().to_string(),
            descriptor,
            registry: registry.clone(),
            adapter,
        }));
    }
    agent_registry
}

struct ToolRuntimeAgentTool {
    name: ToolName,
    runtime_id: String,
    descriptor: tool_runtime::RuntimeToolDescriptor,
    registry: tool_runtime::ToolRegistry,
    adapter: tool_runtime::ToolAdapter,
}
```

- [ ] **Step 2: Implement Agent Tool trait for adapter tool**

Add:

```rust
#[async_trait]
impl Tool for ToolRuntimeAgentTool {
    fn name(&self) -> ToolName {
        self.name.clone()
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::from_runtime_descriptor(&self.descriptor)
    }

    fn supports_parallel(&self) -> bool {
        self.descriptor.annotations.supports_parallel
    }

    async fn execute(
        &self,
        invocation: crate::tools::ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let result = self
            .registry
            .call(
                &self.runtime_id,
                invocation.arguments.clone(),
                tool_runtime::ToolContext::for_adapter(self.adapter),
            )
            .await
            .map_err(runtime_tool_error)?;
        Ok(runtime_result_to_observation(invocation, result))
    }
}
```

- [ ] **Step 3: Implement result and error conversion**

Add:

```rust
fn runtime_tool_error(error: tool_runtime::ToolError) -> ToolError {
    match error {
        tool_runtime::ToolError::UnknownTool { id } => ToolError::NotFound(id),
        tool_runtime::ToolError::UnsupportedAdapter { id, adapter } => {
            ToolError::Execution(format!("tool `{id}` is not exposed for adapter {adapter:?}"))
        }
        tool_runtime::ToolError::Failed { message } => ToolError::Execution(message),
    }
}

fn runtime_result_to_observation(
    invocation: crate::tools::ToolInvocation,
    result: tool_runtime::ToolResult,
) -> ToolObservation {
    let data = ObservationData::Json(result.structured_content);
    let summary = data.to_text();
    ToolObservation::success(
        invocation.call_id,
        invocation.tool_name,
        if summary.trim().is_empty() { "Tool succeeded".to_string() } else { summary },
        data,
    )
    .with_resource(invocation.resource_id)
}
```

- [ ] **Step 4: Verify registry bridge tests pass**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter runtime_registry
```

Expected: registry bridge tests pass.

## Task 3: Full Phase 2b Verification

**Files:**
- Modified files above.

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt -p agent_runtime
```

Expected: formatting completes.

- [ ] **Step 2: Run targeted tests**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter
rtk cargo test -p agent_runtime
rtk cargo test -p tool_runtime
```

Expected: all tests pass.

- [ ] **Step 3: Compile dependent crates**

Run:

```bash
rtk cargo check -p public_mcp
rtk cargo check -p ai_chat_view
```

Expected: both crates compile.

- [ ] **Step 4: Commit Phase 2b**

Run:

```bash
rtk git add crates/agent_runtime docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-2b-agent-runtime-registry-bridge.md
rtk git commit -m "feat(agent_runtime): bridge tool runtime registry"
```

Expected: commit includes the registry bridge, tests, and plan.

## Plan Self-Review

Spec coverage:

1. Agent can derive tool specs from a shared `tool_runtime::ToolRegistry`.
2. Agent can execute a runtime handler through a compatibility `agent_runtime::Tool`.
3. Canonical runtime id is retained internally while Agent uses sanitized `ToolName`.
4. Existing Agent tool execution flow remains unchanged.

Marker scan:

1. No unfinished markers are present.
2. Each step has concrete files, code, and verification commands.

Type consistency:

1. `ToolRuntimeAgentTool` stores `RuntimeToolDescriptor` and calls the canonical runtime id.
2. `runtime_result_to_observation` converts `tool_runtime::ToolResult` to `ObservationData::Json`.
3. Error conversion preserves unknown-tool as Agent `ToolError::NotFound`.
