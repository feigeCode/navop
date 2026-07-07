# Terminal Internal Tool Dock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move terminal tool panels into an internal TerminalView-managed dock so AI/sidebar layout is stable while preserving all existing terminal tool features.

**Current status (2026-07-07):** Implementation is present in the worktree and automated verification has passed for terminal dock state/layout/view tests, AI sidebar layout tests, and `cargo check` for `terminal_view` / `ai_chat_view`. Commit steps and manual runtime UI verification are intentionally still open.

**Architecture:** Terminal tools remain backed by `TerminalToolDockState`, but rendering moves out of external `SidebarContribution` slots and into `TerminalView` itself. The terminal tab owns left, right, and bottom tool regions, renders the existing toolbar internally, and calls `TerminalSidebar::move_tool` / `close_tool` for all panel actions. `TabContainer` no longer receives one contribution per terminal tool panel.

**Tech Stack:** Rust, GPUI, `terminal_view`, `ai_chat_view`, existing `one_core::sidebar_contribution` placement types.

---

## Non-Negotiable Behavior Contracts

- Preserve toolbar behavior: terminal tool toolbar remains visible and can open Settings, AI Chat, File Manager, Server Monitor, History, Quick Commands, and Rich Input as available.
- Preserve multi-position behavior: at most one tool panel per edge, but different edges can be open at the same time.
- Preserve movement behavior: any open panel can move between Left, Right, and Bottom without disappearing.
- Preserve close behavior: closing one panel does not close panels on other edges and does not hide the toolbar.
- Preserve panel availability behavior: File Manager and Server Monitor remain SSH-only; History remains gated by history scope.
- Preserve AI behavior: AI panel still uses terminal context, code actions, paste-to-terminal actions, and sidebar-mode compact UI.
- Fix layout behavior: AI cards, message list, and input area must not shrink, drift left, or resize according to streaming card content.
- Keep outer `TabContainer` uninvolved in terminal tool panel layout. It should not receive separate `terminal.ai-chat`, `terminal.settings`, or `terminal.toolbar` contributions.

## Files

- Modify: `crates/terminal_view/src/sidebar/mod.rs`
  - Keep `TerminalToolDockState`, `TerminalSidebar`, `TerminalSidebarToolbar`, `TerminalSidebarToolPanel`.
  - Add small public/internal helpers needed by `TerminalView` to render panel content by placement.
  - Keep existing panel content construction and event emission.
- Create: `crates/terminal_view/src/sidebar/tool_dock.rs`
  - Define pure layout helpers and GPUI render helpers for left/right/bottom terminal tool docks.
  - Render internal panel frame with title, icon, move buttons, and close button.
- Modify: `crates/terminal_view/src/view.rs`
  - Remove terminal tool panels from external `sidebar_contributions`.
  - Integrate internal tool dock into `TerminalView::render`.
  - Route resize handling to internal left/right/bottom panel regions.
- Modify: `crates/terminal_view/src/lib.rs`
  - Export no new public API unless existing exports require module visibility.
- Modify: `crates/ai_chat_view/src/message_view.rs`
  - Keep or add sidebar-specific edge-to-edge message layout.
- Modify: `crates/ai_chat_view/src/agent_view.rs`
  - Keep or add sidebar-mode fixed-host layout regression tests.
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
  - Keep or add `min_w_0`/edge-to-edge constraints required by sidebar mode.
- Modify: `crates/ai_chat_view/src/agent_cards.rs`
  - Add debug selector for tool card layout tests if not already present.

---

## Task 1: Lock Existing Tool Dock State Semantics

**Files:**
- Modify: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Verify current state tests exist**

Confirm these tests are present in `crates/terminal_view/src/sidebar/mod.rs`:

```rust
tool_dock_can_keep_multiple_tools_open_at_different_edges
tool_dock_opening_tool_closes_existing_tool_at_same_edge
tool_dock_moving_tool_to_occupied_edge_closes_existing_tool
tool_dock_closes_one_panel_without_hiding_toolbar_or_other_panels
```

- [ ] **Step 2: Add a test for moving an open panel without closing it**

Add this test near the other `TerminalToolDockState` tests:

```rust
#[test]
fn tool_dock_moving_open_panel_keeps_it_open() {
    let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

    dock.open_tool(SidebarPanel::AiChat);
    assert!(dock.move_tool(SidebarPanel::AiChat, SidebarPlacement::Bottom));

    assert_eq!(
        dock.open_panels(),
        vec![(SidebarPanel::AiChat, SidebarPlacement::Bottom)],
    );
    assert!(dock.toolbar_visible());
}
```

