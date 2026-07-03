use crate::layout::TOOLBAR_WIDTH;
use crate::sidebar_contribution::{
    SidebarPanelChrome, SidebarPanelPolicy, SidebarPanelSize, SidebarPanelStyle, SidebarPlacement,
    SidebarPlacementSet, sidebar_panel_renders_header,
};
use crate::tab_container::{
    sidebar_panel_allows_resize, sidebar_panel_allows_size_override,
    sidebar_panel_blocks_exclusive_target, sidebar_panel_initial_visibility,
    sidebar_panel_should_hide_for_exclusive_target, sidebar_panel_uses_exclusive_slot,
};

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

#[test]
fn default_style_uses_host_theme_colors() {
    let style = SidebarPanelStyle::default();

    assert!(style.background.is_none());
    assert!(style.header_background.is_none());
    assert!(style.border.is_none());
    assert!(style.text.is_none());
}

#[test]
fn default_size_uses_host_fallback_sizes() {
    let size = SidebarPanelSize::default();

    assert!(size.side_width.is_none());
    assert!(size.bottom_height.is_none());
}

#[test]
fn hideable_panel_can_start_hidden() {
    let policy = SidebarPanelPolicy {
        initially_visible: false,
        ..SidebarPanelPolicy::default()
    };

    assert!(policy.hideable);
    assert!(!policy.initially_visible);
    assert!(!sidebar_panel_initial_visibility(policy));
}

#[test]
fn non_hideable_tree_panel_remains_visible() {
    let policy = SidebarPanelPolicy {
        hideable: false,
        initially_visible: false,
        ..SidebarPanelPolicy::default()
    };

    assert!(sidebar_panel_initial_visibility(policy));
}

#[test]
fn host_no_header_chrome_suppresses_header_only() {
    assert!(sidebar_panel_renders_header(SidebarPanelChrome::Host));
    assert!(!sidebar_panel_renders_header(
        SidebarPanelChrome::HostNoHeader
    ));
    assert!(!sidebar_panel_renders_header(SidebarPanelChrome::None));
}

#[test]
fn hideable_host_panel_closes_when_another_panel_targets_same_position() {
    assert!(sidebar_panel_should_hide_for_exclusive_target(
        true,
        SidebarPlacement::Right,
        true,
        SidebarPanelChrome::Host,
        SidebarPlacement::Right,
    ));
    assert!(!sidebar_panel_should_hide_for_exclusive_target(
        true,
        SidebarPlacement::Left,
        true,
        SidebarPanelChrome::Host,
        SidebarPlacement::Right,
    ));
}

#[test]
fn chrome_less_toolbar_does_not_use_exclusive_sidebar_slot() {
    assert!(sidebar_panel_uses_exclusive_slot(SidebarPanelChrome::Host));
    assert!(sidebar_panel_uses_exclusive_slot(
        SidebarPanelChrome::HostNoHeader
    ));
    assert!(!sidebar_panel_uses_exclusive_slot(SidebarPanelChrome::None));
}

#[test]
fn non_hideable_host_panel_blocks_target_position() {
    assert!(sidebar_panel_blocks_exclusive_target(
        true,
        SidebarPlacement::Left,
        false,
        SidebarPanelChrome::HostNoHeader,
        SidebarPlacement::Left,
    ));
    assert!(!sidebar_panel_blocks_exclusive_target(
        true,
        SidebarPlacement::Left,
        false,
        SidebarPanelChrome::None,
        SidebarPlacement::Left,
    ));
}

#[test]
fn collapsed_toolbar_sized_panel_does_not_allow_resize() {
    assert!(!sidebar_panel_allows_resize(
        SidebarPanelChrome::HostNoHeader,
        Some(TOOLBAR_WIDTH),
        Some(TOOLBAR_WIDTH),
    ));
}

#[test]
fn host_panel_larger_than_toolbar_allows_resize() {
    let size = TOOLBAR_WIDTH + gpui::px(120.0);

    assert!(sidebar_panel_allows_resize(
        SidebarPanelChrome::HostNoHeader,
        Some(size),
        None,
    ));
}

#[test]
fn chrome_less_toolbar_does_not_allow_resize() {
    assert!(!sidebar_panel_allows_resize(
        SidebarPanelChrome::None,
        Some(TOOLBAR_WIDTH),
        Some(TOOLBAR_WIDTH),
    ));
}

#[test]
fn toolbar_sized_panel_ignores_resize_override() {
    assert!(!sidebar_panel_allows_size_override(Some(TOOLBAR_WIDTH)));
    assert!(sidebar_panel_allows_size_override(Some(
        TOOLBAR_WIDTH + gpui::px(1.0)
    )));
    assert!(sidebar_panel_allows_size_override(None));
}
