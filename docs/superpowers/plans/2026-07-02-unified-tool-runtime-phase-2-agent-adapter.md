# Unified Tool Runtime Phase 2 Agent Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first `agent_runtime` adapter layer for `tool_runtime` descriptors, resource pools, permission policies, and invocations without replacing the existing Agent tool execution flow.

**Architecture:** `agent_runtime` gains a small `tools::runtime_adapter` module that converts `tool_runtime::RuntimeToolDescriptor` into existing `ToolSpec`, converts `agent_runtime::ResourceContext` into `tool_runtime::ResourcePool`, maps `ToolExecutionMode` into `tool_runtime::PermissionPolicy`, and converts `ToolCall` into `tool_runtime::ToolInvocation`. Existing `agent_runtime::Tool`, `ToolRegistry`, `ToolRouter`, and all business tools remain unchanged in this phase.

**Tech Stack:** Rust 2024, `agent_runtime`, `tool_runtime`, existing `cargo test -p agent_runtime`.

---

## File Structure

- Modify: `crates/agent_runtime/Cargo.toml`
  - Add `tool_runtime = { workspace = true }`.
- Create: `crates/agent_runtime/src/tools/runtime_adapter.rs`
  - Implements pure conversion helpers.
- Modify: `crates/agent_runtime/src/tools/mod.rs`
  - Exports runtime adapter helpers.
- Modify: `crates/agent_runtime/src/tools/spec.rs`
  - Adds `ToolSpec::from_runtime_descriptor`.
- Modify: `crates/agent_runtime/src/resource.rs`
  - Adds `ResourceKind` and `ResourceContext` conversion helpers.
- Create: `crates/agent_runtime/tests/tool_runtime_adapter.rs`
  - Tests descriptor, resource, permission, and invocation adapter contracts.

## Task 1: Add Descriptor To ToolSpec Adapter

**Files:**
- Modify: `crates/agent_runtime/Cargo.toml`
- Modify: `crates/agent_runtime/src/tools/spec.rs`
- Create: `crates/agent_runtime/src/tools/runtime_adapter.rs`
- Modify: `crates/agent_runtime/src/tools/mod.rs`
- Create: `crates/agent_runtime/tests/tool_runtime_adapter.rs`

- [ ] **Step 1: Write failing descriptor adapter test**

Create `crates/agent_runtime/tests/tool_runtime_adapter.rs` with:

```rust
use agent_runtime::{RiskLevel, ToolName, ToolSpec};
use serde_json::json;
use tool_runtime::{
    RuntimeToolDescriptor, ToolAdapter, ToolAnnotations, ToolId, ToolMode, ToolOrigin,
    ToolTargetSpec,
};

#[test]
fn runtime_descriptor_converts_to_agent_tool_spec() {
    let descriptor = runtime_descriptor("db.query", ToolAnnotations::read_only("Query"));

    let spec = ToolSpec::from_runtime_descriptor(&descriptor);

    assert_eq!(ToolName::new("db.query"), spec.name);
    assert_eq!("Run query", spec.description);
    assert_eq!(json!({ "type": "object" }), spec.parameters);
    assert_eq!(RiskLevel::Read, spec.risk);
}

#[test]
fn runtime_descriptor_maps_high_risk_annotations() {
    let descriptor = runtime_descriptor(
        "db.exec",
        ToolAnnotations::mutating("Exec").with_risk(tool_runtime::RiskLevel::High),
    );

    let spec = ToolSpec::from_runtime_descriptor(&descriptor);

    assert_eq!(RiskLevel::High, spec.risk);
}
```

Include this helper in the same test file:

```rust
fn runtime_descriptor(
    id: &str,
    annotations: ToolAnnotations,
) -> RuntimeToolDescriptor {
    RuntimeToolDescriptor {
        id: ToolId::new(id),
        title: "Query".to_string(),
        description: "Run query".to_string(),
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        permissions: Vec::new(),
        mode: ToolMode::Deterministic,
        adapters: vec![ToolAdapter::FunctionCalling],
        annotations,
        target: ToolTargetSpec::default(),
        origin: ToolOrigin::Database,
        aliases: Vec::new(),
    }
}
```