- [ ] **Step 3: Run state tests**

Run:

```bash
rtk cargo test -p terminal_view sidebar::tests::tool_dock
```

Expected: all `tool_dock_*` tests pass.

- [ ] **Step 4: Commit state contract**

```bash
git add crates/terminal_view/src/sidebar/mod.rs
git commit -m "test: lock terminal tool dock state behavior"
```

---

## Task 2: Add Pure Internal Dock Layout Model

**Files:**
- Create: `crates/terminal_view/src/sidebar/tool_dock.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Create `tool_dock.rs` with layout data**

Create `crates/terminal_view/src/sidebar/tool_dock.rs`:

```rust
use super::SidebarPanel;
use gpui::Pixels;
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalToolDockLayout {
    pub(crate) left: Option<SidebarPanel>,
    pub(crate) right: Option<SidebarPanel>,
    pub(crate) bottom: Option<SidebarPanel>,
}

impl TerminalToolDockLayout {
    pub(crate) fn from_open_panels(
        open_panels: impl IntoIterator<Item = (SidebarPanel, SidebarPlacement)>,
    ) -> Self {
        let mut layout = Self::default();
        for (panel, placement) in open_panels {
            match placement {
                SidebarPlacement::Left => layout.left = Some(panel),
                SidebarPlacement::Right => layout.right = Some(panel),
                SidebarPlacement::Bottom => layout.bottom = Some(panel),
            }
        }
        layout
    }

    pub(crate) fn has_left(&self) -> bool {
        self.left.is_some()
    }

    pub(crate) fn has_right(&self) -> bool {
        self.right.is_some()
    }

    pub(crate) fn has_bottom(&self) -> bool {
        self.bottom.is_some()
    }
}

pub(crate) fn right_tool_region_width(layout: &TerminalToolDockLayout, panel_size: Pixels) -> Pixels {
    if layout.has_right() {
        panel_size + TOOLBAR_WIDTH
    } else {
        TOOLBAR_WIDTH
    }
}
```

- [ ] **Step 2: Wire module into `sidebar/mod.rs`**

At the top-level module section in `crates/terminal_view/src/sidebar/mod.rs`, add:

```rust
pub(crate) mod tool_dock;
```

- [ ] **Step 3: Add pure layout tests**

In `tool_dock.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn layout_maps_open_panels_to_edges() {
        let layout = TerminalToolDockLayout::from_open_panels([
            (SidebarPanel::Settings, SidebarPlacement::Left),
            (SidebarPanel::AiChat, SidebarPlacement::Bottom),
            (SidebarPanel::FileManager, SidebarPlacement::Right),
        ]);

        assert_eq!(Some(SidebarPanel::Settings), layout.left);
        assert_eq!(Some(SidebarPanel::FileManager), layout.right);
        assert_eq!(Some(SidebarPanel::AiChat), layout.bottom);
    }

    #[test]
    fn right_region_keeps_toolbar_width_without_right_panel() {
        let layout = TerminalToolDockLayout::from_open_panels([
            (SidebarPanel::Settings, SidebarPlacement::Left),
        ]);

        assert_eq!(TOOLBAR_WIDTH, right_tool_region_width(&layout, px(420.0)));
    }

    #[test]
    fn right_region_includes_panel_and_toolbar_when_right_panel_is_open() {
        let layout = TerminalToolDockLayout::from_open_panels([
            (SidebarPanel::AiChat, SidebarPlacement::Right),
        ]);

        assert_eq!(px(420.0) + TOOLBAR_WIDTH, right_tool_region_width(&layout, px(420.0)));
    }
}
```

- [ ] **Step 4: Run layout tests**

Run:

```bash
rtk cargo test -p terminal_view sidebar::tool_dock::tests
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit layout model**

```bash
git add crates/terminal_view/src/sidebar/mod.rs crates/terminal_view/src/sidebar/tool_dock.rs
git commit -m "test: add terminal internal tool dock layout model"
```

---

## Task 3: Render Internal Tool Panel Frame

**Files:**
- Modify: `crates/terminal_view/src/sidebar/tool_dock.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Add a frame rendering helper**

In `tool_dock.rs`, add imports and helper:

```rust
use super::{SidebarPanel, TerminalSidebar};
use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

