# Agent Resource Catalog/Scope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split AI resource handling into a discoverable catalog and an executable task scope, while fixing resource-pool card readability and `@` completion clipping.

**Architecture:** Add `ResourceCatalog` and `AgentResourceScope` as compatibility wrappers around the existing `ResourceRef` / `ResourceContext` runtime path. UI reads catalog for `@` and add-resource flows, but tools execute only against scope/default target. Resource UI gets compact rows plus detail/add dialogs; mention completion is verified not to be clipped by the bottom composer.

**Tech Stack:** Rust, GPUI, `agent_runtime`, `ai_chat_view`, `tool_runtime`, existing `gpui_component` `Popover`/`DialogButtonProps`, `rtk cargo test`, `rtk cargo check`.

---

## File Structure

- Create `crates/agent_runtime/src/resource_scope.rs`: catalog/scope/default-target types and conversions to `ResourceContext`.
- Modify `crates/agent_runtime/src/lib.rs`: export new catalog/scope types.
- Modify `crates/agent_runtime/src/tasks/agent_prompt.rs`: describe scope and catalog distinctly in the system prompt.
- Modify `crates/agent_runtime/src/tools/runtime_adapter.rs`: centralize target resolution and reject no-target execution when a tool requires target and scope has no default.
- Modify `crates/ai_chat_view/src/resource_builder.rs`: build catalog/scope pairs for workbench and sidebars.
- Modify `crates/ai_chat_view/src/resource_builder_tests.rs`: cover workbench empty scope and sidebar current default.
- Modify `crates/ai_chat_view/src/agent_view.rs`: replace ambiguous `available_resources` flows with catalog/scope conversion, keep runtime-compatible `ResourceContext`.
- Modify `crates/ai_chat_view/src/default_panel.rs`: pass catalog/scope into workbench/sidebar constructors without losing mode.
- Modify `crates/ai_chat_view/src/input/context.rs`: extend resource item display data with status/default reason/primary metadata.
- Modify `crates/ai_chat_view/src/input/agent_input.rs`: render compact resource rows, open resource detail/add dialogs, and test `@` popover visibility.
- Modify `main/src/onetcli_app.rs`: initialize workbench with all-connection catalog and empty scope.
- Modify `crates/terminal_view/src/sidebar/mod.rs`, `crates/db_view/src/sidebar/mod.rs`, `crates/redis_view/src/sidebar.rs`, `crates/mongodb_view/src/sidebar.rs`: initialize catalog as all connections and scope as current connection.

---

### Task 1: Runtime Catalog and Scope Types

**Files:**
- Create: `crates/agent_runtime/src/resource_scope.rs`
- Modify: `crates/agent_runtime/src/lib.rs`
- Test: `crates/agent_runtime/src/resource_scope.rs`

- [ ] **Step 1: Write failing runtime scope tests**

Add `crates/agent_runtime/src/resource_scope.rs` with tests first:

