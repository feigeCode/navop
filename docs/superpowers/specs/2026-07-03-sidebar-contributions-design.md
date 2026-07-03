# Sidebar Contributions Design

## Goal

Move service sidebars out of individual service layouts into the owning `TabContainer` content area, while preserving every existing service capability. Sidebar support is capability-based: each active tab may contribute zero or more sidebar panels with explicit policies for visibility, movement, and allowed placements. Split support remains a separate capability and is not implied by having sidebar panels.

## Non-Goals

- Do not migrate to the full `gpui_component::dock` framework in this phase.
- Do not share service sessions across tabs or panes.
- Do not make Home, Settings, Extensions, or nested service subtabs split-capable by default.
- Do not persist sidebar layout in the first implementation pass.
- Do not remove existing embedded sidebar rendering paths until the external mode is verified.

## Requirements

- Sidebar panels are registered by the active tab content through a typed contribution API.
- Each `TabContainer` renders contributions from its own active tab inside the tab content area, below that pane's tab bar.
- Contributions may opt into `Left`, `Right`, and/or `Bottom` placement.
- Contributions may independently allow or disallow hiding.
- Contributions may independently allow or disallow moving.
- Contributions may choose an initial visibility state.
- Contributions may provide host chrome colors for panel background, header background, border, and text. Unset color fields fall back to the active app theme.
- Contributions may keep the host frame while hiding the host header. Tree panels use this so they can stay docked without adding a duplicate title row.
- Contributions may provide preferred side width and bottom height. Side width follows the panel when it moves left or right, so moving a panel between sides does not unexpectedly resize it.
- Each placement shows at most one visible host sidebar panel. Opening or moving
  a panel into an occupied placement closes the existing hideable panel there
  instead of stacking panels. Chrome-less toolbar contributions do not consume
  the host panel slot.
- Existing Database, Redis, MongoDB, and Terminal sidebars retain their current behavior and event wiring.
- Split capability is independent from sidebar capability. Top-level work tabs such as Terminal, SSH/SFTP, Database, Redis, MongoDB, RDP, and VNC opt into split. Home stays unsplittable.
- Opening a new connection from Home always targets the primary tab container, even if a split pane is currently focused. This matches the app's main-tab mental model and avoids accidentally routing new connections into a secondary group.

## Architecture

The app shell becomes:

```text
OnetCliApp
  SplitTabContainer
    pane A TabContainer
      tab bar
      content area
        left sidebar dock     optional
        active tab content
        right sidebar dock    optional
        bottom sidebar dock   optional
    pane B TabContainer
      tab bar
      content area
        pane-local sidebar docks
```

`TabContainer` owns only sidebar layout state for the currently active tab in that pane. Service views continue to own the entities they create today, such as `DbTreeView`, `DatabaseSidebar`, `RedisSidebar`, `MongoSidebar`, and `TerminalSidebar`. The tab container renders these entities through `AnyView` returned by the active tab content.

This keeps the implementation narrow: ownership, subscriptions, and business state stay in the service view; only rendering placement moves outward.

## Core Contracts

`TabContent` gains two default methods:

```rust
fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> {
    Vec::new()
}

fn can_split(&self, cx: &App) -> bool {
    false
}
```

`TabContentView` forwards these methods for dynamic tab contents.

`SidebarContribution` describes one panel:

```rust
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

`SidebarPanelId` combines the owner tab content entity and a local stable key:

```rust
pub struct SidebarPanelId {
    pub owner: EntityId,
    pub local_id: &'static str,
}
```

This avoids collisions between two tabs that both register `database.ai` or `terminal.settings`.

`SidebarPanelPolicy` contains all per-panel switches:

```rust
pub struct SidebarPanelPolicy {
    pub hideable: bool,
    pub movable: bool,
    pub allowed_placements: SidebarPlacementSet,
    pub initially_visible: bool,
}
```

The policy is authoritative. The tab container must not hide a non-hideable panel or move a non-movable panel. If a stored in-memory override conflicts with the policy, it falls back to the contribution default.

`SidebarPanelStyle` contains optional host chrome colors:

```rust
pub struct SidebarPanelStyle {
    pub background: Option<Hsla>,
    pub header_background: Option<Hsla>,
    pub border: Option<Hsla>,
    pub text: Option<Hsla>,
}
```

The tab container applies these colors only to its outer panel frame, header, controls, and hidden-panel launcher. Service content keeps its own internal rendering and theme handling. This lets Terminal use its terminal theme for the sidebar shell while Database, Redis, and MongoDB can continue using the app theme.

`SidebarPanelSize` contains optional host sizing hints:

```rust
pub struct SidebarPanelSize {
    pub side_width: Option<Pixels>,
    pub bottom_height: Option<Pixels>,
}
```

Left and right docks size themselves from the sum of visible panels' `side_width` values. This keeps a panel's width stable when moved between left and right. Bottom docks use the maximum visible `bottom_height`, falling back to a default when a panel does not provide one.

`SidebarPanelChrome` lets a contribution choose whether the tab container wraps
it in host chrome:

```rust
pub enum SidebarPanelChrome {
    Host,
    HostNoHeader,
    None,
}
```

`Host` renders the tab-container header, move controls, close control, frame,
and configured colors. `HostNoHeader` keeps the host frame, border, sizing, and
resize behavior, but suppresses the host header and its controls. Database,
Redis, and MongoDB tree panels use this so the tree appears as navigation chrome
instead of a second titled panel. `None` renders the contribution view directly.
Terminal uses this for the persistent tool toolbar so it is always visible but
does not look like a closable panel.

`SidebarContributionActions` lets a contribution own close and move semantics:

```rust
pub struct SidebarContributionActions {
    pub close: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub move_to: Option<Arc<dyn Fn(SidebarPlacement, &mut Window, &mut App) + 'static>>,
}
```

If an action is provided, the host button calls it and does not write a local
override. This is required for terminal tool panels: closing a tool closes only
that tool panel, and moving a tool updates the terminal tool dock state. If an
action is absent, the tab container keeps using its in-memory override behavior,
which is how Database, Redis, and MongoDB panels continue to work.

## Sidebar State

The first pass uses in-memory state only:

```rust
pub struct SidebarPanelOverride {
    pub visible: bool,
    pub placement: SidebarPlacement,
}

pub struct TabContainer {
    pub overrides: HashMap<SidebarPanelId, SidebarPanelOverride>,
}
```

`SidebarPanelOverride` records the user's current placement and visibility choice. State is currently in-memory and scoped to the owning `TabContainer`. Contributions with explicit `actions` may choose to keep their own equivalent state instead of using this override map.

Reconciliation rules:

- New contribution without override uses `default_placement` and `initially_visible`.
- Non-hideable contributions are always visible, even if `initially_visible` is false.
- Existing override is used only if it still satisfies the contribution policy.
- Contributions absent from the active tab are not rendered, but their overrides remain in memory.
- Each placement has one exclusive host panel slot. When a hideable host panel
  is opened or moved into a placement, other hideable host panels in that
  placement are hidden. A non-hideable host panel blocks hideable panels from
  moving into its placement. Chrome-less contributions, such as the terminal
  toolbar, may render beside the active host panel without counting as the host
  panel for that placement.

## Rendering

`TabContainer` renders sidebar docks inside `render_tab_content` using native
fixed flex containers and a local mouse-tracking resize handler. It does not use
`gpui_component::resizable::{h_resizable, v_resizable, resizable_panel}` for
sidebar docks, because those primitives own panel sizing and can conflict with
the tab content area's flex contract.

```text
tab-content
  tab-sidebar-root (native h_flex)
  left dock panel if visible
  tab-sidebar-center
    active tab content
    bottom dock panel if visible
  right dock panel if visible