pub(crate) fn render_internal_tool_panel_frame(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    content: AnyElement,
    _window: &mut Window,
    cx: &mut Context<TerminalSidebar>,
) -> AnyElement {
    let colors = sidebar.read(cx).colors();
    let title: SharedString = panel.title().into();
    v_flex()
        .debug_selector(|| format!("terminal-internal-tool-panel-{}", panel.local_id()))
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(colors.background)
        .border_1()
        .border_color(colors.border)
        .child(render_internal_tool_panel_header(sidebar, panel, title, cx))
        .child(div().flex_1().min_h_0().min_w_0().overflow_hidden().child(content))
        .into_any_element()
}
```

- [ ] **Step 2: Add header helper with movement buttons**

In the same file, add:

```rust
fn render_internal_tool_panel_header(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    title: SharedString,
    cx: &mut Context<TerminalSidebar>,
) -> AnyElement {
    let colors = sidebar.read(cx).colors();
    h_flex()
        .h(px(34.0))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_2()
        .bg(colors.muted)
        .border_b_1()
        .border_color(colors.border)
        .child(Icon::new(panel.icon_name()).with_size(gpui_component::Size::Small))
        .child(div().flex_1().min_w_0().truncate().child(title))
        .child(move_button(sidebar.clone(), panel, SidebarPlacement::Left, IconName::PanelLeft))
        .child(move_button(sidebar.clone(), panel, SidebarPlacement::Right, IconName::PanelRight))
        .child(move_button(sidebar.clone(), panel, SidebarPlacement::Bottom, IconName::PanelBottom))
        .child(close_button(sidebar, panel))
        .into_any_element()
}
```

- [ ] **Step 3: Add button helpers**

In the same file, add:

```rust
fn move_button(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
    icon: IconName,
) -> Button {
    Button::new(SharedString::from(format!(
        "terminal-tool-move-{placement:?}-{}",
        panel.local_id()
    )))
    .icon(icon)
    .ghost()
    .compact()
    .on_click(move |_, _window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.move_tool(panel, placement, cx));
    })
}

fn close_button(sidebar: Entity<TerminalSidebar>, panel: SidebarPanel) -> Button {
    Button::new(SharedString::from(format!(
        "terminal-tool-close-{}",
        panel.local_id()
    )))
    .icon(IconName::Close)
    .ghost()
    .compact()
    .on_click(move |_, _window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.close_tool(panel, cx));
    })
}
```

- [ ] **Step 4: Expose immutable colors from `TerminalSidebar`**

In `TerminalSidebar`, add:

```rust
pub(crate) fn colors(&self) -> TerminalColors {
    self.colors.clone()
}
```

- [ ] **Step 5: Run check**

Run:

```bash
rtk cargo check -p terminal_view
```

Expected: 0 errors.

- [ ] **Step 6: Commit frame helper**

```bash
git add crates/terminal_view/src/sidebar/mod.rs crates/terminal_view/src/sidebar/tool_dock.rs
git commit -m "feat: add terminal internal tool panel frame"
```

---

## Task 4: Integrate Internal Dock Into TerminalView Render

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/sidebar/tool_dock.rs`

- [ ] **Step 1: Stop storing external tool panel entities in TerminalView**

Remove these fields from `TerminalView`:

```rust
sidebar_toolbar: Entity<TerminalSidebarToolbar>,
sidebar_tool_panels: HashMap<SidebarPanel, Entity<TerminalSidebarToolPanel>>,
```

Remove the construction block that creates `sidebar_toolbar` and `sidebar_tool_panels` in `TerminalView::new`.

- [ ] **Step 2: Add render helpers in `view.rs`**

Add helper methods on `TerminalView`:

```rust
fn terminal_tool_layout(&self, cx: &App) -> TerminalToolDockLayout {
    TerminalToolDockLayout::from_open_panels(self.sidebar.read(cx).open_tool_panels())
}

fn render_internal_tool_panel(
    &self,
    panel: SidebarPanel,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> AnyElement {
    let content = self.sidebar.read(cx).render_panel_content(panel, window, cx);
    render_internal_tool_panel_frame(
        self.sidebar.clone(),
        panel,
        content,
        window,
        cx,
    )
}
```

