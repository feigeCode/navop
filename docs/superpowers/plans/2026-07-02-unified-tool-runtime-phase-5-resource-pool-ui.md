# Unified Tool Runtime Phase 5 Resource Pool UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename and reshape the AI resource selector from “上下文 / 当前资源” into a resource pool UI with an explicit default target, without changing the execution boundary or removing existing target selection behavior.

**Architecture:** Keep `agent_runtime::ResourceContext` as the migration bridge for this phase, but treat `current` as `default_target` in UI wording and prompt-facing display. `AgentChatView` remains the integration point that maps `ResourceContext` into pure `AgentInput` view models. `AgentInput` stays a dumb GPUI component and receives all resource pool display data through `AgentComposerContext`.

**Tech Stack:** Rust, GPUI, `ai_chat_view`, `agent_runtime::ResourceContext`, existing `tool_runtime::ResourcePool` bridge.

Current status: Resource pool wording/default-target/type-filter checkpoint verified.

---

## File Structure

Modify:

- `crates/ai_chat_view/src/input/context.rs`
  - Add pure display structs for resource pool summary and type filters.
  - Keep the file business-crate-free.

- `crates/ai_chat_view/src/agent_view.rs`
  - Convert `ResourceContext` into the new resource pool display model.
  - Preserve `SelectTarget` behavior as “set default target”.

- `crates/ai_chat_view/src/input/agent_input.rs`
  - Rename UI copy from context/current-context to resource pool/default target.
  - Add type filter state and filter target options by kind.
  - Keep existing search behavior.

- `crates/ai_chat_view/src/resource_builder.rs`
  - Keep sidebar single-connection default behavior.
  - Add tests that document `current` as default target, not pool boundary.

- `crates/ai_chat_view/src/resource_builder_tests.rs`
  - Add resource pool semantics tests.

- `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
  - Update Phase 5 tracking row after implementation and verification.

Out of scope:

- Multi-select add/remove resource management.
- Workspace/tag/all resource source picker.
- ToolRouter parallel execution.
- Replacing `agent_runtime::ResourceContext` storage with `tool_runtime::ResourcePool`.

These remain later Phase 5/6 checkpoints.

## Task 1: Add Resource Pool Display Model

**Files:**
- Modify: `crates/ai_chat_view/src/input/context.rs`

- [x] **Step 1: Add failing tests for pool summary and type filters**

Append these tests in `context.rs` test module:

```rust
#[test]
fn resource_pool_summary_defaults_to_empty_pool() {
    let summary = ComposerResourcePoolSummary::default();

    assert_eq!(summary.default_label.as_ref(), "无默认目标");
    assert_eq!(summary.total_resources, 0);
    assert_eq!(summary.default_target_id, None);
}

#[test]
fn resource_type_filter_keeps_stable_element_ids() {
    let filter = ComposerResourceTypeFilter::new("ssh", "SSH", 3, true);

    assert_eq!(filter.element_id().as_ref(), "resource-filter-ssh");
    assert_eq!(filter.label.as_ref(), "SSH");
    assert_eq!(filter.count, 3);
    assert!(filter.selected);
}
```

Expected red result:

```text
cannot find type `ComposerResourcePoolSummary` in this scope
cannot find type `ComposerResourceTypeFilter` in this scope
```

- [x] **Step 2: Add the display structs**

Insert after `ComposerTarget`:

```rust
/// 资源池摘要。`default_target_id` 只是默认目标,不是资源池边界。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerResourcePoolSummary {
    pub default_target_id: Option<SharedString>,
    pub default_label: SharedString,
    pub total_resources: usize,
}

impl Default for ComposerResourcePoolSummary {
    fn default() -> Self {
        Self {
            default_target_id: None,
            default_label: SharedString::from("无默认目标"),
            total_resources: 0,
        }
    }
}

impl ComposerResourcePoolSummary {
    pub fn new(
        default_target_id: Option<SharedString>,
        default_label: impl Into<SharedString>,
        total_resources: usize,
    ) -> Self {
        Self {
            default_target_id,
            default_label: default_label.into(),
            total_resources,
        }
    }
}

