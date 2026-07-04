use one_core::keybindings::action_id;
use one_core::tab_navigation::{
    ActiveTabSlot, TabCycleDirection, next_regular_tab_index, previous_regular_tab_index,
    tab_index_after_cycle, tab_number_target, tab_slot_after_cycle,
};

#[test]
fn next_regular_tab_index_wraps_forward() {
    assert_eq!(Some(1), next_regular_tab_index(0, 3));
    assert_eq!(Some(0), next_regular_tab_index(2, 3));
    assert_eq!(None, next_regular_tab_index(0, 0));
}

#[test]
fn previous_regular_tab_index_wraps_backward() {
    assert_eq!(Some(2), previous_regular_tab_index(0, 3));
    assert_eq!(Some(0), previous_regular_tab_index(1, 3));
    assert_eq!(None, previous_regular_tab_index(0, 0));
}

#[test]
fn tab_cycle_starts_from_edge_when_pinned_tab_is_active() {
    assert_eq!(
        Some(0),
        tab_index_after_cycle(2, 3, true, TabCycleDirection::Next)
    );
    assert_eq!(
        Some(2),
        tab_index_after_cycle(0, 3, true, TabCycleDirection::Previous)
    );
}

#[test]
fn tab_number_targets_pinned_tabs_before_regular_tabs() {
    assert_eq!(Some(ActiveTabSlot::Pinned(0)), tab_number_target(1, 2, 3));
    assert_eq!(Some(ActiveTabSlot::Pinned(1)), tab_number_target(2, 2, 3));
    assert_eq!(Some(ActiveTabSlot::Regular(0)), tab_number_target(3, 2, 3));
    assert_eq!(Some(ActiveTabSlot::Regular(2)), tab_number_target(5, 2, 3));
    assert_eq!(None, tab_number_target(6, 2, 3));
    assert_eq!(None, tab_number_target(0, 2, 3));
}

#[test]
fn tab_slot_cycle_walks_pinned_tabs_then_regular_tabs() {
    assert_eq!(
        Some(ActiveTabSlot::Pinned(1)),
        tab_slot_after_cycle(ActiveTabSlot::Pinned(0), 2, 2, TabCycleDirection::Next)
    );
    assert_eq!(
        Some(ActiveTabSlot::Regular(0)),
        tab_slot_after_cycle(ActiveTabSlot::Pinned(1), 2, 2, TabCycleDirection::Next)
    );
    assert_eq!(
        Some(ActiveTabSlot::Pinned(0)),
        tab_slot_after_cycle(ActiveTabSlot::Regular(1), 2, 2, TabCycleDirection::Next)
    );
    assert_eq!(
        Some(ActiveTabSlot::Pinned(1)),
        tab_slot_after_cycle(ActiveTabSlot::Regular(0), 2, 2, TabCycleDirection::Previous)
    );
}

#[test]
fn tab_slot_cycle_wraps_when_only_pinned_tabs_exist() {
    assert_eq!(
        Some(ActiveTabSlot::Pinned(1)),
        tab_slot_after_cycle(ActiveTabSlot::Pinned(0), 2, 0, TabCycleDirection::Previous)
    );
    assert_eq!(
        Some(ActiveTabSlot::Pinned(0)),
        tab_slot_after_cycle(ActiveTabSlot::Pinned(1), 2, 0, TabCycleDirection::Next)
    );
}

#[test]
fn tab_cycle_action_ids_are_stable() {
    assert_eq!("app.switch_next_tab", action_id::APP_SWITCH_NEXT_TAB);
    assert_eq!(
        "app.switch_previous_tab",
        action_id::APP_SWITCH_PREVIOUS_TAB,
    );
}
