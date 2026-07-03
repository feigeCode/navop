# Sidebar Contributions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a production-ready lightweight sidebar contribution system around the existing split tab container, preserving current service behavior and limiting split support to opted-in work tab contents.

**Architecture:** `SplitTabContainer` owns split panes, and each pane's `TabContainer` renders sidebar panels contributed by that pane's active tab inside `render_tab_content`. Service views keep ownership of their sidebar entities and opt into external rendering mode incrementally. Split capability is a separate `TabContent::can_split` contract for top-level work tabs such as Terminal, SSH/SFTP, Database, Redis, MongoDB, RDP, and VNC; Home remains unsplittable.

**Tech Stack:** Rust 2024, GPUI entities/rendering, `gpui_component` icons, native fixed-flex sidebar docks with local mouse-tracking resize, existing `TabContent`/`TabContentView` dynamic tab model.

---

### Task 1: Core Sidebar Contract

**Files:**
- Create: `crates/core/src/sidebar_contribution.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tab_container.rs`

- [x] **Step 1: Add failing contract tests**

Create unit tests in `crates/core/src/sidebar_contribution.rs` under `#[cfg(test)]` that verify:

```rust
#[test]
fn default_policy_allows_right_hideable_movable_panel() {
    let policy = SidebarPanelPolicy::default();
    assert!(policy.hideable);
    assert!(policy.movable);
    assert!(policy.allowed_placements.contains(SidebarPlacement::Right));
    assert!(policy.initially_visible);
}

#[test]
fn placement_set_rejects_disallowed_placements() {
    let set = SidebarPlacementSet::left_right();
    assert!(set.contains(SidebarPlacement::Left));
    assert!(set.contains(SidebarPlacement::Right));
    assert!(!set.contains(SidebarPlacement::Bottom));
}
```

Run: `rtk cargo test -p one-core sidebar_contribution`

Expected before implementation: compile failure because the module and types do not exist.

- [x] **Step 2: Implement minimal sidebar contribution types**

Add:

```rust
pub enum SidebarPlacement { Left, Right, Bottom }
pub struct SidebarPlacementSet { pub left: bool, pub right: bool, pub bottom: bool }
pub struct SidebarPanelPolicy { pub hideable: bool, pub movable: bool, pub allowed_placements: SidebarPlacementSet, pub initially_visible: bool }
pub struct SidebarPanelId { pub owner: EntityId, pub local_id: &'static str }
pub enum SidebarPanelChrome {
    Host,
    None,
}

pub struct SidebarContributionActions {
    pub close: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub move_to: Option<Arc<dyn Fn(SidebarPlacement, &mut Window, &mut App) + 'static>>,
}

pub struct SidebarContribution {
    pub id: SidebarPanelId,
    pub title: SharedString,
    pub icon: IconName,
    pub view: AnyView,
    pub default_placement: SidebarPlacement,
    pub policy: SidebarPanelPolicy,
    pub style: SidebarPanelStyle,
    pub size: SidebarPanelSize,
    pub chrome: SidebarPanelChrome,
    pub actions: SidebarContributionActions,
}
```

Expose the module from `crates/core/src/lib.rs`.

- [x] **Step 3: Extend tab content contracts**

Add default methods to `TabContent` and forwarding methods to `TabContentView`:

```rust
fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> { Vec::new() }
fn can_split(&self, cx: &App) -> bool { false }
```

- [x] **Step 4: Verify green**

Run: `rtk cargo test -p one-core sidebar_contribution`

Expected after implementation: the sidebar contribution tests pass.

### Task 2: Split Capability Gating

**Files:**
- Modify: `crates/core/src/tab_container.rs`
- Test: existing one-core tests plus new unit tests where possible

- [x] **Step 1: Add failing tests for default split capability**

Add a small test-only `TabContent` implementation in `tab_container.rs` tests that verifies default `can_split` is false through the `TabContentView` forwarding path.

Run: `rtk cargo test -p one-core tab_container_split_capability`

Expected before implementation: failure because `can_split` forwarding does not exist.

- [x] **Step 2: Gate split commands**

Change the tab context menu so `Split Right` and `Split Down` are shown or enabled only when:

```rust
self.split_enabled && tab.content().can_split(cx)
```