/// 资源类型筛选项。`id == "all"` 表示全部资源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerResourceTypeFilter {
    pub id: SharedString,
    pub label: SharedString,
    pub count: usize,
    pub selected: bool,
}

impl ComposerResourceTypeFilter {
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
        }
    }

    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!("resource-filter-{}", self.id))
    }
}
```

Extend `AgentComposerContext`:

```rust
pub resource_pool: ComposerResourcePoolSummary,
pub resource_type_filters: Vec<ComposerResourceTypeFilter>,
```

Update `Default` behavior by relying on `ComposerResourcePoolSummary::default()` and `Vec::new()`.

- [x] **Step 3: Run the focused test**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_summary_defaults_to_empty_pool
rtk cargo test -p ai_chat_view resource_type_filter_keeps_stable_element_ids
```

Expected: both pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/context.rs
rtk git commit -m "feat(ai_chat): add resource pool display model"
```

## Task 2: Build Resource Pool Display Data From ResourceContext

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`

- [x] **Step 1: Add failing tests for default target and type counts**

Add tests near the existing `target_from_resource` tests in `agent_view.rs`:

```rust
#[test]
fn build_context_marks_current_resource_as_default_target() {
    let resources = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

    let context = build_context(
        &resources,
        TaskKind::Agent,
        &SharedString::from("自动"),
        None,
    );

    assert_eq!(context.resource_pool.total_resources, 2);
    assert_eq!(
        context.resource_pool.default_target_id.as_ref().map(|id| id.as_ref()),
        Some("ssh-a")
    );
    assert_eq!(context.resource_pool.default_label.as_ref(), "prod-a");
}

#[test]
fn build_context_counts_resource_types_for_filters() {
    let resources = ResourceContext::new()
        .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
        .with_resource(ResourceRef::new("db-a", ResourceKind::Postgres, "prod-db"))
        .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));

    let context = build_context(
        &resources,
        TaskKind::Agent,
        &SharedString::from("自动"),
        None,
    );

    let filters = context
        .resource_type_filters
        .iter()
        .map(|filter| (filter.id.as_ref(), filter.count))
        .collect::<Vec<_>>();

    assert_eq!(
        vec![("all", 3), ("postgres", 1), ("redis", 1), ("ssh", 1)],
        filters
    );
}
```

Expected red result:

```text
no field `resource_pool` on type `AgentComposerContext`
no field `resource_type_filters` on type `AgentComposerContext`
```

- [x] **Step 2: Add mapping helpers**

Add helpers near `target_from_resource`:

```rust
fn resource_pool_summary(resources: &ResourceContext) -> ComposerResourcePoolSummary {
    let current = resources.current();
    ComposerResourcePoolSummary::new(
        current.map(|resource| SharedString::from(resource.id.as_str().to_string())),
        current
            .map(|resource| resource.label.clone())
            .unwrap_or_else(|| "无默认目标".to_string()),
        resources.resources.len(),
    )
}

fn resource_type_filters(resources: &ResourceContext) -> Vec<ComposerResourceTypeFilter> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for resource in &resources.resources {
        *counts.entry(resource.kind.as_str().to_string()).or_default() += 1;
    }

    let mut filters = vec![ComposerResourceTypeFilter::new(
        "all",
        "全部",
        resources.resources.len(),
        true,
    )];
    filters.extend(counts.into_iter().map(|(kind, count)| {
        ComposerResourceTypeFilter::new(kind.clone(), kind.to_uppercase(), count, false)
    }));
    filters
}
```

Update `build_context`:

```rust
resource_pool: resource_pool_summary(resources),
resource_type_filters: resource_type_filters(resources),
```

- [x] **Step 3: Run focused tests**

Run:

```bash
rtk cargo test -p ai_chat_view build_context_marks_current_resource_as_default_target
rtk cargo test -p ai_chat_view build_context_counts_resource_types_for_filters
```

Expected: both pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/agent_view.rs crates/ai_chat_view/src/input/context.rs
rtk git commit -m "feat(ai_chat): map resource context to pool display"
```

## Task 3: Rename Context UI To Resource Pool And Default Target

**Files:**
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`