- [ ] **Step 2: Verify descriptor adapter test fails**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter runtime_descriptor_converts_to_agent_tool_spec
```

Expected: compile failure because `ToolSpec::from_runtime_descriptor` does not exist and `agent_runtime` has no `tool_runtime` dependency.

- [ ] **Step 3: Add dependency and conversion implementation**

Add to `crates/agent_runtime/Cargo.toml` dependencies:

```toml
tool_runtime = { workspace = true }
```

Add to `crates/agent_runtime/src/tools/spec.rs`:

```rust
impl ToolSpec {
    pub fn from_runtime_descriptor(descriptor: &tool_runtime::RuntimeToolDescriptor) -> Self {
        Self {
            name: ToolName::new(descriptor.id.as_str()),
            description: descriptor.description.clone(),
            parameters: descriptor.input_schema.clone(),
            risk: runtime_risk_to_agent(descriptor.annotations.risk),
        }
    }
}

fn runtime_risk_to_agent(risk: tool_runtime::RiskLevel) -> RiskLevel {
    match risk {
        tool_runtime::RiskLevel::Read => RiskLevel::Read,
        tool_runtime::RiskLevel::Low => RiskLevel::Low,
        tool_runtime::RiskLevel::Medium => RiskLevel::Medium,
        tool_runtime::RiskLevel::High => RiskLevel::High,
        tool_runtime::RiskLevel::Critical => RiskLevel::Critical,
    }
}
```

- [ ] **Step 4: Create runtime adapter module shell**

Create `crates/agent_runtime/src/tools/runtime_adapter.rs`:

```rust
//! Adapters between agent_runtime compatibility types and tool_runtime core types.

pub fn runtime_descriptors_to_specs(
    descriptors: &[tool_runtime::RuntimeToolDescriptor],
) -> Vec<crate::tools::ToolSpec> {
    descriptors
        .iter()
        .map(crate::tools::ToolSpec::from_runtime_descriptor)
        .collect()
}
```

Update `crates/agent_runtime/src/tools/mod.rs`:

```rust
mod runtime_adapter;
pub use runtime_adapter::*;
```

- [ ] **Step 5: Verify descriptor adapter tests pass**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter runtime_descriptor
```

Expected: descriptor adapter tests pass.

## Task 2: Add ResourceContext To ResourcePool Adapter

**Files:**
- Modify: `crates/agent_runtime/src/resource.rs`
- Modify: `crates/agent_runtime/tests/tool_runtime_adapter.rs`

- [ ] **Step 1: Add failing resource adapter tests**

Append to `tool_runtime_adapter.rs`:

```rust
use agent_runtime::{ResourceContext, ResourceKind, ResourceRef, ResourceScope};

#[test]
fn resource_context_converts_to_runtime_resource_pool() {
    let context = ResourceContext::new()
        .with_resource(
            ResourceRef::new("db-prod", ResourceKind::Mysql, "prod db")
                .with_scope(ResourceScope::new("database", "Database", "ai_app")),
        )
        .with_resource(ResourceRef::new("ssh-prod", ResourceKind::Ssh, "prod ssh"));

    let pool = context.to_runtime_resource_pool();

    assert_eq!(Some(&tool_runtime::ResourceId::new("db-prod")), pool.default_target.as_ref());
    assert_eq!("prod db", pool.resolve_target("prod db").unwrap().label);
    assert_eq!("ai_app", pool.resources[0].scopes[0].value);
}
```

- [ ] **Step 2: Verify resource adapter test fails**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter resource_context_converts_to_runtime_resource_pool
```

Expected: compile failure because `to_runtime_resource_pool` does not exist.

- [ ] **Step 3: Implement resource conversion helpers**

Add to `crates/agent_runtime/src/resource.rs`:

```rust
impl ResourceContext {
    pub fn to_runtime_resource_pool(&self) -> tool_runtime::ResourcePool {
        let mut pool = tool_runtime::ResourcePool::new();
        for resource in &self.resources {
            pool = pool.with_resource(resource.to_runtime_resource_ref());
        }
        if let Some(current) = &self.current {
            pool = pool.with_default_target(tool_runtime::ResourceId::new(current.as_str()));
        }
        pool
    }
}

