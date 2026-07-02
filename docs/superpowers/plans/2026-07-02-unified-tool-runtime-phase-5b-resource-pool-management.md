# Unified Tool Runtime Phase 5b Resource Pool Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add explicit resource pool membership management so a session can keep a default target while adding or removing allowed resources from a broader resource catalog.

**Architecture:** `AgentChatViewConfig` gains an optional `available_resources` catalog. `AgentChatView` owns the mutable session `ResourceContext` as the selected resource pool, maps the pool plus catalog into pure `AgentInput` display rows, and handles add/remove/default-target events. `AgentInput` stays a dumb GPUI component: it renders rows and emits events without depending on `agent_runtime`.

**Tech Stack:** Rust, GPUI, `ai_chat_view`, `agent_runtime::ResourceContext`, `agent_runtime::ResourceRef`.

Current status: Planned.

---

## File Structure

Modify:

- `crates/ai_chat_view/src/input/context.rs`
  - Add `ComposerResourcePoolItem` for a resource catalog row.
  - Keep display structs free of `agent_runtime` types.

- `crates/ai_chat_view/src/input/agent_input.rs`
  - Add `AddResourceToPool` and `RemoveResourceFromPool` events.
  - Render pool membership rows with add/remove buttons.
  - Keep selecting a row as “set default target” when the row is already in the pool.

- `crates/ai_chat_view/src/agent_view.rs`
  - Add `available_resources` to `AgentChatViewConfig`.
  - Build resource pool item display data from `resources` and `available_resources`.
  - Handle add/remove events by updating `ResourceContext`, runtime session resources, target options, and composer context.

- `crates/ai_chat_view/src/resource_builder.rs`
  - Add helper to build a catalog from all saved connections while keeping side-panel pool single-resource by default.

- `crates/ai_chat_view/src/resource_builder_tests.rs`
  - Cover catalog behavior and default target preservation.

- `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Update Phase 5 row after verification.

Out of scope:

- Workspace/tag/all source UI selector.
- Persisting custom resource-pool membership.
- Multi-resource execution or parallel ToolRouter behavior.
- Replacing `agent_runtime::ResourceContext` with `tool_runtime::ResourcePool` in `AgentChatView` storage.

## Task 1: Add Display Rows For Resource Pool Membership

**Files:**
- Modify: `crates/ai_chat_view/src/input/context.rs`

- [x] **Step 1: Write failing display-model tests**

Add these tests to the `context.rs` test module:

```rust
#[test]
fn resource_pool_item_exposes_add_and_remove_state() {
    let in_pool = ComposerResourcePoolItem::new(
        "ssh-a",
        "prod-a",
        "SH",
        "ssh",
        "ssh · ssh-a",
        true,
        true,
    );
    let out_pool = ComposerResourcePoolItem::new(
        "ssh-b",
        "prod-b",
        "SH",
        "ssh",
        "ssh · ssh-b",
        false,
        false,
    );

    assert_eq!(in_pool.element_id().as_ref(), "resource-pool-item-ssh-a");
    assert!(in_pool.in_pool);
    assert!(in_pool.is_default);
    assert!(!out_pool.in_pool);
    assert!(!out_pool.is_default);
}
```

Expected red result:

```text
cannot find type `ComposerResourcePoolItem` in this scope
```

- [x] **Step 2: Add `ComposerResourcePoolItem`**

Insert after `ComposerResourceTypeFilter`:

```rust
/// 资源池候选行。`in_pool` 表示是否已授权进入本会话资源池。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerResourcePoolItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: SharedString,
    pub kind: SharedString,
    pub subtitle: SharedString,
    pub in_pool: bool,
    pub is_default: bool,
}

impl ComposerResourcePoolItem {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        kind: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        in_pool: bool,
        is_default: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: icon.into(),
            kind: kind.into(),
            subtitle: subtitle.into(),
            in_pool,
            is_default,
        }
    }

    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!("resource-pool-item-{}", self.id))
    }
}
```

Extend `AgentComposerContext`:

```rust
pub resource_pool_items: Vec<ComposerResourcePoolItem>,
```

Re-export `ComposerResourcePoolItem` in `crates/ai_chat_view/src/input/mod.rs`.

- [x] **Step 3: Run focused test**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_item_exposes_add_and_remove_state
```

Expected: pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/context.rs crates/ai_chat_view/src/input/mod.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): add resource pool item display model"
```

## Task 2: Add Available Resource Catalog To AgentChatViewConfig

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`

- [x] **Step 1: Write failing config tests**

Add tests in `agent_view.rs` test module:

```rust
#[test]
fn agent_config_defaults_available_resources_to_pool_resources() {
    let resources = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));

    let config = AgentChatViewConfig::new(test_runtime(), resources.clone(), Vec::new());

    assert_eq!(config.available_resources, resources.resources);
}

#[test]
fn agent_config_accepts_available_resource_catalog() {
    let pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
    ];

    let config = AgentChatViewConfig::new(test_runtime(), pool, Vec::new())
        .with_available_resources(catalog.clone());

    assert_eq!(config.available_resources, catalog);
}
```

Expected red result:

```text
no field `available_resources` on type `AgentChatViewConfig`
no method named `with_available_resources`
```

- [x] **Step 2: Add config field and builder**

Add to `AgentChatViewConfig`:

```rust
pub available_resources: Vec<ResourceRef>,
```

In `AgentChatViewConfig::new`:

```rust
let available_resources = resources.resources.clone();
```

Set field:

```rust
available_resources,
```

Add builder:

```rust
pub fn with_available_resources(mut self, resources: Vec<ResourceRef>) -> Self {
    self.available_resources = resources;
    self
}
```

When destructuring config in `AgentChatView::new`, capture:

```rust
let available_resources = config.available_resources;
```

Store it in `AgentChatView`:

```rust
available_resources: Vec<ResourceRef>,
```

- [x] **Step 3: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view agent_config_defaults_available_resources_to_pool_resources
rtk cargo test -p ai_chat_view agent_config_accepts_available_resource_catalog
```

Expected: both pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): add available resource catalog"
```

## Task 3: Map Pool And Catalog Into Membership Rows

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`

- [x] **Step 1: Write failing mapping tests**

Add tests in `agent_view.rs` test module:

```rust
#[test]
fn resource_pool_items_mark_pool_membership_and_default_target() {
    let pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
    ];

    let items = resource_pool_items(&pool, &catalog);

    assert_eq!(2, items.len());
    assert_eq!(items[0].id.as_ref(), "ssh-a");
    assert!(items[0].in_pool);
    assert!(items[0].is_default);
    assert_eq!(items[1].id.as_ref(), "ssh-b");
    assert!(!items[1].in_pool);
    assert!(!items[1].is_default);
}
```

Expected red result:

```text
cannot find function `resource_pool_items` in this scope
```

- [x] **Step 2: Add mapping helper**

Add helper near `resource_type_filters`:

```rust
fn resource_pool_items(
    pool: &ResourceContext,
    catalog: &[ResourceRef],
) -> Vec<ComposerResourcePoolItem> {
    let pool_ids = pool
        .resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let default_id = pool.current.clone();

    catalog
        .iter()
        .map(|resource| {
            let in_pool = pool_ids.contains(&resource.id);
            let is_default = default_id.as_ref() == Some(&resource.id);
            ComposerResourcePoolItem::new(
                resource.id.as_str().to_string(),
                resource.label.clone(),
                kind_icon(&resource.kind),
                resource.kind.as_str().to_string(),
                format!("{} · {}", resource.kind.as_str(), resource.id),
                in_pool,
                is_default,
            )
        })
        .collect()
}
```

Update `build_composer_context` to accept `available_resources: &[ResourceRef]` and set:

```rust
context.resource_pool_items = resource_pool_items(resources, available_resources);
```

Update all call sites to pass `&self.available_resources` or `&resources.resources` during initialization.

- [x] **Step 3: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_items_mark_pool_membership_and_default_target
rtk cargo test -p ai_chat_view build_context_marks_current_resource_as_default_target
```

Expected: both pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): map resource catalog to pool rows"
```

## Task 4: Add Resource Pool Add/Remove Events And Rendering

**Files:**
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`

- [x] **Step 1: Write failing pure event tests**

Add tests in `agent_input.rs`:

```rust
#[test]
fn resource_pool_action_labels_match_membership() {
    let add = crate::input::context::ComposerResourcePoolItem::new(
        "ssh-b", "prod-b", "SH", "ssh", "ssh · ssh-b", false, false,
    );
    let remove = crate::input::context::ComposerResourcePoolItem::new(
        "ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a", true, false,
    );
    let default = crate::input::context::ComposerResourcePoolItem::new(
        "ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a", true, true,
    );

    assert_eq!(resource_pool_action_label(&add), "+");
    assert_eq!(resource_pool_action_label(&remove), "-");
    assert_eq!(resource_pool_action_label(&default), "默认");
}
```

Expected red result:

```text
cannot find function `resource_pool_action_label` in this scope
```

- [x] **Step 2: Add events**

Extend `AgentInputEvent`:

```rust
AddResourceToPool { id: SharedString },
RemoveResourceFromPool { id: SharedString },
```

- [x] **Step 3: Add helper**

Add near label helpers:

```rust
fn resource_pool_action_label(item: &ComposerResourcePoolItem) -> &'static str {
    if item.is_default {
        "默认"
    } else if item.in_pool {
        "-"
    } else {
        "+"
    }
}
```

- [x] **Step 4: Render membership rows**

In `render_context_mode_content`, replace target list rows with `resource_pool_items` rows when `context.resource_pool_items` is not empty. Each row behavior:

```text
in_pool row click -> SelectTarget
out_pool row click -> AddResourceToPool
remove button on non-default in_pool row -> RemoveResourceFromPool
default row button -> no-op label "默认"
```

Keep existing `context_target_option` as a fallback for tests and contexts that do not provide pool rows.

- [x] **Step 5: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_action_labels_match_membership
rtk cargo test -p ai_chat_view resource_pool_trigger_label_uses_pool_wording
```

Expected: both pass.

- [x] **Step 6: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/agent_input.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): render resource pool membership actions"
```

## Task 5: Handle Add/Remove Resource Pool Events

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`

- [x] **Step 1: Write failing pure mutation tests**

Add tests in `agent_view.rs`:

```rust
#[test]
fn add_resource_to_pool_uses_catalog_resource() {
    let mut pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
    ];

    assert!(add_resource_to_pool(&mut pool, &catalog, "ssh-b"));
    assert_eq!(2, pool.resources.len());
    assert_eq!(Some("prod-a"), pool.current().map(|resource| resource.label.as_str()));
}

#[test]
fn remove_default_resource_reassigns_default_target() {
    let mut pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    assert!(remove_resource_from_pool(&mut pool, "ssh-a"));
    assert_eq!(1, pool.resources.len());
    assert_eq!(Some("prod-b"), pool.current().map(|resource| resource.label.as_str()));
}
```

Expected red result:

```text
cannot find function `add_resource_to_pool` in this scope
cannot find function `remove_resource_from_pool` in this scope
```

- [x] **Step 2: Add mutation helpers**

Add helpers near `select_target`:

```rust
fn add_resource_to_pool(pool: &mut ResourceContext, catalog: &[ResourceRef], id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    if pool.get(&rid).is_some() {
        return false;
    }
    let Some(resource) = catalog.iter().find(|resource| resource.id == rid).cloned() else {
        return false;
    };
    pool.resources.push(resource);
    if pool.current.is_none() {
        pool.current = Some(rid);
    }
    true
}

fn remove_resource_from_pool(pool: &mut ResourceContext, id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    let before = pool.resources.len();
    pool.resources.retain(|resource| resource.id != rid);
    if pool.resources.len() == before {
        return false;
    }
    if pool.current.as_ref() == Some(&rid) {
        pool.current = pool.resources.first().map(|resource| resource.id.clone());
    }
    true
}
```

- [x] **Step 3: Wire events**

Extend `on_input_event`:

```rust
AgentInputEvent::AddResourceToPool { id } => {
    if !self.is_running {
        self.add_resource_to_pool(&id, cx);
    }
}
AgentInputEvent::RemoveResourceFromPool { id } => {
    if !self.is_running {
        self.remove_resource_from_pool(&id, cx);
    }
}
```

Add methods:

```rust
fn add_resource_to_pool(&mut self, id: &str, cx: &mut Context<Self>) {
    if add_resource_to_pool(&mut self.resources, &self.available_resources, id) {
        self.sync_session_resources();
        self.sync_resource_targets(cx);
    }
}

fn remove_resource_from_pool(&mut self, id: &str, cx: &mut Context<Self>) {
    if remove_resource_from_pool(&mut self.resources, id) {
        self.sync_session_resources();
        self.sync_resource_targets(cx);
    }
}
```

Extract shared sync logic:

```rust
fn sync_session_resources(&self) {
    if let Some(session) = self.runtime.session(&self.session_id) {
        session.set_resources(self.resources.clone());
    }
}

fn sync_resource_targets(&self, cx: &mut Context<Self>) {
    let target_options = self
        .resources
        .resources
        .iter()
        .map(target_from_resource)
        .collect::<Vec<_>>();
    let ctx = build_composer_context(
        &self.resources,
        self.task_kind,
        &self.selected_tool,
        self.selected_model.as_ref(),
        self.transcript.latest_plan(),
        self.transcript.active_subagents(),
        self.backend,
        &self.acp_agents,
        self.current_acp_id.as_ref(),
        self.acp_connecting,
        self.acp.as_ref().map(|acp| acp.state()),
        &self.available_resources,
    );
    self.input.update(cx, |input, cx| {
        input.set_target_options(target_options, cx);
        input.set_context(ctx, cx);
    });
}
```

Use these helpers from `select_target` and `set_resource_context` where possible.

- [x] **Step 4: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view add_resource_to_pool_uses_catalog_resource
rtk cargo test -p ai_chat_view remove_default_resource_reassigns_default_target
rtk cargo test -p ai_chat_view resource_pool_items_mark_pool_membership_and_default_target
```