```rust
use crate::resource::{ResourceContext, ResourceId, ResourceKind, ResourceRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCatalog {
    pub resources: Vec<ResourceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResourceScope {
    pub selected: Vec<ResourceRef>,
    pub default_target: Option<DefaultTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultTarget {
    pub resource_id: ResourceId,
    pub reason: DefaultTargetReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultTargetReason {
    CurrentTerminal,
    CurrentDatabase,
    CurrentConnection,
    UserSelected,
    MentionedFirst,
    RestoredSession,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(id: &str, kind: ResourceKind, label: &str) -> ResourceRef {
        ResourceRef::new(id, kind, label)
    }

    #[test]
    fn workbench_scope_can_start_empty_while_catalog_has_resources() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let scope = AgentResourceScope::empty();

        assert_eq!(2, catalog.resources.len());
        assert!(scope.selected.is_empty());
        assert!(scope.default_target.is_none());
        assert!(scope.to_resource_context().is_empty());
    }

    #[test]
    fn current_connection_scope_sets_explicit_default_target() {
        let current = resource("ssh-a", ResourceKind::Ssh, "prod-a");

        let scope =
            AgentResourceScope::single_default(current.clone(), DefaultTargetReason::CurrentConnection);

        assert_eq!(vec![current], scope.selected);
        assert_eq!(
            Some(&ResourceId::new("ssh-a")),
            scope.default_target.as_ref().map(|target| &target.resource_id)
        );
        assert_eq!(
            Some("prod-a"),
            scope.to_resource_context()
                .current()
                .map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn adding_mentioned_resource_sets_default_only_when_scope_has_no_default() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let mut scope = AgentResourceScope::empty();

        assert!(scope.add_from_catalog(&catalog, &ResourceId::new("db-a"), DefaultTargetReason::MentionedFirst));

        assert_eq!(1, scope.selected.len());
        assert_eq!(
            Some(&ResourceId::new("db-a")),
            scope.default_target.as_ref().map(|target| &target.resource_id)
        );
    }

    #[test]
    fn adding_second_mentioned_resource_keeps_existing_default() {
        let catalog = ResourceCatalog::new(vec![
            resource("ssh-a", ResourceKind::Ssh, "prod-a"),
            resource("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let mut scope =
            AgentResourceScope::single_default(resource("ssh-a", ResourceKind::Ssh, "prod-a"), DefaultTargetReason::CurrentConnection);

        assert!(scope.add_from_catalog(&catalog, &ResourceId::new("db-a"), DefaultTargetReason::MentionedFirst));

        assert_eq!(2, scope.selected.len());
        assert_eq!(
            Some(&ResourceId::new("ssh-a")),
            scope.default_target.as_ref().map(|target| &target.resource_id)
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
rtk cargo test -p agent_runtime resource_scope
```

Expected: FAIL with missing methods such as `ResourceCatalog::new`, `AgentResourceScope::empty`, `AgentResourceScope::single_default`, `AgentResourceScope::add_from_catalog`, and `AgentResourceScope::to_resource_context`.

- [ ] **Step 3: Implement minimal runtime types**

Replace the top of `crates/agent_runtime/src/resource_scope.rs` with full implementations:

```rust
use crate::resource::{ResourceContext, ResourceId, ResourceKind, ResourceRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCatalog {
    pub resources: Vec<ResourceRef>,
}

impl ResourceCatalog {
    pub fn new(resources: Vec<ResourceRef>) -> Self {
        Self { resources }
    }

    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRef> {
        self.resources.iter().find(|resource| &resource.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResourceScope {
    pub selected: Vec<ResourceRef>,
    pub default_target: Option<DefaultTarget>,
}

impl AgentResourceScope {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn single_default(resource: ResourceRef, reason: DefaultTargetReason) -> Self {
        let target = DefaultTarget {
            resource_id: resource.id.clone(),
            reason,
        };
        Self {
            selected: vec![resource],
            default_target: Some(target),
        }
    }

    pub fn from_resource_context(context: ResourceContext, reason: DefaultTargetReason) -> Self {
        let default_target = context.current.map(|id| DefaultTarget {
            resource_id: id,
            reason,
        });
        Self {
            selected: context.resources,
            default_target,
        }
    }

    pub fn to_resource_context(&self) -> ResourceContext {
        ResourceContext {
            current: self.default_target.as_ref().map(|target| target.resource_id.clone()),
            resources: self.selected.clone(),
        }
    }

    pub fn add_from_catalog(
        &mut self,
        catalog: &ResourceCatalog,
        id: &ResourceId,
        reason: DefaultTargetReason,
    ) -> bool {
        if self.selected.iter().any(|resource| &resource.id == id) {
            return false;
        }
        let Some(resource) = catalog.get(id).cloned() else {
            return false;
        };
        self.selected.push(resource);
        if self.default_target.is_none() {
            self.default_target = Some(DefaultTarget {
                resource_id: id.clone(),
                reason,
            });
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultTarget {
    pub resource_id: ResourceId,
    pub reason: DefaultTargetReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultTargetReason {
    CurrentTerminal,
    CurrentDatabase,
    CurrentConnection,
    UserSelected,
    MentionedFirst,
    RestoredSession,
}
```

