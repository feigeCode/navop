# Unified Tool Runtime Phase 5d Resource Source Presets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add explicit resource source presets so a resource pool can be populated from current, all, type-filtered, and later workspace/tag/manual sources without changing the core resource-pool semantics.

**Architecture:** `ResourceContext` remains the selected resource pool and `available_resources` remains the broader catalog. Phase 5d adds a small display and state contract that describes source presets, then wires preset selection to replace or derive the selected pool from the existing catalog. Workspace/tag persistence stays behind a later storage-backed step until a real workspace/tag catalog source is available.

**Tech Stack:** Rust, GPUI, `ai_chat_view`, `agent_runtime::ResourceContext`, `agent_runtime::ResourceRef`.

Current status: Phase 5d resource source preset checkpoint verified; tracking ready to commit.

---

## File Structure

Modify:

- `crates/ai_chat_view/src/input/context.rs`
  - Add `ComposerResourceSourceOption`, a pure display row for source presets.
  - Keep it free of `agent_runtime` and storage dependencies.

- `crates/ai_chat_view/src/input/mod.rs`
  - Re-export `ComposerResourceSourceOption`.

- `crates/ai_chat_view/src/agent_view.rs`
  - Add source option generation from current `ResourceContext` and `available_resources`.
  - Later handle source selection events and apply source presets to the selected pool.

- `crates/ai_chat_view/src/input/agent_input.rs`
  - Later render source preset controls inside the resource-pool popover.
  - Later emit source selection events.

- `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Track Phase 5d checkpoint status.

Out of scope for the first checkpoint:

- Persisted custom resource-pool presets.
- Workspace/tag-backed source expansion without a real workspace/tag resource catalog.
- Terminal sidebar catalog expansion without an all-connections source.
- Phase 6 parallel multi-resource execution.

## Source Preset Contract

Initial source ids:

```text
current  Keep or restore the current/default resource only.
pool     Keep the current selected resource pool.
all      Select every resource in the available catalog.
ssh      Select all SSH resources from the available catalog.
db       Select MySQL/Postgres/SQLite/Mongo resources from the available catalog.
redis    Select all Redis resources from the available catalog.
terminal Select all Terminal resources from the available catalog.
manual   Keep manual add/remove behavior; selected when no other preset exactly describes the pool.
```

`workspace` and `tag` remain planned ids, not visible enabled options, until the app passes real workspace/tag metadata into `ai_chat_view`.

## Task 1: Add Resource Source Display Model

**Files:**
- Modify: `crates/ai_chat_view/src/input/context.rs`
- Modify: `crates/ai_chat_view/src/input/mod.rs`

- [x] **Step 1: Write failing display-model test**

Add this test to `context.rs`:

```rust
#[test]
fn resource_source_option_has_stable_element_id_and_selection_state() {
    let selected = ComposerResourceSourceOption::new("all", "全部资源", 3, true);
    let disabled = ComposerResourceSourceOption::new("workspace", "工作区", 0, false)
        .disabled("暂无工作区资源来源");

    assert_eq!(selected.element_id().as_ref(), "resource-source-all");
    assert_eq!(selected.count, 3);
    assert!(selected.selected);
    assert!(selected.enabled);
    assert!(!disabled.enabled);
    assert_eq!(disabled.hint.as_ref().map(|s| s.as_ref()), Some("暂无工作区资源来源"));
}
```

Expected red result:

```text
cannot find type `ComposerResourceSourceOption` in this scope
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source_option_has_stable_element_id_and_selection_state
```

Expected: fail because `ComposerResourceSourceOption` is not defined.

- [x] **Step 3: Add minimal display model**

Add to `context.rs` after `ComposerResourceTypeFilter`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerResourceSourceOption {
    pub id: SharedString,
    pub label: SharedString,
    pub count: usize,
    pub selected: bool,
    pub enabled: bool,
    pub hint: Option<SharedString>,
}

impl ComposerResourceSourceOption {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        count: usize,
        selected: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count,
            selected,
            enabled: true,
            hint: None,
        }
    }

    pub fn disabled(mut self, hint: impl Into<SharedString>) -> Self {
        self.enabled = false;
        self.hint = Some(hint.into());
        self
    }

    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!("resource-source-{}", self.id))
    }
}
```

Add to `AgentComposerContext`:

```rust
pub resource_source_options: Vec<ComposerResourceSourceOption>,
```

Re-export it in `crates/ai_chat_view/src/input/mod.rs`.

- [x] **Step 4: Run test to verify it passes**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source_option_has_stable_element_id_and_selection_state
```

Expected: pass.

- [x] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/context.rs crates/ai_chat_view/src/input/mod.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md
rtk git commit -m "feat(ai_chat): add resource source option model"
```

## Task 2: Derive Source Options From Pool And Catalog

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md`

- [x] **Step 1: Write failing derivation tests**

Add tests near existing resource-pool helper tests in `agent_view.rs`:

```rust
#[test]
fn resource_source_options_mark_all_when_pool_matches_catalog() {
    let pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
    let catalog = pool.resources.clone();

    let options = resource_source_options(&pool, &catalog);

    assert!(source_option(&options, "all").selected);
    assert_eq!(source_option(&options, "all").count, 2);
    assert!(!source_option(&options, "current").selected);
}

#[test]
fn resource_source_options_mark_manual_for_mixed_subset() {
    let pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
    ];

    let options = resource_source_options(&pool, &catalog);

    assert!(source_option(&options, "manual").selected);
    assert_eq!(source_option(&options, "ssh").count, 2);
    assert_eq!(source_option(&options, "redis").count, 1);
}