impl ResourceRef {
    pub fn to_runtime_resource_ref(&self) -> tool_runtime::ResourceRef {
        let mut resource = tool_runtime::ResourceRef::new(
            self.id.as_str(),
            self.kind.to_runtime_resource_kind(),
            self.label.clone(),
        );
        for scope in &self.scopes {
            resource = resource.with_scope(tool_runtime::ResourceScope::new(
                scope.key.clone(),
                scope.label.clone(),
                scope.value.clone(),
            ));
        }
        resource
    }
}

impl ResourceKind {
    pub fn to_runtime_resource_kind(&self) -> tool_runtime::ResourceKind {
        match self {
            ResourceKind::Ssh => tool_runtime::ResourceKind::Ssh,
            ResourceKind::Mysql => tool_runtime::ResourceKind::Mysql,
            ResourceKind::Postgres => tool_runtime::ResourceKind::Postgres,
            ResourceKind::Sqlite => tool_runtime::ResourceKind::Sqlite,
            ResourceKind::Redis => tool_runtime::ResourceKind::Redis,
            ResourceKind::Mongo => tool_runtime::ResourceKind::Mongo,
            ResourceKind::Terminal => tool_runtime::ResourceKind::Terminal,
            ResourceKind::Other(value) => tool_runtime::ResourceKind::Other(value.clone()),
        }
    }
}
```

- [ ] **Step 4: Verify resource adapter test passes**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter resource_context_converts_to_runtime_resource_pool
```

Expected: resource adapter test passes.

## Task 3: Add Permission And Invocation Adapter

**Files:**
- Modify: `crates/agent_runtime/src/tools/runtime_adapter.rs`
- Modify: `crates/agent_runtime/tests/tool_runtime_adapter.rs`

- [ ] **Step 1: Add failing permission and invocation tests**

Append to `tool_runtime_adapter.rs`:

```rust
use agent_runtime::{SessionId, ToolCall, ToolCallId, ToolExecutionMode, TurnId};
use agent_runtime::tools::runtime_tool_invocation_from_call;

#[test]
fn tool_execution_mode_maps_to_runtime_permission_profile() {
    assert_eq!(
        tool_runtime::PermissionProfile::Safe,
        agent_runtime::tools::permission_policy_for_tool_mode(ToolExecutionMode::ReadOnly).mode,
    );
    assert_eq!(
        tool_runtime::PermissionProfile::Confirm,
        agent_runtime::tools::permission_policy_for_tool_mode(ToolExecutionMode::Manual).mode,
    );
    assert_eq!(
        tool_runtime::PermissionProfile::Auto,
        agent_runtime::tools::permission_policy_for_tool_mode(ToolExecutionMode::Auto).mode,
    );
}

#[test]
fn tool_call_converts_to_runtime_invocation_with_resource_pool() {
    let context = ResourceContext::new().with_resource(ResourceRef::new(
        "ssh-prod",
        ResourceKind::Ssh,
        "prod ssh",
    ));
    let call = ToolCall::new("ssh.exec", json!({ "command": "df -h" }))
        .with_call_id(ToolCallId::from_string("call-1".to_string()));

    let invocation = runtime_tool_invocation_from_call(
        &call,
        &context,
        ToolExecutionMode::Manual,
        SessionId::from_string("session-1".to_string()),
        TurnId::from_string("turn-1".to_string()),
    );

    assert_eq!(tool_runtime::ToolId::new("ssh_exec"), invocation.tool_id);
    assert_eq!(json!({ "command": "df -h" }), invocation.arguments);
    assert_eq!(tool_runtime::ToolCaller::Agent, invocation.caller);
    assert_eq!(tool_runtime::PermissionProfile::Confirm, invocation.permission.mode);
    assert_eq!(Some("session-1".to_string()), invocation.audit.session_id);
}
```