Modify `crates/agent_runtime/src/lib.rs`:

```rust
pub mod resource_scope;

pub use resource_scope::{
    AgentResourceScope, DefaultTarget, DefaultTargetReason, ResourceCatalog,
};
```

- [ ] **Step 4: Run runtime scope tests**

Run:

```bash
rtk cargo test -p agent_runtime resource_scope
```

Expected: PASS for the four `resource_scope` tests.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agent_runtime/src/resource_scope.rs crates/agent_runtime/src/lib.rs
rtk git commit -m "feat: add agent resource catalog scope types"
```

---

### Task 2: Build Catalog/Scope From Saved Connections

**Files:**
- Modify: `crates/ai_chat_view/src/resource_builder.rs`
- Modify: `crates/ai_chat_view/src/lib.rs`
- Test: `crates/ai_chat_view/src/resource_builder_tests.rs`

- [ ] **Step 1: Write failing builder tests**

Add tests to `crates/ai_chat_view/src/resource_builder_tests.rs`:

```rust
#[test]
fn workbench_resource_state_has_catalog_but_empty_scope() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-db", ConnectionType::Database, r#"{"type":"mysql"}"#),
    ];

    let (scope, catalog, mentions) = build_workbench_resource_state(&conns);

    assert!(scope.selected.is_empty());
    assert!(scope.default_target.is_none());
    assert_eq!(2, catalog.resources.len());
    assert_eq!(vec!["prod-a", "prod-db"], mentions.iter().map(|item| item.label.as_str()).collect::<Vec<_>>());
}

#[test]
fn sidebar_resource_state_keeps_current_connection_as_default_scope() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-db", ConnectionType::Database, r#"{"type":"mysql"}"#),
    ];

    let (scope, catalog, mentions) =
        build_sidebar_resource_state(&conns[1], &conns, DefaultTargetReason::CurrentDatabase);

    assert_eq!(1, scope.selected.len());
    assert_eq!("prod-db", scope.selected[0].label);
    assert_eq!(
        Some(&ResourceId::new("2")),
        scope.default_target.as_ref().map(|target| &target.resource_id)
    );
    assert_eq!(2, catalog.resources.len());
    assert_eq!(2, mentions.len());
}
```

Also add imports:

```rust
use agent_runtime::{DefaultTargetReason, ResourceId};
use crate::{build_sidebar_resource_state, build_workbench_resource_state};
```

- [ ] **Step 2: Run builder tests to verify failure**

Run:

```bash
rtk cargo test -p ai_chat_view resource_state
```

Expected: FAIL with unresolved imports `build_workbench_resource_state` and `build_sidebar_resource_state`.

- [ ] **Step 3: Implement builder functions**

Add to `crates/ai_chat_view/src/resource_builder.rs`:

```rust
use agent_runtime::{AgentResourceScope, DefaultTargetReason, ResourceCatalog};

pub fn build_workbench_resource_state(
    connections: &[StoredConnection],
) -> (AgentResourceScope, ResourceCatalog, Vec<MentionItem>) {
    let catalog = ResourceCatalog::new(build_resource_catalog(connections));
    let mentions = build_mentions_from_connections(connections);
    (AgentResourceScope::empty(), catalog, mentions)
}