The split drag drop path must also reject `SplitRequested` when the dragged tab content does not allow split.

- [x] **Step 3: Verify green**

Run: `rtk cargo test -p one-core tab_container_split_capability`

Expected: tests pass.

### Task 3: TabContainer Sidebar State

**Files:**
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/core/src/lib.rs`

- [x] **Step 1: Add failing state reconciliation tests**

Add tests for:

```rust
#[test]
fn override_falls_back_when_placement_is_disallowed() { ... }

#[test]
fn hidden_non_hideable_panel_is_forced_visible() { ... }
```

Run: `rtk cargo test -p one-core sidebar`

Expected before implementation: compile failure because sidebar state helpers do not exist.

- [x] **Step 2: Implement state-only reconciliation**

Implement:

```rust
pub struct SidebarPanelOverride { ... }
pub struct ResolvedSidebarPanel { ... }
```

Keep rendering out of this task. The state API resolves contributions to visible panels with valid placement.

- [x] **Step 3: Verify green**

Run: `rtk cargo test -p one-core sidebar`

Expected: state reconciliation tests pass.

### Task 4: TabContainer Sidebar Rendering

**Files:**
- Modify: `crates/core/src/tab_container.rs`
- Modify: `main/src/onetcli_app.rs`

- [x] **Step 1: Render sidebars inside TabContainer**

Render sidebar docks from `TabContainer::render_tab_content`, below the tab bar and inside the pane bounds.

- [x] **Step 2: Render active tab contributions**

`TabContainer` reads:

```rust
active_tab.content().sidebar_contributions(cx)
```

and renders left/right/bottom docks around the active tab content with native fixed-flex containers.

- [x] **Step 3: Add move/hide controls**

Each rendered contribution gets host-level controls for allowed placements and hide.

- [x] **Step 4: Verify compile**

Run: `rtk cargo check -p main`

Expected: compile succeeds.

### Task 5: Terminal External Sidebar Mode

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `main/src/home/home_tabs.rs`

- [x] **Step 1: Add external sidebar render mode**

Add `TerminalSidebarRenderMode::{Embedded, External}`. Default constructors preserve embedded mode. Main app terminal creation uses external mode.

- [x] **Step 2: Contribute terminal toolbar and tool panels**

`TerminalView::sidebar_contributions` contributes:

```rust
terminal.toolbar
open terminal.settings / terminal.quick-command / terminal.ai-chat / terminal.file-manager / terminal.server-monitor panels
```

The toolbar is non-hideable, non-movable, chrome-less, and always visible while
the terminal tab is active. Tool panels use contribution actions so close and
move operations update `TerminalToolDockState` instead of hiding the entire
terminal sidebar.

- [x] **Step 3: Opt terminal into split**

`TerminalView::can_split` returns `true`.

- [x] **Step 4: Verify terminal/main compile**

Run: `rtk cargo check -p terminal_view`
Run: `rtk cargo check -p main`

Expected: both compile.

### Task 6: Database, Redis, MongoDB External Sidebar Mode

**Files:**
- Modify: `crates/db_view/src/database_tab.rs`
- Modify: `crates/redis_view/src/redis_tab.rs`
- Modify: `crates/mongodb_view/src/mongo_tab.rs`
- Modify: `main/src/home/home_tabs.rs`

- [x] **Step 1: Add external render mode per service**

Each service keeps the current embedded layout as default and adds center-only rendering for external mode.

- [x] **Step 2: Contribute existing panels**

Database contributes `database.tree` and `database.sidebar`.
Redis contributes `redis.tree` and `redis.sidebar`.
MongoDB contributes `mongodb.tree` and `mongodb.sidebar`.

- [x] **Step 3: Switch main app service creation to external mode**

Only top-level app-created service tabs use external mode.

- [x] **Step 4: Verify service crates**

Run:

```bash
rtk cargo check -p db_view
rtk cargo check -p redis_view
rtk cargo check -p mongodb_view
rtk cargo check -p main
```

Expected: all compile.

### Task 7: Final Verification

**Files:**
- All touched files

- [x] **Step 1: Format**

Run: `rtk cargo fmt`

- [x] **Step 2: Run focused tests**

Run:

```bash
rtk cargo test -p one-core sidebar
rtk cargo test -p one-core tab_container_split_capability
rtk cargo test -p main
```

- [x] **Step 3: Run compile checks**

Run:

```bash
rtk cargo check -p one-core
rtk cargo check -p terminal_view
rtk cargo check -p db_view
rtk cargo check -p redis_view
rtk cargo check -p mongodb_view
rtk cargo check -p main
```

- [x] **Step 4: Diff sanity**

Run: `rtk git diff --check`

Expected: no whitespace errors.

### Task 8: Terminal Tool Dock and Split Visibility Corrections

**Files:**
- Modify: `crates/core/src/sidebar_contribution.rs`
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/core/src/split_tab_container.rs`
- Modify: `crates/core/src/tab_container_split_tests.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Add terminal tool dock state tests**

Add pure state tests for:

```rust
tool_dock_keeps_toolbar_visible_when_no_panel_is_open
tool_dock_can_keep_multiple_tools_open_at_different_edges
tool_dock_closes_one_panel_without_hiding_toolbar_or_other_panels
```

Run: `rtk cargo test -p terminal_view tool_dock -- --nocapture`

Expected before implementation: compile failure because `TerminalToolDockState`
does not exist.

- [x] **Step 2: Implement per-tool dock state**

Replace the terminal sidebar's single active panel model with
`TerminalToolDockState`, preserving compatibility methods such as
`active_panel()` and `set_active_panel()` for existing shortcuts and embedded
rendering.

- [x] **Step 3: Add contribution actions**

Add `SidebarContributionActions` with optional `close` and `move_to` callbacks.
Host sidebar buttons call those callbacks when present and use local
TabContainer overrides only when no callback is provided.

- [x] **Step 4: Split terminal toolbar from tool panels**

Add stable wrapper views for `TerminalSidebarToolbar` and
`TerminalSidebarToolPanel`. Terminal tabs contribute one persistent
`terminal.toolbar` and one contribution per open tool panel.

- [x] **Step 5: Add split visibility regression test**

Add:

```rust
split_tree_visibility_follows_layout_not_active_tab_capability
```

Run: `rtk cargo test -p one-core split -- --nocapture`

Expected before implementation: compile failure because the visibility helper
does not exist.

- [x] **Step 6: Keep split tree independent from active tab capability**

`SplitTabContainer` renders the split tree whenever the split tree exists.
`TabContent::can_split` gates only split commands and drag/drop split requests;
it must not hide secondary panes. Home remains unsplittable, but selecting Home
inside the primary pane keeps existing secondary groups visible.

- [x] **Step 7: Verify corrections**

Run:

```bash
rtk cargo fmt
rtk cargo test -p terminal_view tool_dock -- --nocapture
rtk cargo test -p one-core sidebar -- --nocapture
rtk cargo test -p one-core split -- --nocapture
rtk cargo check -p one-core
rtk cargo check -p terminal_view
rtk cargo check -p db_view
rtk cargo check -p redis_view
rtk cargo check -p mongodb_view
rtk cargo check -p main
rtk cargo test -p main
rtk git diff --check
```

Expected: all commands succeed, aside from the known `block v0.1.6`
future-incompat warning on cargo check.

### Task 9: Work Tab Split Capability and Main Tab Routing

**Files:**
- Modify: `main/src/onetcli_app.rs`
- Modify: `main/src/home/home_tabs.rs`
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/db_view/src/database_tab.rs`
- Modify: `crates/redis_view/src/redis_tab.rs`
- Modify: `crates/mongodb_view/src/mongo_tab.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Route Home-opened tabs to the primary pane**

Add `GlobalTabContainer::primary_pane()` and make `HomePage::active_tab_container`
return the primary pane instead of `active_pane`. New connections and utility
tabs opened from Home therefore land in the main tab bar even when a secondary
split pane has focus.

- [x] **Step 2: Keep split pane tab bars**

Retain one `TabContainer` per split pane. Each split pane keeps its own tab bar,
matching VSCode editor groups and preserving close, switch, drag, and further
split operations.

- [x] **Step 3: Opt top-level service tabs into split**

Set `can_split() -> true` on `DatabaseTabView`, `RedisTabView`,
`MongoTabView`, `SftpView`, and `RemoteDesktopView`, in addition to the
existing terminal support. Home remains unsplittable through the default
`TabContent::can_split() -> false`.

- [x] **Step 4: Emit layout change when pinned Home activates**

`TabContainer::activate_pinned_tab` emits `TabContainerEvent::LayoutChanged` so
`SplitTabContainer` recomputes split visibility when switching back to Home.

### Task 10: VSCode-Style Split Pane Lifecycle

**Status:** Done.

**Goal:** Match VSCode editor group behavior when the primary group is empty:
secondary panes must remain visible as long as any split pane still has tabs.

**Files:**
- Modify: `crates/core/src/split_tab_container.rs`
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/core/src/tab_container_split_tests.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Add failing split visibility tests**

Add unit tests covering:

```rust
split_tree_stays_visible_for_home_when_primary_still_has_regular_tabs
split_tree_stays_visible_when_primary_regular_tabs_are_empty_and_secondary_has_tabs
split_tree_collapses_only_when_all_secondary_panes_are_empty
```

Run:

```bash
rtk cargo test -p one-core split -- --nocapture
```

Expected before implementation: the Home-active case fails because current
visibility is tied to the primary pane's effective active tab capability.

- [x] **Step 2: Keep split tree pruning separate from split tree visibility**

Use pruning to remove empty secondary panes:

```rust
fn prune_node(node: SplitNode, primary_pane: &Entity<TabContainer>, cx: &App) -> Option<SplitNode>
```

Visibility should not re-check tab capability or active content. Once pruning
has removed empty secondary panes, `root.is_split()` is the source of truth for
whether the split tree should render.

- [x] **Step 3: Change split tree visibility rule**

Replace the current visibility rule with:

```rust
render_split_tree = split_tree_exists
```

This preserves the desired Home behavior:

- Home selected while primary still has a regular work tab behind it: keep the
  split tree visible and render Home inside the primary group.
- Primary regular tabs all closed while secondary panes still contain work tabs:
  keep the split tree visible so secondary panes remain reachable.
- All secondary panes empty: prune back to primary only.

- [x] **Step 4: Verify**

Run:

```bash
rtk cargo test -p one-core split -- --nocapture
rtk cargo check -p one-core
rtk cargo check -p main
```

Expected: split tests pass and checks report zero errors.

### Task 11: Service Sidebar Defaults and Header Chrome

**Status:** Done.

**Goal:** Make service sidebars match expected startup ergonomics:
tree/sidebar navigation is available by default, tools/AI sidebars are not
opened by default, and tree panel headers can be hidden so the tree starts
without an extra title row.

**Files:**
- Modify: `crates/core/src/sidebar_contribution.rs`
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/core/src/sidebar_contribution_tests.rs`
- Modify: `crates/db_view/src/database_tab.rs`
- Modify: `crates/redis_view/src/redis_tab.rs`
- Modify: `crates/mongodb_view/src/mongo_tab.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Add chrome variant for hidden host header**

Extend sidebar chrome:

```rust
pub enum SidebarPanelChrome {
    Host,
    HostNoHeader,
    None,
}
```

`HostNoHeader` keeps the host frame, border, resize handle, and sizing behavior,
but skips `render_sidebar_panel_header`.

- [x] **Step 2: Apply headerless chrome to tree panels**

Set these tree contributions to `SidebarPanelChrome::HostNoHeader`:

```rust
database.tree
redis.tree
mongodb.tree
```

Tree panels remain non-hideable and visible by default. The requested "树的头部
需要支持隐藏" is handled at the contribution chrome level, not by changing the
tree view internals.

- [x] **Step 3: Keep service toolbars visible with panels collapsed**

Set these contributions to `initially_visible: true` so the service-owned
toolbar remains visible:

```rust
database.sidebar
redis.sidebar
mongodb.sidebar
```

They remain hideable/movable. Their internal AI/tool panel stays collapsed
because each service sidebar starts with `active_panel: None`, so opening a
database-like tab shows the narrow toolbar rather than the expanded AI panel.

- [x] **Step 4: Add state tests**

Add tests proving:

```rust
hideable_panel_with_initially_visible_false_starts_hidden
host_no_header_panel_still_renders_frame_without_header_controls
non_hideable_tree_panel_remains_visible
```

Use pure state tests where possible. For rendering behavior, test the chrome
branch with a small structural helper instead of relying on brittle visual
pixel checks.

- [x] **Step 5: Verify**

Run:

```bash
rtk cargo test -p one-core sidebar -- --nocapture
rtk cargo check -p db_view
rtk cargo check -p redis_view
rtk cargo check -p mongodb_view
rtk cargo check -p main
```

Expected: all commands succeed; service toolbars are visible by default and
service tool panels no longer open by default.

### Task 12: Split Tab Routing and Toolbar Semantics Review

**Status:** Done.

**Goal:** Confirm and codify routing semantics so future changes do not regress
the tab model.

**Files:**
- Modify: `main/src/home/home_tabs.rs`
- Modify: `main/src/onetcli_app.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`
- Test as needed in `main/src/home/home_tabs.rs` or a focused pure helper.

- [x] **Step 1: Codify new-tab routing**

New connections and Home-created tabs must route to the primary pane:

```rust
HomePage::active_tab_container -> GlobalTabContainer::primary_pane()
```

Do not route Home actions to `active_pane`, because that sends new connections
into secondary groups when a split pane has focus.

- [x] **Step 2: Preserve pane tab bars**

Keep one `TabContainer` per pane. Do not remove secondary pane tab bars. This is
required for VSCode-style editor groups:

- switch tabs inside a group
- close tabs inside a group
- drag tabs between groups
- further split from a group

A later visual polish task may compact the tab bar when a group has one tab,
but the interaction surface stays.

- [x] **Step 3: Verify by inspection and tests**

Run:

```bash
rtk rg -n "active_pane\\(|primary_pane\\(" main/src/home/home_tabs.rs main/src/onetcli_app.rs
rtk cargo check -p main
```

Expected: Home tab creation uses primary pane only; no new dead-code warnings
are introduced.

### Task 13: Sidebar Placement Exclusivity

**Status:** Done.

**Goal:** Enforce the UX rule that each sidebar placement shows at most one
visible host sidebar panel. Opening or moving a panel into a placement closes
the existing hideable panel there instead of stacking panels.

**Files:**
- Modify: `crates/core/src/tab_container.rs`
- Modify: `crates/core/src/sidebar_contribution_tests.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Add failing exclusivity tests**