If `Context<Self>` cannot be passed to a helper expecting `Context<TerminalSidebar>`, adjust `render_internal_tool_panel_frame` to accept colors and action closures instead of a `Context<TerminalSidebar>`. Keep the action target as `Entity<TerminalSidebar>`.

- [ ] **Step 3: Replace render layout**

In `TerminalView::render`, replace the existing embedded-sidebar block:

```rust
.when(sidebar_visible, |this| {
    this.child(
        div()
            .relative()
            .h_full()
            .w(sidebar_panel_size)
            .flex_shrink_0()
            .child(self.render_sidebar_resize_handle(window, cx))
            .child(self.sidebar.clone()),
    )
})
.when(render_embedded_sidebar && !sidebar_visible, |this| {
    this.child(self.sidebar.clone())
})
```

with an internal layout that renders:

```rust
let tool_layout = self.terminal_tool_layout(cx);
let left_panel = tool_layout.left;
let right_panel = tool_layout.right;
let bottom_panel = tool_layout.bottom;
```

and the outer structure:

```rust
v_flex()
    .size_full()
    .bg(bg_color)
    .child(
        h_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when_some(left_panel, |this, panel| {
                this.child(
                    div()
                        .relative()
                        .h_full()
                        .w(sidebar_panel_size)
                        .flex_shrink_0()
                        .child(self.render_internal_tool_panel(panel, window, cx))
                )
            })
            .child(terminal_region)
            .child(
                h_flex()
                    .h_full()
                    .flex_shrink_0()
                    .w(right_tool_region_width(&tool_layout, sidebar_panel_size))
                    .when_some(right_panel, |this, panel| {
                        this.child(
                            div()
                                .relative()
                                .h_full()
                                .w(sidebar_panel_size)
                                .flex_shrink_0()
                                .child(self.render_internal_tool_panel(panel, window, cx))
                        )
                    })
                    .child(self.sidebar.clone())
            )
    )
    .when_some(bottom_panel, |this, panel| {
        this.child(
            div()
                .relative()
                .w_full()
                .h(sidebar_panel_size)
                .flex_shrink_0()
                .child(self.render_internal_tool_panel(panel, window, cx))
        )
    })
```

Keep the terminal surface code unchanged inside `terminal_region`. Do not rewrite terminal rendering logic.

- [ ] **Step 4: Make `TerminalSidebar::render` toolbar-only-safe**

`TerminalSidebar::render` currently renders active panel plus toolbar. For internal dock mode, it should be used as toolbar only in `TerminalView::render`.

Add a method:

```rust
pub(crate) fn render_toolbar_only(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    self.render_toolbar(window, cx)
}
```

Then use an entity wrapper if direct method rendering is awkward. Do not reintroduce an external `TerminalSidebarToolbar` contribution.

- [ ] **Step 5: Run render compilation check**

Run:

```bash
rtk cargo check -p terminal_view
```

Expected: 0 errors.

- [ ] **Step 6: Commit internal render integration**

```bash
git add crates/terminal_view/src/view.rs crates/terminal_view/src/sidebar/mod.rs crates/terminal_view/src/sidebar/tool_dock.rs
git commit -m "feat: render terminal tools inside terminal view"
```

---

## Task 5: Remove External Terminal Tool Contributions

**Files:**
- Modify: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Change `sidebar_contributions`**

In `impl TabContent for TerminalView`, change:

```rust
fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> {
    if self.sidebar_render_mode != TerminalSidebarRenderMode::External {
        return Vec::new();
    }
    ...
}
```

to:

```rust
fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
    Vec::new()
}
```

This is the architectural boundary: `TabContainer` must not manage terminal tools.

- [ ] **Step 2: Remove unused imports**

Remove unused imports from `view.rs`:

```rust
SidebarContributionActions
SidebarPanelChrome
SidebarPanelId
SidebarPanelPolicy
SidebarPanelSize
SidebarPanelStyle
SidebarPlacementSet
```

Keep `SidebarPlacement` if still used by internal movement or resize logic.

- [ ] **Step 3: Run search to verify no external tool panel references remain**

Run:

```bash
rtk rg -n "TerminalSidebarToolPanel|TerminalSidebarToolbar|sidebar_tool_panels|sidebar_toolbar|terminal\\.toolbar|terminal\\.ai-chat" crates/terminal_view/src
```

Expected: no matches for old external contribution fields or contribution ids.

- [ ] **Step 4: Run check**

Run:

```bash
rtk cargo check -p terminal_view
```

Expected: 0 errors.