pub fn build_sidebar_resource_state(
    current_connection: &StoredConnection,
    connections: &[StoredConnection],
    reason: DefaultTargetReason,
) -> (AgentResourceScope, ResourceCatalog, Vec<MentionItem>) {
    let catalog_connections = if connections.is_empty() {
        vec![current_connection.clone()]
    } else {
        connections.to_vec()
    };
    let catalog = ResourceCatalog::new(build_resource_catalog(&catalog_connections));
    let current_resource = connection_to_resource_ref(current_connection);
    let scope = AgentResourceScope::single_default(current_resource, reason);
    let mentions = build_mentions_from_connections(&catalog_connections);
    (scope, catalog, mentions)
}
```

Export from `crates/ai_chat_view/src/lib.rs`:

```rust
pub use resource_builder::{
    build_sidebar_resource_state, build_workbench_resource_state,
};
```

- [ ] **Step 4: Run builder tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_builder
```

Expected: PASS for all resource builder tests.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/resource_builder.rs crates/ai_chat_view/src/resource_builder_tests.rs crates/ai_chat_view/src/lib.rs
rtk git commit -m "feat: build agent catalog and scope from connections"
```

---

### Task 3: Wire Catalog/Scope Through AgentChatView

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `crates/ai_chat_view/src/default_panel.rs`
- Modify: `main/src/onetcli_app.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`
- Modify: `crates/db_view/src/sidebar/mod.rs`
- Modify: `crates/redis_view/src/sidebar.rs`
- Modify: `crates/mongodb_view/src/sidebar.rs`
- Test: `crates/ai_chat_view/src/agent_view.rs`
- Test: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Write failing AgentChatView tests**

Add tests near existing `agent_config_defaults_available_resources_to_pool_resources` in `crates/ai_chat_view/src/agent_view.rs`:

```rust
#[test]
fn agent_config_can_start_with_empty_scope_and_non_empty_catalog() {
    let catalog = ResourceCatalog::new(vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
    ]);
    let scope = AgentResourceScope::empty();

    let config = AgentChatViewConfig::new_with_scope(
        test_runtime("m"),
        scope.clone(),
        catalog.clone(),
        Vec::new(),
    );

    assert!(config.resources.is_empty());
    assert_eq!(catalog.resources, config.available_resources);
}