Add pure state tests covering:

```rust
hideable_host_panel_closes_when_another_panel_targets_same_position
chrome_less_toolbar_does_not_use_exclusive_sidebar_slot
non_hideable_host_panel_blocks_target_position
tool_dock_opening_tool_closes_existing_tool_at_same_edge
tool_dock_moving_tool_to_occupied_edge_closes_existing_tool
```

Run:

```bash
rtk cargo test -p one-core sidebar -- --nocapture
rtk cargo test -p terminal_view tool_dock -- --nocapture
```

Expected before implementation: one-core fails for missing helper functions,
and terminal tool dock fails because two tools can occupy `Right`.

- [x] **Step 2: Implement TabContainer placement exclusivity**

Before showing or moving a local host panel into a placement:

- hide other visible hideable host panels already in that placement
- reject the move/show if a non-hideable host panel already occupies the target
  placement
- keep `SidebarPanelChrome::None` contributions, such as the terminal toolbar,
  outside the exclusive host panel slot

- [x] **Step 3: Implement terminal tool placement exclusivity**

`TerminalToolDockState::open_tool` closes any other open tool at the same
placement. `TerminalToolDockState::move_tool` closes any other open tool in the
target placement when the moved tool is open.

- [x] **Step 4: Verify**

Run:

```bash
rtk cargo test -p one-core sidebar -- --nocapture
rtk cargo test -p terminal_view tool_dock -- --nocapture
```