Expected: all pass.

- [x] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): handle resource pool membership changes"
```

## Task 6: Resource Builder Catalog Helpers

**Files:**
- Modify: `crates/ai_chat_view/src/resource_builder.rs`
- Modify: `crates/ai_chat_view/src/resource_builder_tests.rs`

- [x] **Step 1: Write failing catalog tests**

Add tests:

```rust
#[test]
fn connection_catalog_contains_all_saved_resources() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
    ];

    let catalog = build_resource_catalog(&conns);

    assert_eq!(2, catalog.len());
    assert_eq!("prod-a", catalog[0].label);
    assert_eq!("prod-b", catalog[1].label);
}

#[test]
fn agent_context_single_can_receive_all_resources_as_catalog() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
    ];
    let current = conns[0].clone();

    let (pool, mentions, catalog) = build_agent_context_single_with_catalog(&current, &conns);

    assert_eq!(1, pool.resources.len());
    assert_eq!(2, catalog.len());
    assert!(mentions.is_empty());
}
```

Expected red result:

```text
cannot find function `build_resource_catalog`
cannot find function `build_agent_context_single_with_catalog`
```

- [x] **Step 2: Add helpers**

Add:

```rust
pub fn build_resource_catalog(connections: &[StoredConnection]) -> Vec<ResourceRef> {
    connections.iter().map(connection_to_resource_ref).collect()
}

pub fn build_agent_context_single_with_catalog(
    connection: &StoredConnection,
    connections: &[StoredConnection],
) -> (ResourceContext, Vec<MentionItem>, Vec<ResourceRef>) {
    let (pool, mentions) = build_agent_context_single(connection);
    (pool, mentions, build_resource_catalog(connections))
}
```

Update exports in `crates/ai_chat_view/src/lib.rs` if needed.

- [x] **Step 3: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view connection_catalog_contains_all_saved_resources
rtk cargo test -p ai_chat_view agent_context_single_can_receive_all_resources_as_catalog
rtk cargo test -p ai_chat_view resource_builder
```

Expected: all pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/resource_builder.rs crates/ai_chat_view/src/resource_builder_tests.rs crates/ai_chat_view/src/lib.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "feat(ai_chat): build resource catalog for pool management"
```

## Task 7: Verification And Tracking

**Files:**
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
- Modify: `docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md`

- [ ] **Step 1: Run focused tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool
rtk cargo test -p ai_chat_view resource_type_filter
rtk cargo test -p ai_chat_view resource_builder
rtk cargo test -p ai_chat_view add_resource_to_pool
rtk cargo test -p ai_chat_view remove_default_resource
```

Expected: all pass.

- [ ] **Step 2: Run crate check**

Run:

```bash
rtk cargo check -p ai_chat_view
rtk git diff --check
```

Expected: `ai_chat_view` check exits 0. Existing `block v0.1.6` future-incompat warning can remain. `git diff --check` has no output.

- [ ] **Step 3: Update tracking**

Update Phase 5 row:

```text
Phase 5 Resource Pool UI | In progress | Resource pool wording, default target display, type filtering, and explicit add/remove pool membership are implemented in ai_chat_view. Focused tests and cargo check passed on 2026-07-02. | Next checkpoint: workspace/tag/all source selector and persisted resource-pool presets.
```

Update this plan status:

```text
Current status: Resource pool membership add/remove checkpoint verified.
```

- [ ] **Step 4: Commit**

```bash
rtk git add docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5b-resource-pool-management.md
rtk git commit -m "docs: track resource pool management checkpoint"
```

## Manual Smoke

1. Open an SSH side panel with a single current connection.
2. Confirm the resource pool starts with only the current connection and marks it as default.
3. Open resource pool popover.
4. Add a second SSH resource from the catalog.
5. Confirm both resources remain in the pool and the original default target remains default.
6. Select the second resource and confirm it becomes default.
7. Remove the first resource and confirm the second resource remains default.
8. Confirm running a prompt still uses the current default target when the user says “这台机器”.

## Self-Review

Spec coverage:

1. Adds explicit resource pool membership management.
2. Keeps default target separate from pool boundary.
3. Keeps side-panel default single-resource behavior while allowing a broader catalog.

Placeholder scan:

1. No placeholder tasks remain.
2. Each task includes files, tests, expected red failure, verification commands, and commit boundaries.

Type consistency:

1. `ComposerResourcePoolItem` is display-only.
2. `AgentChatViewConfig.available_resources` uses `ResourceRef` catalog data.
3. `ResourceContext.resources` remains the selected pool for this checkpoint.