#[test]
fn applying_mentioned_resource_adds_from_catalog_and_sets_default() {
    let mut resources = ResourceContext::new();
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
    ];
    let mentions = vec![MentionItem::new("db-a", "prod-db", "mysql", "mysql")];

    assert!(apply_mentioned_resources(&mut resources, &catalog, &mentions));

    assert_eq!(1, resources.resources.len());
    assert_eq!(Some("prod-db"), resources.current().map(|resource| resource.label.as_str()));
}
```

Add these imports in the test module:

```rust
use agent_runtime::{AgentResourceScope, ResourceCatalog};
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
rtk cargo test -p ai_chat_view agent_config_can_start_with_empty_scope_and_non_empty_catalog
rtk cargo test -p ai_chat_view applying_mentioned_resource_adds_from_catalog_and_sets_default
```

Expected: FAIL because `AgentChatViewConfig::new_with_scope` does not exist.

- [ ] **Step 3: Implement config compatibility constructor**

Add to `impl AgentChatViewConfig` in `crates/ai_chat_view/src/agent_view.rs`:

```rust
pub fn new_with_scope(
    runtime: Arc<Runtime>,
    scope: AgentResourceScope,
    catalog: ResourceCatalog,
    mentions: Vec<MentionItem>,
) -> Self {
    let resources = scope.to_resource_context();
    let mut config = Self::new(runtime, resources, mentions);
    config.available_resources = catalog.resources;
    config
}
```

Keep existing fields for compatibility in this task. Do not rename `available_resources` yet.

- [ ] **Step 4: Add panel constructors for catalog/scope**

In `crates/ai_chat_view/src/default_panel.rs`, add:

```rust
pub fn new_workbench_with_scope_and_catalog(
    scope: agent_runtime::AgentResourceScope,
    catalog: agent_runtime::ResourceCatalog,
    mentions: Vec<MentionItem>,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> Self {
    Self::new_workbench_with_context_and_catalog(
        scope.to_resource_context(),
        mentions,
        catalog.resources,
        window,
        cx,
    )
}

pub fn new_sidebar_with_scope_and_catalog(
    scope: agent_runtime::AgentResourceScope,
    catalog: agent_runtime::ResourceCatalog,
    mentions: Vec<MentionItem>,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> Self {
    Self::new_with_context_and_catalog(
        scope.to_resource_context(),
        mentions,
        catalog.resources,
        window,
        cx,
    )
}
```

- [ ] **Step 5: Update app and sidebars to use scope/catalog builders**

In `main/src/onetcli_app.rs`, replace workbench context setup with:

```rust
let (scope, catalog, mentions) =
    ai_chat_view::build_workbench_resource_state(&connections);
let workbench = cx.new(|cx| {
    ai_chat_view::DefaultAgentChatPanel::new_workbench_with_scope_and_catalog(
        scope, catalog, mentions, window, cx,
    )
});
```

In terminal sidebar, use:

```rust
let (scope, catalog, mentions) = build_sidebar_resource_state(
    connection,
    &connections,
    agent_runtime::DefaultTargetReason::CurrentTerminal,
);
DefaultAgentChatPanel::new_sidebar_with_scope_and_catalog(
    scope, catalog, mentions, window, cx,
)
```

In DB sidebar, use `DefaultTargetReason::CurrentDatabase`. In Redis/Mongo sidebars, use `DefaultTargetReason::CurrentConnection`.

- [ ] **Step 6: Run view and sidebar tests**

Run:

```bash
rtk cargo test -p ai_chat_view agent_config_can_start_with_empty_scope_and_non_empty_catalog
rtk cargo test -p ai_chat_view applying_mentioned_resource_adds_from_catalog_and_sets_default
rtk cargo test -p terminal_view terminal_ai_context_keeps_current_connection_default_and_mentions_all_connections
rtk cargo test -p main initial_layout_pins_home_and_ai_workbench_with_ai_active
```

Expected: all listed tests PASS.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs crates/ai_chat_view/src/default_panel.rs main/src/onetcli_app.rs crates/terminal_view/src/sidebar/mod.rs crates/db_view/src/sidebar/mod.rs crates/redis_view/src/sidebar.rs crates/mongodb_view/src/sidebar.rs
rtk git commit -m "feat: wire agent catalog scope through chat views"
```

---

### Task 4: Enforce Target Resolution Semantics

**Files:**
- Modify: `crates/agent_runtime/src/tools/runtime_adapter.rs`
- Test: `crates/agent_runtime/tests/tool_runtime_target_adapter.rs`

- [ ] **Step 1: Write failing target resolver tests**

Add to `crates/agent_runtime/tests/tool_runtime_target_adapter.rs` near `runtime_registry_agent_tool_maps_default_target_to_runtime_connection`:

```rust
#[tokio::test]
async fn target_tool_without_target_or_default_returns_clear_error() {
    let handler =
        Arc::new(RuntimeEchoTool::new("db.query").with_input_schema(connection_sql_schema()));
    let registry = ToolRegistry::new(vec![handler]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry.get(&ToolName::new("db.query")).unwrap();

    let error = tool
        .execute(agent_invocation(
            "db.query",
            json!({ "sql": "select 1" }),
            ResourceContext::new(),
        ))
        .await
        .expect_err("targeted tool should require target or default scope resource");

    assert!(error.to_string().contains("target is required"));
}
```

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
rtk cargo test -p agent_runtime target_tool_without_target_or_default_returns_clear_error
```

Expected: FAIL because current resolver returns `None` instead of an explicit target-required error.

- [ ] **Step 3: Implement target-required error**

In `normalize_agent_arguments` after `resolve_target_id`, add:

```rust
if has_target && resource_id.is_none() {
    return Err(ToolError::InvalidArguments(
        "target is required: specify a resource target or select a default resource".to_string(),
    ));
}
if provider_field.is_some() && resource_id.is_none() {
    return Err(ToolError::InvalidArguments(
        "target is required: specify a resource target or select a default resource".to_string(),
    ));
}
```

Keep explicit target resolution using `pool.resolve_target_for_spec(target, target_spec)`. Do not make no-target execution search catalog.

- [ ] **Step 4: Run resolver tests**

Run:

```bash
rtk cargo test -p agent_runtime target_tool_without_target_or_default_returns_clear_error
rtk cargo test -p agent_runtime tool_runtime_adapter
```

Expected: PASS. Keep `runtime_registry_agent_tool_maps_default_target_to_runtime_connection` passing by preserving default-target resolution from `ResourceContext.current`; only the empty-scope/no-target case returns `target is required`.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agent_runtime/src/tools/runtime_adapter.rs crates/agent_runtime/tests/tool_runtime_target_adapter.rs
rtk git commit -m "fix: require explicit target without scope default"
```

---

### Task 5: Resource Pool Card and Dialog UI

**Files:**
- Modify: `crates/ai_chat_view/src/input/context.rs`
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Test: `crates/ai_chat_view/src/input/context.rs`
- Test: `crates/ai_chat_view/src/input/agent_input.rs`

- [ ] **Step 1: Write failing context data tests**

Update `resource_pool_item_exposes_add_and_remove_state` in `crates/ai_chat_view/src/input/context.rs` to use the richer item:

```rust
let in_pool = ComposerResourcePoolItem::new(
    "ssh-a",
    "prod-a",
    "SH",
    "ssh",
    "10.2.4.54",
    "active",
    Some("Current connection"),
    3,
    true,
    true,
);

assert_eq!(in_pool.primary_meta.as_ref(), "10.2.4.54");
assert_eq!(in_pool.status.as_ref(), "active");
assert_eq!(in_pool.default_reason.as_ref().map(|s| s.as_ref()), Some("Current connection"));
assert_eq!(3, in_pool.capability_count);
```

- [ ] **Step 2: Run context test to verify failure**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_item_exposes_add_and_remove_state
```

Expected: FAIL because `ComposerResourcePoolItem::new` does not accept `primary_meta`, `status`, `default_reason`, or `capability_count`.

- [ ] **Step 3: Extend item data**

Modify `ComposerResourcePoolItem`:

```rust
pub struct ComposerResourcePoolItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: SharedString,
    pub kind: SharedString,
    pub primary_meta: SharedString,
    pub status: SharedString,
    pub default_reason: Option<SharedString>,
    pub capability_count: usize,
    pub in_pool: bool,
    pub is_default: bool,
}
```

Update constructor:

```rust
pub fn new(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: impl Into<SharedString>,
    kind: impl Into<SharedString>,
    primary_meta: impl Into<SharedString>,
    status: impl Into<SharedString>,
    default_reason: Option<impl Into<SharedString>>,
    capability_count: usize,
    in_pool: bool,
    is_default: bool,
) -> Self {
    Self {
        id: id.into(),
        label: label.into(),
        icon: icon.into(),
        kind: kind.into(),
        primary_meta: primary_meta.into(),
        status: status.into(),
        default_reason: default_reason.map(Into::into),
        capability_count,
        in_pool,
        is_default,
    }
}
```

Update all call sites in `agent_view.rs::resource_pool_items`.

- [ ] **Step 4: Add resource dialog render helpers**

In `crates/ai_chat_view/src/input/agent_input.rs`, add helper methods on `AgentInput`:

```rust
fn render_resource_detail_dialog(
    item: ComposerResourcePoolItem,
) -> impl IntoElement {
    v_flex()
        .gap_3()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(item.label.clone()))
        .child(div().text_xs().child(format!("类型: {}", item.kind)))
        .child(div().text_xs().child(format!("状态: {}", item.status)))
        .child(div().text_xs().child(format!("主要信息: {}", item.primary_meta)))
        .when_some(item.default_reason.clone(), |this, reason| {
            this.child(div().text_xs().child(format!("默认目标: {reason}")))
        })
        .child(div().text_xs().child(format!("能力数量: {}", item.capability_count)))
}

