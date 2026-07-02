# Unified Tool Runtime Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the core unified Tool Runtime contract to `crates/tool_runtime` while preserving current registry, handler, MCP, CLI, and business-tool compatibility.

**Architecture:** Keep `tool_runtime` as a pure core crate with no Agent, MCP, UI, DB, SSH, or GPUI dependencies. Split the current monolithic `lib.rs` into focused modules for ids, resources, descriptors, invocation, permissions, audit, registry, result, and error. Existing `ToolRegistry::list/get/call`, `ToolDescriptor` struct literals, and `ToolHandler::call(input, ToolContext)` remain compatible, while new types model the final ResourcePool, PermissionPolicy, ToolInvocation, alias, and audit contracts.

**Compatibility Amendment:** Do not add required fields to the existing `ToolDescriptor` struct in Phase 1. Many downstream crates construct it with struct literals. Add `RuntimeToolDescriptor` for the final descriptor shape and add default `ToolHandler` metadata methods (`aliases`, `target_spec`, `origin`) so existing handlers keep compiling. `ToolAnnotations` may gain default-compatible semantic fields because downstream code mostly uses constructors; update any explicit literal compile break mechanically without changing behavior.

**Tech Stack:** Rust 2024, `serde`, `serde_json`, `thiserror`, current workspace `cargo test -p tool_runtime`.

---

## File Structure

- Create: `crates/tool_runtime/src/ids.rs`
  - Defines `ToolId` and `ResourceId` newtypes.
- Create: `crates/tool_runtime/src/resource.rs`
  - Defines `ResourcePool`, `ResourceRef`, `ResourceKind`, `ResourceScope`, `ResourceCapability`, `ResourceOrigin`, `ResourceTarget`, and target resolution.
- Create: `crates/tool_runtime/src/descriptor.rs`
  - Defines `RiskLevel`, existing-compatible `ToolAnnotations`, `ToolOrigin`, `ToolAlias`, `ToolTargetSpec`, existing-compatible `ToolDescriptor`, final-shape `RuntimeToolDescriptor`, `ToolAdapter`, and `ToolMode`.
- Create: `crates/tool_runtime/src/invocation.rs`
  - Defines `ToolInvocation`, `ToolCaller`, and `AuditContext`.
- Create: `crates/tool_runtime/src/permission.rs`
  - Defines `PermissionPolicy`, `PermissionProfile`, `OperationPolicy`, `PermissionDecision`, and permission decision logic.
- Create: `crates/tool_runtime/src/audit.rs`
  - Defines `ApprovalStatus`, `ApprovalRequest`, and `AuditEvent`.
- Create: `crates/tool_runtime/src/result.rs`
  - Moves `ToolResult`.
- Create: `crates/tool_runtime/src/error.rs`
  - Moves and extends `ToolError`.
- Create: `crates/tool_runtime/src/registry.rs`
  - Moves `ToolHandler`, `ToolContext`, `ToolFuture`, `ToolRegistry`, and duplicate/alias checks.
- Modify: `crates/tool_runtime/src/lib.rs`
  - Re-export the public API so downstream crates keep existing imports.
- Modify: `crates/tool_runtime/tests/registry.rs`
  - Keep existing registry tests and update helper descriptor construction for new defaulted fields.
- Create: `crates/tool_runtime/tests/resource_pool.rs`
  - Covers default target and id / label / alias target matching.
- Create: `crates/tool_runtime/tests/permission.rs`
  - Covers Safe / Confirm / Auto / Unrestricted profile decisions.
- Create: `crates/tool_runtime/tests/descriptor_alias.rs`
  - Covers canonical ids, aliases, duplicate canonical ids, duplicate aliases, and alias lookup.

## Task 1: Split Existing Runtime Without Behavior Change

**Files:**
- Create: `crates/tool_runtime/src/ids.rs`
- Create: `crates/tool_runtime/src/resource.rs`
- Create: `crates/tool_runtime/src/descriptor.rs`
- Create: `crates/tool_runtime/src/result.rs`
- Create: `crates/tool_runtime/src/error.rs`
- Create: `crates/tool_runtime/src/registry.rs`
- Modify: `crates/tool_runtime/src/lib.rs`
- Test: `crates/tool_runtime/tests/registry.rs`