- [ ] **Step 2: Verify permission/invocation tests fail**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter tool_execution_mode_maps_to_runtime_permission_profile
rtk cargo test -p agent_runtime --test tool_runtime_adapter tool_call_converts_to_runtime_invocation_with_resource_pool
```

Expected: compile failure because adapter functions do not exist.

- [ ] **Step 3: Implement adapter functions**

Add to `runtime_adapter.rs`:

```rust
use crate::{ResourceContext, SessionId, ToolCall, ToolExecutionMode, TurnId};

pub fn permission_policy_for_tool_mode(
    mode: ToolExecutionMode,
) -> tool_runtime::PermissionPolicy {
    let profile = match mode {
        ToolExecutionMode::ReadOnly => tool_runtime::PermissionProfile::Safe,
        ToolExecutionMode::Manual => tool_runtime::PermissionProfile::Confirm,
        ToolExecutionMode::Auto => tool_runtime::PermissionProfile::Auto,
    };
    tool_runtime::PermissionPolicy::for_profile(profile)
}

pub fn runtime_tool_invocation_from_call(
    call: &ToolCall,
    resources: &ResourceContext,
    tool_mode: ToolExecutionMode,
    session_id: SessionId,
    turn_id: TurnId,
) -> tool_runtime::ToolInvocation {
    tool_runtime::ToolInvocation::new(
        tool_runtime::ToolId::new(call.tool_name.as_str()),
        call.arguments.clone(),
        resources.to_runtime_resource_pool(),
        permission_policy_for_tool_mode(tool_mode),
        tool_runtime::ToolCaller::Agent,
    )
    .with_audit(tool_runtime::AuditContext {
        session_id: Some(session_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        request_id: Some(call.call_id.to_string()),
    })
}
```

If `SessionId`, `TurnId`, or `ToolCallId` lack `from_string` / `Display`, adapt the tests and implementation to the existing id API instead of adding broader ID changes.

- [ ] **Step 4: Verify adapter tests pass**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter
```

Expected: all adapter tests pass.

## Task 4: Full Phase 2 Verification

**Files:**
- All Phase 2 files above.

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt -p agent_runtime -p tool_runtime
```

Expected: formatting completes.

- [ ] **Step 2: Run tests**

Run:

```bash
rtk cargo test -p agent_runtime --test tool_runtime_adapter
rtk cargo test -p agent_runtime
rtk cargo test -p tool_runtime
```

Expected: all listed tests pass.

- [ ] **Step 3: Compile dependent crates**

Run:

```bash
rtk cargo check -p public_mcp
rtk cargo check -p ai_chat_view
```

Expected: both crates compile, proving existing Agent UI and Public MCP compatibility remain intact.

- [ ] **Step 4: Commit Phase 2**

Run:

```bash
rtk git add crates/agent_runtime docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-2-agent-adapter.md
rtk git commit -m "feat(agent_runtime): add tool runtime adapter contracts"
```

Expected: commit includes Phase 2 adapter and tests only.

## Plan Self-Review

Spec coverage:

1. Phase 2 adds descriptor conversion from `tool_runtime` to Agent model tools.
2. Phase 2 adds resource pool semantics by converting `ResourceContext` to `ResourcePool`.
3. Phase 2 adds existing Agent tool mode to unified permission profile mapping.
4. Phase 2 adds Agent tool call to `tool_runtime::ToolInvocation` conversion.
5. Phase 2 intentionally keeps existing Agent `Tool` execution flow unchanged.

Marker scan:

1. No unfinished markers are present.
2. Each step has concrete files, code, and verification commands.

Type consistency:

1. `ToolSpec::from_runtime_descriptor` consumes Phase 1 `RuntimeToolDescriptor`.
2. Resource conversion uses existing `agent_runtime::ResourceContext` as the compatibility source.
3. Invocation conversion uses existing `ToolCall` and returns Phase 1 `tool_runtime::ToolInvocation`.