fn show_resource_detail_dialog(
    item: ComposerResourcePoolItem,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title("资源详情")
            .w(px(420.0))
            .child(Self::render_resource_detail_dialog(item.clone()))
    });
}
```

Use the same `window.open_dialog` pattern for the add-resource dialog in this task; it should render a searchable list from catalog-derived `ComposerResourcePoolItem` values and call the existing add-resource callback when the user confirms.

- [ ] **Step 5: Render compact rows and dialogs**

Replace `resource_pool_item_row` body summary area with:

```rust
.child(
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.0))
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(div().text_sm().truncate().child(item.label.clone()))
                .when(item.is_default, |this| {
                    this.child(status_badge("默认", theme.accent, theme))
                })
                .child(status_badge(item.status.clone(), muted, theme)),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .truncate()
                .child(item.primary_meta.clone()),
        ),
)
```

Add a small detail button:

```rust
Button::new(format!("resource-detail-{}", item.id))
    .ghost()
    .xsmall()
    .icon(IconName::Info)
    .on_click(move |_, window, cx| {
        AgentInput::show_resource_detail_dialog(detail_item.clone(), window, cx);
    })
```

- [ ] **Step 6: Add GPUI layout tests**

Add to `crates/ai_chat_view/src/input/agent_input.rs` tests:

```rust
#[gpui::test]
fn resource_pool_rows_keep_stable_compact_height(cx: &mut TestAppContext) {
    let cx: &mut VisualTestContext = cx;
    let (_, cx) = cx.add_window_view(|window, cx| AgentInputLayoutRoot::wide(window, cx));

    let root = cx.debug_bounds("agent-input-root").expect("input should render");
    let area = cx.debug_bounds("agent-input-area").expect("input area should render");

    assert!(root.size.width <= area.size.width);
}
```

Add debug selectors to the resource row:

```rust
.debug_selector(|| "agent-resource-pool-row".to_string())
```

- [ ] **Step 7: Run UI data/layout tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_item_exposes_add_and_remove_state
rtk cargo test -p ai_chat_view resource_pool_rows_keep_stable_compact_height
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/context.rs crates/ai_chat_view/src/input/agent_input.rs crates/ai_chat_view/src/agent_view.rs
rtk git commit -m "feat: improve agent resource pool UI"
```