- [ ] **Step 1: Add minimal id and resource foundations**

Create `crates/tool_runtime/src/ids.rs` before `descriptor.rs` because descriptors expose canonical ids:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolId(String);

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(String);

macro_rules! id_type {
    ($name:ident) => {
        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(ToolId);
id_type!(ResourceId);
```

Create `crates/tool_runtime/src/resource.rs` with `ResourceKind` first. Task 2 expands this file with the full `ResourcePool` model:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Ssh,
    Sftp,
    Mysql,
    Postgres,
    Sqlite,
    Redis,
    Mongo,
    Terminal,
    Other(String),
}

impl ResourceKind {
    pub fn as_str(&self) -> &str {
        match self {
            ResourceKind::Ssh => "ssh",
            ResourceKind::Sftp => "sftp",
            ResourceKind::Mysql => "mysql",
            ResourceKind::Postgres => "postgres",
            ResourceKind::Sqlite => "sqlite",
            ResourceKind::Redis => "redis",
            ResourceKind::Mongo => "mongo",
            ResourceKind::Terminal => "terminal",
            ResourceKind::Other(value) => value,
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
```

- [ ] **Step 2: Move existing descriptor-related types**

Create `crates/tool_runtime/src/descriptor.rs` with current `ToolDescriptor` fields preserved and a separate final-shape descriptor:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ToolId;
use crate::resource::ResourceKind;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAdapter {
    Cli,
    FunctionCalling,
    Mcp,
    Gui,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    Deterministic,
    Interactive,
    LongRunning,
    Streaming,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Read,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolAnnotations {
    pub title: String,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
    #[serde(default)]
    pub supports_parallel: bool,
    #[serde(default)]
    pub risk: RiskLevel,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    #[default]
    Builtin,
    Database,
    Ssh,
    Sftp,
    Redis,
    Terminal,
    PublicMcp,
    ExternalMcp,
    Acp,
    Cli,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolAlias {
    pub id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolTargetSpec {
    pub supported_kinds: Vec<ResourceKind>,
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permissions: Vec<String>,
    pub mode: ToolMode,
    pub adapters: Vec<ToolAdapter>,
    pub annotations: ToolAnnotations,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RuntimeToolDescriptor {
    pub id: ToolId,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permissions: Vec<String>,
    pub mode: ToolMode,
    pub adapters: Vec<ToolAdapter>,
    pub annotations: ToolAnnotations,
    pub target: ToolTargetSpec,
    pub origin: ToolOrigin,
    pub aliases: Vec<ToolAlias>,
}
```

- [ ] **Step 3: Keep existing constructors compatible**

Add these impls in `descriptor.rs`:

```rust
impl ToolAnnotations {
    pub fn read_only(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
            supports_parallel: false,
            risk: RiskLevel::Read,
        }
    }

    pub fn mutating(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: false,
            supports_parallel: false,
            risk: RiskLevel::High,
        }
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk = risk;
        self
    }

    pub fn with_parallel_support(mut self, supports_parallel: bool) -> Self {
        self.supports_parallel = supports_parallel;
        self
    }
}

impl ToolAlias {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ToolTargetSpec {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn required(supported_kinds: Vec<ResourceKind>) -> Self {
        Self {
            supported_kinds,
            required: true,
        }
    }
}

impl ToolDescriptor {
    pub fn tool_id(&self) -> ToolId {
        ToolId::new(self.id.clone())
    }

    pub fn supports_adapter(&self, adapter: ToolAdapter) -> bool {
        self.adapters.contains(&adapter)
    }

    pub fn matches_id_or_alias(&self, value: &str) -> bool {
        self.id == value
    }
}

impl RuntimeToolDescriptor {
    pub fn matches_id_or_alias(&self, value: &str) -> bool {
        self.id.as_str() == value || self.aliases.iter().any(|alias| alias.id == value)
    }
}
```

- [ ] **Step 4: Move result, error, and registry code**

Create `result.rs`, `error.rs`, and `registry.rs` by moving the existing code from `lib.rs`. Preserve these public symbols:

```rust
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<ToolResult, ToolError>> + Send + 'static>>;
pub trait ToolHandler: Send + Sync + 'static {
    fn descriptor(&self) -> ToolDescriptor;
    fn aliases(&self) -> Vec<ToolAlias> { Vec::new() }
    fn target_spec(&self) -> ToolTargetSpec { ToolTargetSpec::default() }
    fn origin(&self) -> ToolOrigin { ToolOrigin::Builtin }
    fn runtime_descriptor(&self) -> RuntimeToolDescriptor { ... }
    fn call_annotations(&self, _input: &Value) -> ToolAnnotations { ... }
    fn call(&self, input: Value, context: ToolContext) -> ToolFuture;
}
pub struct ToolRegistry { ... }
pub struct ToolContext { ... }
pub enum ToolError { ... }
pub struct ToolResult { ... }
```

The existing `ToolError` variants must remain:

```rust
UnknownTool { id: String }
UnsupportedAdapter { id: String, adapter: ToolAdapter }
Failed { message: String }
```

- [ ] **Step 5: Re-export old API from `lib.rs`**

Replace `lib.rs` body with module declarations and re-exports:

```rust
pub mod audit;
pub mod descriptor;
pub mod error;
pub mod ids;
pub mod invocation;
pub mod permission;
pub mod registry;
pub mod resource;
pub mod result;

pub use audit::*;
pub use descriptor::*;
pub use error::*;
pub use ids::*;
pub use invocation::*;
pub use permission::*;
pub use registry::*;
pub use resource::*;
pub use result::*;
```

- [ ] **Step 6: Run existing registry tests**

Run:

```bash
rtk cargo test -p tool_runtime --test registry
```

Expected: existing registry tests pass without adding fields to `ToolDescriptor` helper literals.

## Task 2: Add ResourcePool Core Model

**Files:**
- Modify: `crates/tool_runtime/src/resource.rs`
- Modify: `crates/tool_runtime/src/lib.rs`
- Create: `crates/tool_runtime/tests/resource_pool.rs`

- [ ] **Step 1: Write ResourcePool tests**

Create `crates/tool_runtime/tests/resource_pool.rs`:

```rust
use tool_runtime::{
    ResourceCapability, ResourceId, ResourceKind, ResourceOrigin, ResourcePool, ResourceRef,
    ResourceScope, ResourceTarget, TargetResolutionError,
};

#[test]
fn first_resource_becomes_default_target() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    assert_eq!(Some(&ResourceId::new("ssh-a")), pool.default_target.as_ref());
    assert_eq!("prod-b", pool.resolve_target("prod-b").unwrap().label);
}

#[test]
fn default_target_is_not_a_resource_boundary() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    let target = pool.resolve_target("ssh-b").expect("non-default resource should resolve");

    assert_eq!(ResourceId::new("ssh-b"), target.id);
}

#[test]
fn target_matches_id_label_or_alias() {
    let pool = ResourcePool::new().with_resource(
        ResourceRef::new("db-prod", ResourceKind::Mysql, "primary database")
            .with_alias("prod-db")
            .with_alias("production database")
            .with_scope(ResourceScope::new("schema", "Schema", "public"))
            .with_capability(ResourceCapability::Query),
    );

    assert_eq!(ResourceId::new("db-prod"), pool.resolve_target("db-prod").unwrap().id);
    assert_eq!(ResourceId::new("db-prod"), pool.resolve_target("primary database").unwrap().id);
    assert_eq!(ResourceId::new("db-prod"), pool.resolve_target("prod-db").unwrap().id);
}

#[test]
fn ambiguous_target_is_rejected() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod").with_alias("prod"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod").with_alias("prod"));

    let error = pool.resolve_target("prod").expect_err("ambiguous label should fail");

    assert!(matches!(error, TargetResolutionError::AmbiguousTarget { .. }));
}