Expected: both focused test suites pass.

### Task 14: Service Tools Toolbar Visibility

**Status:** Done.

**Goal:** Restore DB/Redis/Mongo service tool sidebars so users see the service
toolbar by default, while keeping AI/tool panels collapsed until selected.

**Files:**
- Modify: `crates/db_view/src/database_tab.rs`
- Modify: `crates/redis_view/src/redis_tab.rs`
- Modify: `crates/mongodb_view/src/mongo_tab.rs`
- Modify: `docs/superpowers/specs/2026-07-03-sidebar-contributions-design.md`

- [x] **Step 1: Add failing service toolbar tests**

Add pure tests for each service:

```rust
database_tools_sidebar_keeps_toolbar_visible_by_default
database_tools_sidebar_uses_toolbar_width_until_panel_opens
redis_tools_sidebar_keeps_toolbar_visible_by_default
redis_tools_sidebar_uses_toolbar_width_until_panel_opens
mongo_tools_sidebar_keeps_toolbar_visible_by_default
mongo_tools_sidebar_uses_toolbar_width_until_panel_opens
```

Run:

```bash
rtk cargo test -p db_view tools_sidebar -- --nocapture
rtk cargo test -p redis_view tools_sidebar -- --nocapture
rtk cargo test -p mongodb_view tools_sidebar -- --nocapture
```

