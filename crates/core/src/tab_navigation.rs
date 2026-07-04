#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCycleDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTabSlot {
    Pinned(usize),
    Regular(usize),
}

pub fn tab_number_target(
    number: usize,
    pinned_tab_count: usize,
    regular_tab_count: usize,
) -> Option<ActiveTabSlot> {
    if number == 0 {
        return None;
    }
    let zero_based = number - 1;
    if zero_based < pinned_tab_count {
        return Some(ActiveTabSlot::Pinned(zero_based));
    }
    let regular_index = zero_based.checked_sub(pinned_tab_count)?;
    (regular_index < regular_tab_count).then_some(ActiveTabSlot::Regular(regular_index))
}

pub fn tab_slot_after_cycle(
    active: ActiveTabSlot,
    pinned_tab_count: usize,
    regular_tab_count: usize,
    direction: TabCycleDirection,
) -> Option<ActiveTabSlot> {
    let total = pinned_tab_count + regular_tab_count;
    if total == 0 {
        return None;
    }

    let current = slot_to_flat_index(active, pinned_tab_count, regular_tab_count)?;
    let next = match direction {
        TabCycleDirection::Next => (current + 1) % total,
        TabCycleDirection::Previous => {
            if current == 0 {
                total - 1
            } else {
                current - 1
            }
        }
    };
    flat_index_to_slot(next, pinned_tab_count, regular_tab_count)
}

pub fn tab_index_after_cycle(
    active_index: usize,
    tab_count: usize,
    pinned_tab_active: bool,
    direction: TabCycleDirection,
) -> Option<usize> {
    if pinned_tab_active {
        return match direction {
            TabCycleDirection::Next => first_regular_tab_index(tab_count),
            TabCycleDirection::Previous => last_regular_tab_index(tab_count),
        };
    }

    match direction {
        TabCycleDirection::Next => next_regular_tab_index(active_index, tab_count),
        TabCycleDirection::Previous => previous_regular_tab_index(active_index, tab_count),
    }
}

pub fn next_regular_tab_index(active_index: usize, tab_count: usize) -> Option<usize> {
    let active_index = normalized_active_index(active_index, tab_count)?;
    Some((active_index + 1) % tab_count)
}

pub fn previous_regular_tab_index(active_index: usize, tab_count: usize) -> Option<usize> {
    let active_index = normalized_active_index(active_index, tab_count)?;
    if active_index == 0 {
        Some(tab_count - 1)
    } else {
        Some(active_index - 1)
    }
}

fn first_regular_tab_index(tab_count: usize) -> Option<usize> {
    (tab_count > 0).then_some(0)
}

fn last_regular_tab_index(tab_count: usize) -> Option<usize> {
    tab_count.checked_sub(1)
}

fn normalized_active_index(active_index: usize, tab_count: usize) -> Option<usize> {
    if tab_count == 0 {
        None
    } else {
        Some(active_index.min(tab_count - 1))
    }
}

fn slot_to_flat_index(
    slot: ActiveTabSlot,
    pinned_tab_count: usize,
    regular_tab_count: usize,
) -> Option<usize> {
    match slot {
        ActiveTabSlot::Pinned(index) => (index < pinned_tab_count).then_some(index),
        ActiveTabSlot::Regular(index) => {
            (index < regular_tab_count).then_some(pinned_tab_count + index)
        }
    }
}

fn flat_index_to_slot(
    index: usize,
    pinned_tab_count: usize,
    regular_tab_count: usize,
) -> Option<ActiveTabSlot> {
    if index < pinned_tab_count {
        Some(ActiveTabSlot::Pinned(index))
    } else {
        let regular_index = index.checked_sub(pinned_tab_count)?;
        (regular_index < regular_tab_count).then_some(ActiveTabSlot::Regular(regular_index))
    }
}
