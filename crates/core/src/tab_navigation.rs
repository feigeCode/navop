#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCycleDirection {
    Next,
    Previous,
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