- [ ] **Step 5: Commit external contribution removal**

```bash
git add crates/terminal_view/src/view.rs
git commit -m "refactor: stop exposing terminal tools as external sidebars"
```

---

## Task 6: Preserve AI Sidebar Layout Inside Internal Dock

**Files:**
- Modify: `crates/ai_chat_view/src/message_view.rs`
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Modify: `crates/ai_chat_view/src/agent_cards.rs`

- [ ] **Step 1: Add sidebar-specific message renderer**

In `message_view.rs`, add `MessageListLayout` with `Centered` and `EdgeToEdge`, and add:

```rust
pub fn render_sidebar_messages_with_code_actions(
    messages: &[ChatMessageUI],
    scroll_handle: &ScrollHandle,
    code_actions: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    render_messages_with_layout(
        messages,
        scroll_handle,
        code_actions,
        theme,
        MessageListLayout::EdgeToEdge,
        window,
        cx,
    )
}
```

Ensure the message root has:

```rust
.debug_selector(|| "ai-chat-messages".to_string())
.flex_1()
.min_h_0()
.min_w_0()
.w_full()
.relative()
.overflow_hidden()
```

- [ ] **Step 2: Use sidebar renderer in `AgentChatView`**

In `agent_view.rs`, branch on `self.sidebar_mode`:

```rust
let messages = if self.sidebar_mode {
    render_sidebar_messages_with_code_actions(...)
} else {
    render_messages_with_code_actions(...)
};
```

- [ ] **Step 3: Add width constraints to sidebar root/input**

In sidebar mode root, ensure:

```rust
.debug_selector(|| "agent-sidebar-root".to_string())
.size_full()
.min_w_0()
.overflow_hidden()
```

For the stack:

```rust
.debug_selector(|| "agent-sidebar-stack".to_string())
.size_full()
.min_w_0()
.min_h_0()
.overflow_hidden()
```

For input area:

```rust
.debug_selector(|| "agent-input-area".to_string())
.w_full()
.min_w_0()
.flex_shrink_0()
.overflow_hidden()
```

- [ ] **Step 4: Add input root constraints**

In `input/agent_input.rs`, ensure root and toolbar include `.min_w_0()` and inner input wrappers include `.w_full().min_w_0()`.

- [ ] **Step 5: Add tool card debug selector**

In `agent_cards.rs`, add:

```rust
.debug_selector(|| "agent-tool-card".to_string())
```

to the `ToolCard` root `v_flex`.

- [ ] **Step 6: Add AI layout tests**

Add tests in `agent_view.rs`:

```rust
sidebar_mode_fills_fixed_host_frame
sidebar_mode_tool_card_fills_message_column
sidebar_mode_input_is_edge_to_edge
sidebar_mode_user_message_row_fills_message_column
```

Each test should use `debug_bounds` to assert the content fills a fixed 420px sidebar host and does not drift horizontally.

- [ ] **Step 7: Run AI tests**

Run:

```bash
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_fills_fixed_host_frame
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_tool_card_fills_message_column
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_input_is_edge_to_edge
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_user_message_row_fills_message_column
rtk cargo check -p ai_chat_view
```

Expected: all tests pass, 0 check errors.

- [ ] **Step 8: Commit AI layout safeguards**

```bash
git add crates/ai_chat_view/src/message_view.rs crates/ai_chat_view/src/agent_view.rs crates/ai_chat_view/src/input/agent_input.rs crates/ai_chat_view/src/agent_cards.rs
git commit -m "fix: stabilize agent chat layout in terminal dock"
```

---