- [x] **Step 1: Add failing label tests**

Add tests in `agent_input.rs`:

```rust
#[test]
fn resource_pool_trigger_label_uses_pool_wording() {
    let context = AgentComposerContext {
        resource_pool: ComposerResourcePoolSummary::new(
            Some(SharedString::from("ssh-a")),
            "prod-a",
            3,
        ),
        ..AgentComposerContext::default()
    };

    assert_eq!(resource_pool_trigger_label(&context).as_ref(), "资源池 · 3");
}

#[test]
fn resource_pool_trigger_label_handles_empty_pool() {
    assert_eq!(
        resource_pool_trigger_label(&AgentComposerContext::default()).as_ref(),
        "资源池"
    );
}
```

Expected red result:

```text
cannot find function `resource_pool_trigger_label` in this scope
```

- [x] **Step 2: Add trigger label helper**

Add near other label helpers:

```rust
fn resource_pool_trigger_label(context: &AgentComposerContext) -> SharedString {
    if context.resource_pool.total_resources == 0 {
        return SharedString::from("资源池");
    }
    SharedString::from(format!("资源池 · {}", context.resource_pool.total_resources))
}
```

- [x] **Step 3: Update visible copy**

Change:

```rust
.child(div().text_sm().truncate().child("上下文"))
```

to:

```rust
.child(div().text_sm().truncate().child(resource_pool_trigger_label(&self.context)))
```

Change popover group labels:

```rust
context_group_label("当前上下文", cx)
```

to:

```rust
context_group_label("默认目标", cx)
```

Change empty list copy:

```rust
"无可用目标"
```

to:

```rust
"资源池为空"
```

Change search miss copy:

```rust
"未匹配到目标"
```

to:

```rust
"未匹配到资源"
```

Change filter result helper output:

```rust
SharedString::from(format!("匹配到 {} 个目标", filtered.len()))
```

to:

```rust
SharedString::from(format!("匹配到 {} 个资源", filtered.len()))
```

- [x] **Step 4: Run focused tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool_trigger_label_uses_pool_wording
rtk cargo test -p ai_chat_view resource_pool_trigger_label_handles_empty_pool
rtk cargo test -p ai_chat_view target_search_matches_label_subtitle_and_kind_case_insensitively
```

Expected: all pass.

- [x] **Step 5: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/agent_input.rs
rtk git commit -m "feat(ai_chat): rename context selector to resource pool"
```

## Task 4: Add Resource Type Filtering To The Pool Popover

**Files:**
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Modify: `crates/ai_chat_view/src/input/context.rs`

- [x] **Step 1: Add failing pure filter tests**

Add tests in `agent_input.rs`:

```rust
#[test]
fn resource_type_filter_keeps_all_resources_for_all() {
    let targets = vec![
        ComposerTarget::new("ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a"),
        ComposerTarget::new("db-a", "prod-db", "DB", "postgres", "postgres · db-a"),
    ];

    let filtered = filter_targets_by_kind(targets.clone(), "all");

    assert_eq!(filtered, targets);
}

#[test]
fn resource_type_filter_matches_target_kind() {
    let targets = vec![
        ComposerTarget::new("ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a"),
        ComposerTarget::new("db-a", "prod-db", "DB", "postgres", "postgres · db-a"),
    ];

    let filtered = filter_targets_by_kind(targets, "ssh");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id.as_ref(), "ssh-a");
}
```

Expected red result:

```text
cannot find function `filter_targets_by_kind` in this scope
```

- [x] **Step 2: Add selected kind state**

Add to `AgentInput`:

```rust
selected_resource_kind_filter: SharedString,
```

Initialize in constructors:

```rust
selected_resource_kind_filter: SharedString::from("all"),
```

When `set_target_options` receives an empty list, reset to `all`:

```rust
if options.is_empty() {
    self.selected_resource_kind_filter = SharedString::from("all");
}
```

- [x] **Step 3: Add pure filter helper**

Add near search helpers:

```rust
fn filter_targets_by_kind(targets: Vec<ComposerTarget>, kind: &str) -> Vec<ComposerTarget> {
    if kind == "all" {
        return targets;
    }
    targets
        .into_iter()
        .filter(|target| target.kind.as_ref() == kind)
        .collect()
}
```

- [x] **Step 4: Render filter controls above the search results**

Add a helper that renders compact filter rows:

```rust
fn render_resource_type_filters(
    view: Entity<AgentInput>,
    filters: Vec<ComposerResourceTypeFilter>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = cx.theme().list_hover;
    let selected_bg = cx.theme().accent;
    let selected_fg = cx.theme().accent_foreground;
    let muted = cx.theme().muted_foreground;

    h_flex()
        .w_full()
        .px_1()
        .pb_1()
        .gap(px(4.0))
        .children(filters.into_iter().map(|filter| {
            let id = filter.id.clone();
            let selected = filter.selected;
            h_flex()
                .id(filter.element_id())
                .items_center()
                .gap(px(4.0))
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .when(selected, |this| this.bg(selected_bg).text_color(selected_fg))
                .when(!selected, |this| this.text_color(muted).hover(move |s| s.bg(hover_bg)))
                .child(filter.label)
                .child(format!("{}", filter.count))
                .on_click(move |_, _window, cx| {
                    let id = id.clone();
                    view.update(cx, |this, cx| {
                        this.selected_resource_kind_filter = id;
                        cx.notify();
                    });
                })
        }))
        .into_any_element()
}
```

Before filtering by search, mark selected filter and apply kind filtering:

```rust
let selected_kind = self.selected_resource_kind_filter.clone();
let filters = self
    .context
    .resource_type_filters
    .iter()
    .cloned()
    .map(|mut filter| {
        filter.selected = filter.id == selected_kind;
        filter
    })
    .collect::<Vec<_>>();
```

Pass `selected_kind` and `filters` into `render_context_mode_content`. Inside it:

```rust
col = col.child(render_resource_type_filters(view.clone(), filters, cx));
let kind_filtered = filter_targets_by_kind(options, selected_kind.as_ref());
let filtered: Vec<ComposerTarget> = kind_filtered
    .into_iter()
    .filter(|opt| needle.is_empty() || target_matches(opt, &needle))
    .collect();
```

- [x] **Step 5: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_type_filter_keeps_all_resources_for_all
rtk cargo test -p ai_chat_view resource_type_filter_matches_target_kind
rtk cargo test -p ai_chat_view target_search_query_ignores_surrounding_whitespace
```

Expected: all pass.

- [x] **Step 6: Commit**

```bash
rtk git add crates/ai_chat_view/src/input/agent_input.rs crates/ai_chat_view/src/input/context.rs
rtk git commit -m "feat(ai_chat): filter resource pool by type"
```

## Task 5: Document Sidebar Default Target Semantics

**Files:**
- Modify: `crates/ai_chat_view/src/resource_builder_tests.rs`
- Modify: `crates/ai_chat_view/src/resource_builder.rs`

- [x] **Step 1: Add resource pool semantics tests**

Add tests:

```rust
#[test]
fn single_connection_sets_connection_as_default_target() {
    let conn = stored_connection(42, "prod-a", ConnectionType::SshSftp, "{}");

    let ctx = build_resource_context_single(&conn);

    assert_eq!(1, ctx.resources.len());
    assert_eq!(Some("prod-a"), ctx.current().map(|resource| resource.label.as_str()));
}