```

Each visible `Host` panel wraps service content with a host-level header:

```text
[icon] Title                         [Left] [Right] [Bottom] [Hide]
```

Buttons are derived from policy:

- Hide is shown only when `hideable` is true.
- Placement controls are shown only when `movable` is true.
- Individual placement controls are enabled only when allowed by `allowed_placements`.

Service views do not implement these controls.

`HostNoHeader` panels use the same dock slot, frame, size, border, and resize
behavior, but render only the contribution view inside the frame. `None` panels
render the contribution view directly without host chrome.

Multiple host panels in the same placement are not stacked. The target
placement keeps one host panel visible and closes other hideable host panels in
that placement. Different placements may each have one visible host panel at the
same time.

Because this layout lives inside `TabContainer::render_tab_content`, side docks naturally begin below the active pane's tab bar. No artificial top inset is used.

## Service Migration

Service views add an external sidebar mode:

```rust
pub enum SidebarRenderMode {
    Embedded,
    External,
}
```

Default is `Embedded`, preserving current behavior for direct uses and tests. Main app creation switches supported services to `External`.

Migration shape:

- Database external render: center inner `TabContainer`; contributions:
  `database.tree` visible by default with `HostNoHeader`, and
  `database.sidebar` visible by default as a collapsed tools toolbar with
  normal host chrome. Its AI panel is not opened until the user selects the tool.
- Redis external render: center inner `TabContainer`; contributions:
  `redis.tree` visible by default with `HostNoHeader`, and `redis.sidebar`
  visible by default as a collapsed tools toolbar with normal host chrome. Its
  AI panel is not opened until the user selects the tool.
- MongoDB external render: center inner `TabContainer`; contributions:
  `mongodb.tree` visible by default with `HostNoHeader`, and `mongodb.sidebar`
  visible by default as a collapsed tools toolbar with normal host chrome. Its
  AI panel is not opened until the user selects the tool.
- Terminal external render: terminal surface only; contributions: one persistent
  `terminal.toolbar` plus one contribution for each open terminal tool panel.
  The toolbar is non-hideable, non-movable, and chrome-less. Tool panels are
  independently closable and movable through contribution actions.

The embedded render path remains unchanged during this phase.

Each split pane keeps its own tab bar, matching VSCode editor groups. The tab
bar is required for switching, closing, dragging, and further splitting tabs in
that group. It may later become visually compact when a pane contains a single
tab, but it should not be removed from the model.

## Terminal Tool Dock

Terminal is the only service that needs a persistent tool toolbar in this
phase. The terminal sidebar therefore owns a `TerminalToolDockState` rather
than a single `active_panel`:

```rust
pub struct ToolPanelState {
    pub open: bool,
    pub placement: SidebarPlacement,
}
```

The toolbar is always contributed while the terminal tab is active. Tool buttons
toggle their matching panel. Closing a tool panel sets only that tool's `open`
state to `false`; it does not hide the toolbar and does not close other open
tool panels in other placements. Opening a tool closes any other open tool in
the same placement. Moving a tool updates that tool's placement, keeps the tool
open, and closes any other open tool already occupying the target placement.

Supported terminal tools keep their previous capabilities:

- Settings
- Quick Commands
- AI Chat
- File Manager when the terminal has an SSH/SFTP connection
- Server Monitor when the terminal has an SSH session

Multiple terminal tools may be open at the same time only when they occupy
different placements. The toolbar continues to list every available tool even if
a tool panel has been moved left or bottom.

## Split Policy

Split is controlled by `TabContent::can_split`.

- `TerminalView`, `SftpView`, `DatabaseTabView`, `RedisTabView`, `MongoTabView`, and `RemoteDesktopView` return `true`.
- Home and non-workflow utility tabs use the default `false`.
- `TabContainer` shows split commands only for tabs whose content returns `true`.
- Drag split follows the same capability check.
- `SplitTabContainer` renders the split tree whenever the split tree exists.
  Active tab split capability never controls whether secondary panes are
  visible.
- Home is an ordinary tab inside the primary pane for layout purposes. Home
  itself cannot be split, but activating Home does not collapse or cover
  secondary groups.
- When the primary pane's regular tabs are all closed and secondary panes still
  contain tabs, the split tree stays visible so secondary groups remain
  reachable. It collapses only after all secondary panes are empty and pruned.

This preserves Home as an unsplittable primary tab while allowing split-capable
work tabs to behave like VSCode editor groups.

## Validation

Minimum validation for this phase:

- Unit tests for sidebar placement policy and override reconciliation.
- Unit tests for split capability defaults and terminal opt-in.
- Unit tests for terminal tool dock state: persistent toolbar, multiple open
  tools, close-one-tool behavior, and per-tool placement movement.
- Unit tests for split-tree visibility so non-splittable primary tabs such as
  Home do not hide existing secondary split groups.
- Unit tests for headerless host chrome and collapsed-by-default service tool
  sidebars.
- Compile `one-core`, `terminal_view`, `db_view`, `redis_view`, `mongodb_view`, and `main`.
- Run focused package tests for `one-core` and `main`.