## Task 7: Add Internal Dock GPUI Regression Tests

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/sidebar/tool_dock.rs`

- [ ] **Step 1: Add debug selectors to internal dock regions**

Add selectors:

```rust
"terminal-tool-dock-root"
"terminal-tool-dock-left"
"terminal-tool-dock-right"
"terminal-tool-dock-bottom"
"terminal-tool-dock-toolbar"
"terminal-tool-panel-content"
```

- [ ] **Step 2: Add a structural test for source contract**

If constructing full `TerminalView` GPUI state is too expensive, add source-level tests in `view.rs` verifying:

```rust
assert!(source.contains("fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution>"));
assert!(source.contains("Vec::new()"));
assert!(source.contains("TerminalToolDockLayout::from_open_panels"));
assert!(source.contains("right_tool_region_width"));
```

Use source tests only for contribution-boundary checks. Use pure state/layout tests for behavior.

- [ ] **Step 3: Add pure behavior tests for internal layout**

In `tool_dock.rs`, add:

```rust
#[test]
fn layout_preserves_all_three_edges() {
    let layout = TerminalToolDockLayout::from_open_panels([
        (SidebarPanel::Settings, SidebarPlacement::Left),
        (SidebarPanel::AiChat, SidebarPlacement::Right),
        (SidebarPanel::HistoryCommand, SidebarPlacement::Bottom),
    ]);

    assert_eq!(Some(SidebarPanel::Settings), layout.left);
    assert_eq!(Some(SidebarPanel::AiChat), layout.right);
    assert_eq!(Some(SidebarPanel::HistoryCommand), layout.bottom);
}
```

- [ ] **Step 4: Run terminal tests**

Run:

```bash
rtk cargo test -p terminal_view sidebar::tests
rtk cargo test -p terminal_view sidebar::tool_dock::tests
rtk cargo test -p terminal_view view::tests
rtk cargo check -p terminal_view
```

Expected: all targeted tests pass, 0 check errors.

- [ ] **Step 5: Commit regression tests**

```bash
git add crates/terminal_view/src/view.rs crates/terminal_view/src/sidebar/tool_dock.rs
git commit -m "test: cover terminal internal tool dock layout"
```

---

## Task 8: Manual Verification Checklist

**Files:**
- No code changes unless verification finds defects.

- [ ] **Step 1: Build or run app**

Use the project’s normal local app command. If no local app command is documented, run:

```bash
rtk cargo check -p terminal_view
rtk cargo check -p ai_chat_view
```

- [ ] **Step 2: Verify terminal toolbar**

Open a terminal tab and confirm the tool toolbar is visible.

- [ ] **Step 3: Verify AI right dock**

Open AI Chat in the terminal toolbar. Confirm:

- AI panel appears in the terminal internal right dock.
- AI cards do not shrink or grow while streaming.
- User message row stays aligned.
- Input area does not drift left.

- [ ] **Step 4: Verify multi-position behavior**

Move panels so that:

- AI Chat is on Right.
- Settings or File Manager is on Left.
- History or Quick Commands is on Bottom.

Confirm all open panels remain visible simultaneously.

- [ ] **Step 5: Verify same-edge exclusivity**

Open two tools on the same edge. Confirm the second replaces the first only on that edge and does not close tools on other edges.

- [ ] **Step 6: Verify close behavior**

Close one panel. Confirm:

- Other edge panels remain open.
- Toolbar remains visible.
- Reopening the closed panel works.

- [ ] **Step 7: Verify no external contribution regression**

Search:

```bash
rtk rg -n "terminal\\.toolbar|terminal\\.ai-chat|TerminalSidebarToolPanel|TerminalSidebarToolbar" crates/terminal_view/src
```

Expected: no external contribution usage remains. `TerminalSidebarToolPanel` and `TerminalSidebarToolbar` may remain only if they are used internally; if they are internal-only, names are acceptable but must not be referenced from `TerminalView::sidebar_contributions`.

- [ ] **Step 8: Final verification commands**

Run:

```bash
rtk cargo test -p terminal_view sidebar::tests
rtk cargo test -p terminal_view sidebar::tool_dock::tests
rtk cargo test -p terminal_view view::tests
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_fills_fixed_host_frame
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_tool_card_fills_message_column
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_input_is_edge_to_edge
rtk cargo test -p ai_chat_view agent_view::tests::sidebar_mode_user_message_row_fills_message_column
rtk cargo check -p terminal_view
rtk cargo check -p ai_chat_view
```

Expected: all tests pass, both checks report 0 errors. Existing `block v0.1.6` future-incompat warning is acceptable if unchanged.

- [ ] **Step 9: Commit final fixes**

If final verification required fixes:

```bash
git add crates/terminal_view crates/ai_chat_view
git commit -m "fix: preserve terminal tool dock behavior"
```

---

## Completion Criteria

- Terminal tool panels are no longer separate `TabContainer` sidebar contributions.
- Toolbar remains visible and functional.
- Existing multi-position behavior is preserved.
- Moving a panel does not make it disappear.
- Multiple panels on different edges can be visible at once.
- Same-edge mutual exclusion is preserved.
- AI Chat layout is stable inside the internal dock.
- All targeted terminal and AI checks pass.
