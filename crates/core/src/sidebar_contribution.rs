use gpui::{AnyView, App, EntityId, Hsla, Pixels, SharedString, Window};
use gpui_component::IconName;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SidebarPlacement {
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidebarPlacementSet {
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

impl SidebarPlacementSet {
    pub const fn all() -> Self {
        Self {
            left: true,
            right: true,
            bottom: true,
        }
    }

    pub const fn right_only() -> Self {
        Self {
            left: false,
            right: true,
            bottom: false,
        }
    }

    pub const fn left_right() -> Self {
        Self {
            left: true,
            right: true,
            bottom: false,
        }
    }

    pub const fn contains(self, placement: SidebarPlacement) -> bool {
        match placement {
            SidebarPlacement::Left => self.left,
            SidebarPlacement::Right => self.right,
            SidebarPlacement::Bottom => self.bottom,
        }
    }
}

impl Default for SidebarPlacementSet {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidebarPanelPolicy {
    pub hideable: bool,
    pub movable: bool,
    pub allowed_placements: SidebarPlacementSet,
    pub initially_visible: bool,
}

impl Default for SidebarPanelPolicy {
    fn default() -> Self {
        Self {
            hideable: true,
            movable: true,
            allowed_placements: SidebarPlacementSet::all(),
            initially_visible: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SidebarPanelId {
    pub owner: EntityId,
    pub local_id: &'static str,
}

impl SidebarPanelId {
    pub const fn new(owner: EntityId, local_id: &'static str) -> Self {
        Self { owner, local_id }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SidebarPanelStyle {
    pub background: Option<Hsla>,
    pub header_background: Option<Hsla>,
    pub border: Option<Hsla>,
    pub text: Option<Hsla>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SidebarPanelSize {
    pub side_width: Option<Pixels>,
    pub bottom_height: Option<Pixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarPanelChrome {
    Host,
    HostNoHeader,
    None,
}

impl Default for SidebarPanelChrome {
    fn default() -> Self {
        Self::Host
    }
}

pub const fn sidebar_panel_renders_header(chrome: SidebarPanelChrome) -> bool {
    matches!(chrome, SidebarPanelChrome::Host)
}

#[derive(Clone, Default)]
pub struct SidebarContributionActions {
    pub close: Option<Arc<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub move_to: Option<Arc<dyn Fn(SidebarPlacement, &mut Window, &mut App) + 'static>>,
}

#[derive(Clone)]
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