---

### Task 6: Fix `@` Completion Clipping

**Files:**
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Modify: `crates/ai_chat_view/src/input/mention.rs`
- Test: `crates/ai_chat_view/src/input/agent_input.rs`
- Test: `crates/ai_chat_view/src/input/mention.rs`

- [ ] **Step 1: Add mention provider behavior tests**

Add to `mention.rs` tests:

```rust
#[test]
fn mention_completion_trigger_text_accepts_at_character() {
    assert!(mention_completion_trigger_text("@"));
    assert!(mention_completion_trigger_text("p"));
    assert!(!mention_completion_trigger_text(" "));
}
```

The test uses this pure helper, which will be added in the implementation step:

```rust
pub(crate) fn mention_completion_trigger_text(new_text: &str) -> bool {
    new_text.chars().last().is_some_and(|c| !c.is_whitespace())
}
```

- [ ] **Step 2: Add GPUI bounds test for bottom clipping**

In `agent_input.rs` tests, add:

```rust
#[gpui::test]
fn mention_completion_popup_stays_above_bottom_toolbar(cx: &mut TestAppContext) {
    let cx: &mut VisualTestContext = cx;
    let (_, cx) = cx.add_window_view(|window, cx| AgentInputLayoutRoot::new(window, cx));

    let input = cx.debug_bounds("agent-input-root").expect("input root should render");
    let toolbar = cx.debug_bounds("agent-input-toolbar").expect("toolbar should render");

    assert!(
        input.origin.y < toolbar.origin.y,
        "input editor should have vertical room above toolbar for completion popup"
    );
}
```