#[test]
fn all_connections_keep_all_resources_when_default_is_selected() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
        stored_connection(3, "prod-db", ConnectionType::Database, r#"{"type":"mysql"}"#),
    ];

    let ctx = build_resource_context_all(Some(&conns[1]), conns);

    assert_eq!(3, ctx.resources.len());
    assert_eq!(Some("prod-b"), ctx.current().map(|resource| resource.label.as_str()));
    assert!(ctx.resources.iter().any(|resource| resource.label == "prod-a"));
    assert!(ctx.resources.iter().any(|resource| resource.label == "prod-db"));
}
```

Expected: these should pass without production code changes. If they fail, fix `build_resource_context_all` without removing non-current resources.

- [x] **Step 2: Update comments**

Change comments in `resource_builder.rs` from “当前连接” language to “默认目标” where they describe `current`, for example:

```rust
/// 从所有连接构建 ResourceContext，并设置默认目标（用于非侧边栏模式）。
```

Do not rename public functions in this task.

- [x] **Step 3: Run tests**

Run:

```bash
rtk cargo test -p ai_chat_view single_connection_sets_connection_as_default_target
rtk cargo test -p ai_chat_view all_connections_keep_all_resources_when_default_is_selected
rtk cargo test -p ai_chat_view resource_builder
```

Expected: all pass.

- [x] **Step 4: Commit**

```bash
rtk git add crates/ai_chat_view/src/resource_builder.rs crates/ai_chat_view/src/resource_builder_tests.rs
rtk git commit -m "test(ai_chat): document resource pool default target semantics"
```

## Task 6: Phase 5 Checkpoint Verification And Tracking

**Files:**
- Modify: `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`
- Modify: `docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5-resource-pool-ui.md`

- [x] **Step 1: Run focused UI and builder tests**

Run:

```bash
rtk cargo test -p ai_chat_view resource_pool
rtk cargo test -p ai_chat_view resource_type_filter
rtk cargo test -p ai_chat_view resource_builder
rtk cargo test -p ai_chat_view target_search
```

Expected: all pass.

- [x] **Step 2: Run crate check**

Run:

```bash
rtk cargo check -p ai_chat_view
```

Expected: exit 0. Existing workspace warnings outside this crate can remain if they are not introduced by this phase.

- [x] **Step 3: Update design tracking**

Update the Phase 5 row in `docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md`:

```text
Phase 5 Resource Pool UI | In progress | Resource pool wording, default target display, search, and type filtering are implemented in ai_chat_view. Focused ai_chat_view tests and cargo check passed on 2026-07-02. | Next checkpoint: add explicit add/remove resource pool management and workspace/tag/all source selection.
```

- [x] **Step 4: Mark this plan checkpoint**

At the top of this plan, update a status line:

```text
Current status: Resource pool wording/default-target/type-filter checkpoint verified.
```

- [x] **Step 5: Commit**

```bash
rtk git add docs/superpowers/specs/2026-07-02-unified-tool-runtime-design.md docs/superpowers/plans/2026-07-02-unified-tool-runtime-phase-5-resource-pool-ui.md
rtk git commit -m "docs: track resource pool ui checkpoint"
```

## Final Verification

Run before handing off:

```bash
rtk cargo test -p ai_chat_view resource_pool
rtk cargo test -p ai_chat_view resource_type_filter
rtk cargo test -p ai_chat_view resource_builder
rtk cargo test -p ai_chat_view target_search
rtk cargo check -p ai_chat_view
rtk git diff --check
```

Expected:

```text
All selected ai_chat_view tests pass.
ai_chat_view check exits 0.
git diff --check has no output.
```

## Manual Smoke

Manual UI smoke after the code checkpoint:

1. Open an Agent side panel from a single SSH connection.
2. Confirm the top selector says `资源池` and the popover shows `默认目标`.
3. Confirm the pool contains only the current SSH connection in side-panel mode.
4. Open the normal Agent tab with multiple saved connections.
5. Confirm search filters by label/subtitle/kind.
6. Confirm type filters show counts for SSH / DB / Redis resources.
7. Select another resource and confirm it becomes the default target without removing the other resources from the pool.

## Self-Review

Spec coverage:

1. Covers Phase 5 wording, default target display, search, type filtering, and sidebar single-resource default.
2. Defers multi-select add/remove and workspace/tag source selection as a later Phase 5 checkpoint.
3. Does not change Agent tool routing or ToolRouter semantics.

Placeholder scan:

1. No placeholder tasks remain.
2. Each task includes exact files, focused tests, expected failures, verification commands, and commit boundaries.

Type consistency:

1. `ComposerResourcePoolSummary` and `ComposerResourceTypeFilter` live in `input/context.rs`.
2. `AgentComposerContext` carries display-only resource pool fields.
3. `AgentChatView` remains responsible for mapping `ResourceContext` to display models.