Expected before implementation: default-visible tests fail because service
tools contributions are hidden entirely, leaving only the generic hidden-panel
launcher.

- [x] **Step 2: Keep tools contributions visible by default**

Set `database.sidebar`, `redis.sidebar`, and `mongodb.sidebar` policies to
`initially_visible: true`. This shows the service-owned toolbars by default.
The AI/tool panel is still collapsed because the service sidebar's internal
`active_panel` remains `None`.

- [x] **Step 3: Use toolbar size while collapsed**

When the internal service sidebar panel is not visible, report
`TOOLBAR_WIDTH` for side and bottom size. When the tool panel opens, report the
saved sidebar panel size.

- [x] **Step 4: Verify**

Run:

```bash
rtk cargo test -p db_view tools_sidebar -- --nocapture
rtk cargo test -p redis_view tools_sidebar -- --nocapture
rtk cargo test -p mongodb_view tools_sidebar -- --nocapture
```

Expected: all six tests pass.

### Task 15: Manual Runtime QA Pass

**Status:** Pending.

**Goal:** Verify the rendered app behavior after Tasks 10-14, because several
requirements are visual/interactive and cannot be fully proven by pure tests.

**Files:** no source changes unless QA finds bugs.

- [ ] **Step 1: Launch app**

Run:

```bash
CLANG_MODULE_CACHE_PATH=/tmp/clang-cache rtk cargo run -p main
```

Expected: app launches without panic.

- [ ] **Step 2: Verify split visibility**

Manual checks:

- Open two work tabs.
- Split one tab right.
- Click Home while primary still has a work tab: Home should render inside the
  primary split group, and secondary groups should remain visible.
- Close all primary regular tabs while a secondary pane has a tab: secondary
  pane should remain visible.
- Close secondary pane tabs: layout should prune back to primary only.

- [ ] **Step 3: Verify service sidebar defaults**

Manual checks:

- Open Database, Redis, and MongoDB tabs.
- Tree sidebar is visible.
- Tree header row is hidden.
- Tools sidebars show their collapsed toolbar by default.
- AI/tool panels are not opened by default.
- Tools/AI sidebars can be reopened and moved left/right/bottom.
- Moving/opening a Tools/AI sidebar into an occupied position closes the
  previous hideable panel there instead of stacking panels.

- [ ] **Step 4: Verify terminal toolbar behavior**

Manual checks:

- Terminal toolbar is always visible while terminal tab is active.
- Closing a tool panel does not hide the toolbar.
- Multiple tools can be open when they are in different positions.
- Opening or moving a tool into a position that already has another tool closes
  the previous tool in that position.

- [ ] **Step 5: Final verification commands**

Run:

```bash
rtk cargo fmt
rtk cargo test -p one-core sidebar -- --nocapture
rtk cargo test -p one-core split -- --nocapture
rtk cargo test -p terminal_view tool_dock -- --nocapture
rtk cargo test -p main
rtk cargo check -p one-core
rtk cargo check -p terminal_view
rtk cargo check -p db_view
rtk cargo check -p redis_view
rtk cargo check -p mongodb_view
rtk cargo check -p sftp_view
rtk cargo check -p remote_desktop_view
rtk cargo check -p main
rtk git diff --check
```

Expected: all commands succeed, aside from the known `block v0.1.6`
future-incompat warning emitted by cargo.