fn source_option<'a>(
    options: &'a [ComposerResourceSourceOption],
    id: &str,
) -> &'a ComposerResourceSourceOption {
    options.iter().find(|option| option.id.as_ref() == id).unwrap()
}
```

Expected red result:

```text
cannot find function `resource_source_options` in this scope
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source_options_mark
```

Expected: fail because `resource_source_options` is missing.

- [x] **Step 3: Implement source option derivation**

Add a pure helper near `resource_type_filters` that:

1. Builds ids for current pool and available catalog.
2. Marks `current` selected when the pool has exactly one resource and that resource is the default target.
3. Marks `all` selected when pool ids equal catalog ids and catalog is non-empty.
4. Marks type presets selected when the pool exactly matches all catalog resources for that type.
5. Marks `manual` selected when no preset matches.
6. Adds disabled `workspace` and `tag` options with hints.

Update `build_composer_context` to populate `resource_source_options`.

- [x] **Step 4: Run tests to verify they pass**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source_options_mark
rtk cargo test -p ai_chat_view resource_pool
```

Expected: pass.

- [x] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md
rtk git commit -m "feat(ai_chat): derive resource source options"
```

## Task 3: Apply Source Presets To The Selected Pool

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`

- [x] **Step 1: Write failing pure mutation tests**

Add tests in `agent_view.rs`:

```rust
#[test]
fn apply_resource_source_all_replaces_pool_with_catalog() {
    let mut pool = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
    ];

    assert!(apply_resource_source(&mut pool, &catalog, "all"));
    assert_eq!(2, pool.resources.len());
    assert_eq!(Some("prod-a"), pool.current().map(|resource| resource.label.as_str()));
}

#[test]
fn apply_resource_source_ssh_selects_only_ssh_resources() {
    let mut pool = ResourceContext::new()
        .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
    let catalog = vec![
        ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
        ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
    ];

    assert!(apply_resource_source(&mut pool, &catalog, "ssh"));
    assert_eq!(1, pool.resources.len());
    assert_eq!(Some("prod-a"), pool.current().map(|resource| resource.label.as_str()));
}
```

Expected red result:

```text
cannot find function `apply_resource_source` in this scope
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
rtk cargo test -p ai_chat_view apply_resource_source_ssh_selects_only_ssh_resources
```

Expected: fail because `apply_resource_source` is missing.

- [x] **Step 3: Implement minimal source application**

Add an `AgentInputEvent::SelectResourceSource { id: SharedString }` event and handle it in
`AgentChatView::on_input_event`. The pure helper should:

1. Return `false` for disabled or unknown ids.
2. `current`: keep only current/default resource when present.
3. `pool` and `manual`: no-op.
4. `all`: replace pool with catalog.
5. Type ids: replace pool with matching catalog resources.
6. Preserve current default if it is still in the new pool; otherwise use first resource.

- [x] **Step 4: Run tests to verify they pass**

Run:

```bash
rtk cargo test -p ai_chat_view apply_resource_source
rtk cargo test -p ai_chat_view resource_pool
```

Expected: pass.

- [x] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs crates/ai_chat_view/src/input/agent_input.rs docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md
rtk git commit -m "feat(ai_chat): apply resource source presets"
```

## Task 4: Render Source Presets In Resource Pool Popover

**Files:**
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`

- [x] **Step 1: Add focused UI helper test**

Add a pure helper test for labels or enabled state, matching the existing style of
`resource_pool_action_labels_match_membership`.

- [x] **Step 2: Render source presets above type filters**

Render a compact row of source preset buttons in the resource pool popover:

```text
当前 / 资源池 / 全部 / SSH / DB / Redis / Terminal / 手动
```

Disabled workspace/tag options are not rendered in this checkpoint; they stay in the
context model until a real source exists.

- [x] **Step 3: Run focused tests and check**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source
rtk cargo test -p ai_chat_view resource_pool
rtk cargo check -p ai_chat_view
rtk git diff --check
```

Expected: pass, with only the existing `block v0.1.6` future-incompat warning if emitted.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/agent_input.rs docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md
rtk git commit -m "feat(ai_chat): render resource source presets"
```

## Task 5: Phase 5d Verification And Tracking

**Files:**
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
- Modify: `docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md`

- [x] **Step 1: Run verification**

Run:

```bash
rtk cargo test -p ai_chat_view resource_source
rtk cargo test -p ai_chat_view resource_pool
rtk cargo check -p ai_chat_view
rtk git diff --check
```

Expected: pass, with only the existing `block v0.1.6` future-incompat warning if emitted.

- [x] **Step 2: Update tracking**

Update the Phase 5d row in the design doc with commit hashes and the verification result.

- [ ] **Step 3: Commit tracking**

```bash
rtk git add docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5d-resource-source-presets.md
rtk git commit -m "docs: track resource source preset checkpoint"
```

## Manual Smoke

1. Open an Agent resource pool popover with multiple available DB/Redis/Mongo resources.
2. Select `全部` and confirm all available resources enter the pool.
3. Select `Redis` and confirm only Redis resources remain.
4. Select `手动`, add one more resource, and confirm the selection is treated as manual.
5. Confirm the default target is preserved when the selected resource remains in the new pool.
6. Confirm the default target moves to the first selected resource when the old default leaves the pool.

## Self-Review

Spec coverage:

1. Adds source preset tracking without replacing resource pool semantics.
2. Keeps workspace/tag/persistence deferred until real source metadata is available.
3. Preserves side-panel single-resource default behavior.

Placeholder scan:

1. No unfinished task uses placeholder implementation details.
2. Workspace/tag ids are explicitly planned but disabled until real metadata exists.

Type consistency:

1. Source options are display-only in `input/context.rs`.
2. Pool mutation remains in `AgentChatView`, next to existing add/remove/default target logic.