#[test]
fn default_target_resolution_returns_default_resource() {
    let pool = ResourcePool::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    let target = pool.resolve_resource_target(None).unwrap();

    assert_eq!(ResourceId::new("ssh-a"), target.id);
}

#[test]
fn target_outside_pool_is_rejected() {
    let pool = ResourcePool::new().with_resource(ResourceRef::new(
        "ssh-a",
        ResourceKind::Ssh,
        "prod-a",
    ));

    let error = pool.resolve_target("ssh-z").expect_err("unknown target should fail");

    assert!(matches!(error, TargetResolutionError::TargetNotInPool { .. }));
}
```

- [ ] **Step 2: Expand `resource.rs`**

Create resource types with these public names and methods:

```rust
pub enum ResourceKind { Ssh, Sftp, Mysql, Postgres, Sqlite, Redis, Mongo, Terminal, Other(String) }
pub enum ResourceCapability { Query, Execute, ReadFile, WriteFile, ExecCommand, List, OpenSession, Other(String) }
pub enum ResourceOrigin { SavedConnection, ActiveSession, Workspace, PublicMcp, ExternalMcp, Acp, Cli, Other(String) }
pub struct ResourceScope { pub key: String, pub label: String, pub value: String }
pub struct ResourceRef { ... }
pub struct ResourcePool { pub default_target: Option<ResourceId>, pub resources: Vec<ResourceRef> }
pub enum ResourceTarget { Id(ResourceId), Label(String) }
pub enum TargetResolutionError { MissingTarget, TargetNotInPool { target: String }, AmbiguousTarget { target: String, matches: Vec<ResourceId> } }
```

Required methods:

```rust
impl ResourcePool {
    pub fn new() -> Self;
    pub fn with_resource(self, resource: ResourceRef) -> Self;
    pub fn with_default_target(self, id: impl Into<ResourceId>) -> Self;
    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRef>;
    pub fn default_resource(&self) -> Option<&ResourceRef>;
    pub fn resolve_target(&self, value: &str) -> Result<&ResourceRef, TargetResolutionError>;
    pub fn resolve_resource_target(&self, target: Option<&ResourceTarget>) -> Result<&ResourceRef, TargetResolutionError>;
    pub fn matching_kind(&self, kind: &ResourceKind) -> Vec<&ResourceRef>;
}
```

Matching must check id, label, and aliases exactly.

- [ ] **Step 3: Run ResourcePool tests**

Run:

```bash
rtk cargo test -p tool_runtime --test resource_pool
```

Expected: all ResourcePool tests pass.

## Task 3: Add PermissionPolicy Core Model

**Files:**
- Create: `crates/tool_runtime/src/permission.rs`
- Create: `crates/tool_runtime/tests/permission.rs`
- Modify: `crates/tool_runtime/src/lib.rs`

- [ ] **Step 1: Write permission tests**

Create `crates/tool_runtime/tests/permission.rs`:

```rust
use tool_runtime::{
    OperationPolicy, PermissionDecision, PermissionPolicy, PermissionProfile, ResourceId, RiskLevel,
    ToolAnnotations, ToolId,
};

