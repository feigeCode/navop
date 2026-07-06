use crate::{
    split_tab_container::split_tree_visible_for_layout,
    tab_container::{move_to_primary_command_visible, split_command_enabled},
};

#[test]
fn split_command_requires_container_and_tab_capability() {
    assert!(split_command_enabled(true, true, 2));
    assert!(!split_command_enabled(false, true, 2));
    assert!(!split_command_enabled(true, false, 2));
}

#[test]
fn split_command_rejects_single_tab_source_pane() {
    assert!(!split_command_enabled(true, true, 1));
}

#[test]
fn move_to_primary_command_only_shows_for_secondary_split_panes() {
    assert!(!move_to_primary_command_visible(false, false));
    assert!(!move_to_primary_command_visible(true, true));
    assert!(move_to_primary_command_visible(true, false));
}

#[test]
fn split_tree_visibility_follows_layout_not_active_tab_capability() {
    assert!(split_tree_visible_for_layout(false, true, false, false));
    assert!(split_tree_visible_for_layout(true, true, false, false));
    assert!(!split_tree_visible_for_layout(false, false, false, false));
}

#[test]
fn split_tree_stays_visible_when_primary_empty_and_secondary_has_tabs() {
    assert!(split_tree_visible_for_layout(false, true, true, true));
}

#[test]
fn split_tree_stays_visible_for_home_when_primary_still_has_regular_tabs() {
    assert!(split_tree_visible_for_layout(false, true, false, true));
}

#[test]
fn split_visibility_uses_pinned_tab_capability_when_pinned_is_active() {
    assert!(!crate::tab_container::active_content_can_split_for_layout(
        true,
        Some(false),
        Some(true),
    ));
    assert!(crate::tab_container::active_content_can_split_for_layout(
        true,
        Some(true),
        Some(false),
    ));
    assert!(crate::tab_container::active_content_can_split_for_layout(
        false,
        Some(false),
        Some(true),
    ));
}