Add debug selector to toolbar in `render_toolbar`:

```rust
.debug_selector(|| "agent-input-toolbar".to_string())
```

- [ ] **Step 3: Run tests to verify failure or missing selector**

Run:

```bash
rtk cargo test -p ai_chat_view mention_completion_trigger_text
rtk cargo test -p ai_chat_view mention_completion_popup_stays_above_bottom_toolbar
```

Expected: FAIL because helper and/or debug selector does not exist.

- [ ] **Step 4: Implement trigger helper and selector**

In `mention.rs`:

```rust
pub(crate) fn mention_completion_trigger_text(new_text: &str) -> bool {
    new_text.chars().last().is_some_and(|c| !c.is_whitespace())
}
```

Change provider:

```rust
fn is_completion_trigger(
    &self,
    _offset: usize,
    new_text: &str,
    _cx: &mut Context<InputState>,
) -> bool {
    mention_completion_trigger_text(new_text)
}
```

In `agent_input.rs`, ensure the input wrapper does not clip completion popover:

```rust
div()
    .w_full()
    .px_3()
    .pt_1()
    .max_h(px(220.0))
```

Remove `.overflow_hidden()` from the input wrapper that currently owns completion popover placement. Keep text editor growth bounded with the existing `InputState::auto_grow(3, 10)` configuration so the wrapper no longer clips the completion popup while the editor remains height-limited.

- [ ] **Step 5: Verify mention and layout tests**

Run:

```bash
rtk cargo test -p ai_chat_view mention_completion_trigger_text
rtk cargo test -p ai_chat_view mention_completion_popup_stays_above_bottom_toolbar
rtk cargo test -p ai_chat_view resource_builder
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/mention.rs crates/ai_chat_view/src/input/agent_input.rs
rtk git commit -m "fix: keep agent mention completions visible"
```

---

### Task 7: Final Verification

**Files:**
- No code edits.
- Verify modified crates and core behavior.

- [ ] **Step 1: Format**

Run:

```bash
rtk cargo fmt
```

Expected: exits 0.

- [ ] **Step 2: Run targeted tests**

Run:

```bash
rtk cargo test -p agent_runtime resource_scope
rtk cargo test -p agent_runtime tool_runtime_adapter
rtk cargo test -p ai_chat_view resource_builder
rtk cargo test -p ai_chat_view resource_pool_item_exposes_add_and_remove_state
rtk cargo test -p ai_chat_view mention_completion_trigger_text
rtk cargo test -p terminal_view sidebar
rtk cargo test -p main initial_layout_pins_home_and_ai_workbench_with_ai_active
```

Expected: all commands exit 0. Existing `block v0.1.6` future-incompat warnings may appear during build/check and should be reported as existing warnings.

- [ ] **Step 3: Run compile checks**

Run:

```bash
rtk cargo check -p agent_runtime
rtk cargo check -p ai_chat_view
rtk cargo check -p terminal_view
rtk cargo check -p main
```

Expected: all commands exit 0.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
rtk git diff --stat
rtk git status --short
```

Expected: only files touched by this plan and pre-existing unrelated dirty files are shown. Do not revert unrelated user changes.

- [ ] **Step 5: Commit verification-only formatting updates**

Run:

```bash
rtk git add crates/agent_runtime crates/ai_chat_view crates/terminal_view crates/db_view crates/redis_view crates/mongodb_view main/src/onetcli_app.rs
rtk git commit -m "chore: verify agent resource catalog scope"
```

Expected: create the commit only when `rtk git diff --cached --stat` shows staged formatting changes from this plan. When there are no staged changes, `rtk git commit` exits non-zero with "nothing to commit"; report that no verification-only commit was needed and continue.