#[test]
fn safe_profile_allows_read_and_denies_write() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Safe);

    assert_eq!(PermissionDecision::Allow, policy.decide(&ToolId::new("db.query"), None, &ToolAnnotations::read_only("Query")));
    assert_eq!(PermissionDecision::Deny, policy.decide(&ToolId::new("db.exec"), None, &ToolAnnotations::mutating("Exec")));
}

#[test]
fn confirm_profile_asks_for_mutating_and_high_risk_tools() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Confirm);

    assert_eq!(PermissionDecision::Allow, policy.decide(&ToolId::new("db.query"), None, &ToolAnnotations::read_only("Query")));
    assert_eq!(PermissionDecision::Ask, policy.decide(&ToolId::new("db.exec"), None, &ToolAnnotations::mutating("Exec")));
    assert_eq!(PermissionDecision::Ask, policy.decide(&ToolId::new("ssh.exec"), None, &ToolAnnotations::read_only("Exec").with_risk(RiskLevel::High)));
}

#[test]
fn auto_profile_allows_low_and_medium_but_asks_for_high_or_open_world() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Auto);

    assert_eq!(PermissionDecision::Allow, policy.decide(&ToolId::new("redis.get"), None, &ToolAnnotations::read_only("Get").with_risk(RiskLevel::Low)));
    assert_eq!(PermissionDecision::Allow, policy.decide(&ToolId::new("redis.keys"), None, &ToolAnnotations::read_only("Keys").with_risk(RiskLevel::Medium)));
    assert_eq!(PermissionDecision::Ask, policy.decide(&ToolId::new("ssh.exec"), None, &ToolAnnotations::mutating("Exec").with_risk(RiskLevel::High)));
}

#[test]
fn unrestricted_profile_allows_by_default() {
    let policy = PermissionPolicy::for_profile(PermissionProfile::Unrestricted);

    assert_eq!(PermissionDecision::Allow, policy.decide(&ToolId::new("sftp.write"), None, &ToolAnnotations::mutating("Write").with_risk(RiskLevel::Critical)));
}

#[test]
fn per_tool_and_resource_overrides_win() {
    let mut policy = PermissionPolicy::for_profile(PermissionProfile::Auto);
    policy.per_tool_overrides.insert(ToolId::new("db.exec"), OperationPolicy::Deny);
    policy.per_resource_overrides.insert(ResourceId::new("prod-db"), OperationPolicy::Ask);

    assert_eq!(PermissionDecision::Deny, policy.decide(&ToolId::new("db.exec"), Some(&ResourceId::new("staging-db")), &ToolAnnotations::mutating("Exec")));
    assert_eq!(PermissionDecision::Ask, policy.decide(&ToolId::new("db.query"), Some(&ResourceId::new("prod-db")), &ToolAnnotations::read_only("Query")));
}
```

- [ ] **Step 2: Implement `permission.rs`**

Create:

```rust
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{ResourceId, RiskLevel, ToolAnnotations, ToolId};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    Safe,
    Confirm,
    Auto,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPolicy {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PermissionPolicy {
    pub mode: PermissionProfile,
    pub read_policy: OperationPolicy,
    pub write_policy: OperationPolicy,
    pub high_risk_policy: OperationPolicy,
    pub per_tool_overrides: HashMap<ToolId, OperationPolicy>,
    pub per_resource_overrides: HashMap<ResourceId, OperationPolicy>,
}
```

`PermissionPolicy::for_profile` must map profiles as specified in the design doc.

`PermissionPolicy::decide` precedence:

```text
per_tool_overrides
per_resource_overrides
read/write/high-risk profile policy
```

High risk includes:

```rust
annotations.risk >= RiskLevel::High || annotations.destructive || annotations.open_world
```

- [ ] **Step 3: Run permission tests**

Run:

```bash
rtk cargo test -p tool_runtime --test permission
```

Expected: all permission tests pass.

## Task 4: Add Descriptor Alias Support

**Files:**
- Modify: `crates/tool_runtime/src/descriptor.rs`
- Modify: `crates/tool_runtime/src/registry.rs`
- Create: `crates/tool_runtime/tests/descriptor_alias.rs`

- [ ] **Step 1: Write alias tests**

Create `crates/tool_runtime/tests/descriptor_alias.rs`:

```rust
use std::sync::Arc;

use serde_json::json;
use tool_runtime::{
    ToolAdapter, ToolAlias, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler,
    ToolMode, ToolOrigin, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[test]
fn registry_get_resolves_alias_to_canonical_descriptor() {
    let registry = ToolRegistry::new(vec![Arc::new(EchoHandler::new("ssh.exec").with_alias("ssh.remote_exec"))]);

    let descriptor = registry.get("ssh.remote_exec", ToolAdapter::Mcp).unwrap();

    assert_eq!("ssh.exec", descriptor.id);
}

#[test]
fn registry_call_resolves_alias_to_canonical_handler() {
    let registry = ToolRegistry::new(vec![Arc::new(EchoHandler::new("db.query").with_alias("db_query"))]);

    let result = futures::executor::block_on(registry.call(
        "db_query",
        json!({ "sql": "select 1" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .unwrap();

    assert_eq!(json!({ "sql": "select 1" }), result.structured_content);
}

#[test]
fn registry_rejects_duplicate_aliases() {
    let error = ToolRegistry::try_new(vec![
        Arc::new(EchoHandler::new("db.query").with_alias("db_read")),
        Arc::new(EchoHandler::new("db.schema").with_alias("db_read")),
    ])
    .expect_err("duplicate aliases should fail");

    assert_eq!(vec!["db_read"], error.duplicate_tool_ids());
}

#[derive(Clone)]
struct EchoHandler {
    descriptor: ToolDescriptor,
}
```

Use the same `ToolHandler` helper style as `registry.rs`.

- [ ] **Step 2: Implement alias-aware duplicate checks and lookup**

In `registry.rs`, update duplicate detection to include canonical ids and aliases:

```rust
fn descriptor_keys(descriptor: &RuntimeToolDescriptor) -> Vec<String> {
    let mut keys = vec![descriptor.id.as_str().to_string()];
    keys.extend(descriptor.aliases.iter().map(|alias| alias.id.clone()));
    keys
}
```

`ToolRegistry::get`, `call_annotations`, and `call` must compare against `handler.runtime_descriptor()`:

```rust
runtime_descriptor.matches_id_or_alias(id)
```

When `call` receives an alias, it still calls the canonical handler. `UnsupportedAdapter` and `UnknownTool` errors may report the requested id.

- [ ] **Step 3: Run alias tests**

Run:

```bash
rtk cargo test -p tool_runtime --test descriptor_alias
```

Expected: alias lookup, alias call, and duplicate alias rejection pass.

## Task 5: Add Invocation And Audit Contract Types

**Files:**
- Create: `crates/tool_runtime/src/invocation.rs`
- Create: `crates/tool_runtime/src/audit.rs`
- Modify: `crates/tool_runtime/src/lib.rs`
- Create: `crates/tool_runtime/tests/invocation_audit.rs`

- [ ] **Step 1: Write contract tests**

Create `crates/tool_runtime/tests/invocation_audit.rs`:

```rust
use serde_json::json;
use tool_runtime::{
    ApprovalRequest, ApprovalStatus, AuditContext, AuditEvent, PermissionPolicy,
    PermissionProfile, ResourceId, ResourcePool, ResourceRef, ResourceKind, ResourceTarget,
    RiskLevel, ToolCaller, ToolId, ToolInvocation, ToolOrigin,
};

#[test]
fn invocation_carries_resource_pool_permission_and_target() {
    let pool = ResourcePool::new().with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let invocation = ToolInvocation::new(
        ToolId::new("ssh.exec"),
        json!({ "command": "df -h" }),
        pool.clone(),
        PermissionPolicy::for_profile(PermissionProfile::Confirm),
        ToolCaller::Agent,
    )
    .with_target(ResourceTarget::Id(ResourceId::new("ssh-a")));

    assert_eq!(ToolId::new("ssh.exec"), invocation.tool_id);
    assert_eq!(Some(ResourceTarget::Id(ResourceId::new("ssh-a"))), invocation.target);
    assert_eq!(Some(&ResourceId::new("ssh-a")), invocation.resources.default_target.as_ref());
}

#[test]
fn audit_event_records_target_risk_and_approval_status() {
    let event = AuditEvent {
        session_id: Some("session-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        tool_id: ToolId::new("db.exec"),
        origin: ToolOrigin::Database,
        target_resource: Some(ResourceId::new("prod-db")),
        caller: ToolCaller::Agent,
        risk: RiskLevel::High,
        approval_status: ApprovalStatus::Approved,
        arguments_redacted: json!({ "sql": "update users set name = ?" }),
        result_summary: Some("1 row affected".to_string()),
        started_at: "2026-07-02T00:00:00Z".to_string(),
        finished_at: Some("2026-07-02T00:00:01Z".to_string()),
    };

    assert_eq!(ApprovalStatus::Approved, event.approval_status);
    assert_eq!(Some(ResourceId::new("prod-db")), event.target_resource);
}

#[test]
fn approval_request_uses_same_core_tool_identity() {
    let request = ApprovalRequest {
        id: "approval-1".to_string(),
        tool_id: ToolId::new("sftp.write"),
        target_resource: Some(ResourceId::new("prod-a")),
        caller: ToolCaller::Mcp,
        risk: RiskLevel::High,
        summary: "Write SFTP file".to_string(),
        arguments_redacted: json!({ "path": "/tmp/out" }),
    };

    assert_eq!(ToolId::new("sftp.write"), request.tool_id);
    assert_eq!(ToolCaller::Mcp, request.caller);
}
```

- [ ] **Step 2: Implement `invocation.rs`**

Create:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PermissionPolicy, ResourcePool, ResourceTarget, ToolId};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCaller {
    Agent,
    Acp,
    Mcp,
    Cli,
    Ui,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AuditContext {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolInvocation {
    pub tool_id: ToolId,
    pub arguments: Value,
    pub target: Option<ResourceTarget>,
    pub resources: ResourcePool,
    pub permission: PermissionPolicy,
    pub caller: ToolCaller,
    pub audit: AuditContext,
}
```

Do not add a cancellation token yet because `tool_runtime` currently has no `tokio-util` dependency. Cancellation integration belongs to the later router migration.

- [ ] **Step 3: Implement `audit.rs`**

Create:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ResourceId, RiskLevel, ToolCaller, ToolId, ToolOrigin};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    NotRequired,
    Pending,
    Approved,
    Denied,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_id: ToolId,
    pub target_resource: Option<ResourceId>,
    pub caller: ToolCaller,
    pub risk: RiskLevel,
    pub summary: String,
    pub arguments_redacted: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AuditEvent {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_id: ToolId,
    pub origin: ToolOrigin,
    pub target_resource: Option<ResourceId>,
    pub caller: ToolCaller,
    pub risk: RiskLevel,
    pub approval_status: ApprovalStatus,
    pub arguments_redacted: Value,
    pub result_summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}
```

- [ ] **Step 4: Run invocation and audit tests**

Run:

```bash
rtk cargo test -p tool_runtime --test invocation_audit
```

Expected: all contract tests pass.

## Task 6: Full Phase 1 Verification

**Files:**
- Modify only if prior tasks reveal compile issues:
  - `crates/tool_runtime/src/*.rs`
  - `crates/tool_runtime/tests/*.rs`

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt -p tool_runtime
```

Expected: formatting completes without changing unrelated files.

- [ ] **Step 2: Run tool_runtime tests**

Run:

```bash
rtk cargo test -p tool_runtime
```

Expected: all `tool_runtime` tests pass.

- [ ] **Step 3: Compile Public MCP against updated tool_runtime**

Run:

```bash
rtk cargo check -p public_mcp
```

Expected: `public_mcp` compiles, proving `ToolRuntimeMcpProvider` still works with the expanded descriptor model.

- [ ] **Step 4: Compile onetcli_runtime against updated tool_runtime**

Run:

```bash
rtk cargo check -p onetcli_runtime
```

Expected: `onetcli_runtime` compiles, proving existing business tools still build descriptor literals with the compatibility fields resolved.

- [ ] **Step 5: Review working tree**

Run:

```bash
rtk git diff --stat
rtk git status --short
```

Expected:

1. Only `crates/tool_runtime/**` and this plan are changed by this phase.
2. Existing unrelated `crates/terminal_view/src/terminal_element.rs` remains unstaged and untouched.

- [ ] **Step 6: Commit implementation**

Run:

```bash
rtk git add crates/tool_runtime docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-1.md
rtk git commit -m "feat(tool_runtime): add unified core models"
```

Expected: commit includes Phase 1 code and tests only.

## Plan Self-Review

Spec coverage:

1. ResourcePool default target and resource pool semantics are covered in Task 2.
2. Descriptor aliases and canonical ids are covered in Task 4.
3. PermissionProfile decisions are covered in Task 3.
4. ToolInvocation, ApprovalRequest, and AuditEvent contracts are covered in Task 5.
5. Existing registry compatibility is preserved in Task 1 and verified in Task 6.

Marker scan:

1. No unfinished markers are present in implementation steps.
2. Each code task includes concrete test files, implementation files, and verification commands.

Type consistency:

1. `ToolId`, `ResourceId`, and `ResourceKind` are introduced in Task 1 before descriptors use them.
2. `RiskLevel` and expanded `ToolAnnotations` are introduced in Task 1 before permission tests use them.
3. `ToolCaller` is introduced before audit tests use it.
4. Existing `ToolDescriptor` struct literals remain compatible; final descriptor fields live on `RuntimeToolDescriptor` until later migration phases.
