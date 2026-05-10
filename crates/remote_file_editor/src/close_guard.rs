#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseIntercept {
    Allow,
    Prompt,
    Ignore,
}

pub fn decide_close_intercept(is_dirty: bool, prompt_open: bool) -> CloseIntercept {
    if !is_dirty {
        CloseIntercept::Allow
    } else if prompt_open {
        CloseIntercept::Ignore
    } else {
        CloseIntercept::Prompt
    }
}

pub fn find_tab_index(paths: &[String], remote_path: &str) -> Option<usize> {
    paths.iter().position(|path| path == remote_path)
}

pub fn active_index_after_open(paths: &[String], remote_path: &str) -> usize {
    find_tab_index(paths, remote_path).unwrap_or(paths.len())
}

pub fn active_index_after_close(
    active_index: usize,
    closed_index: usize,
    tab_count: usize,
) -> Option<usize> {
    if tab_count <= 1 || closed_index >= tab_count {
        return None;
    }

    if closed_index < active_index {
        Some(active_index - 1)
    } else if closed_index == active_index && active_index >= tab_count - 1 {
        Some(active_index - 1)
    } else {
        Some(active_index)
    }
}

pub fn has_dirty_tabs(dirty_tabs: &[bool]) -> bool {
    dirty_tabs.iter().any(|dirty| *dirty)
}

#[cfg(test)]
mod tests {
    use super::{
        CloseIntercept, active_index_after_close, active_index_after_open, decide_close_intercept,
        find_tab_index, has_dirty_tabs,
    };

    #[test]
    fn allows_close_when_editor_is_clean() {
        assert_eq!(decide_close_intercept(false, false), CloseIntercept::Allow);
    }

    #[test]
    fn prompts_once_for_unsaved_changes() {
        assert_eq!(decide_close_intercept(true, false), CloseIntercept::Prompt);
    }

    #[test]
    fn ignores_repeated_close_while_prompt_is_open() {
        assert_eq!(decide_close_intercept(true, true), CloseIntercept::Ignore);
    }

    #[test]
    fn finds_existing_tab_index_by_remote_path() {
        let paths = vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()];

        assert_eq!(find_tab_index(&paths, "/tmp/b.txt"), Some(1));
    }

    #[test]
    fn returns_next_index_for_new_remote_path() {
        let paths = vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()];

        assert_eq!(active_index_after_open(&paths, "/tmp/c.txt"), 2);
    }

    #[test]
    fn reuses_existing_index_for_existing_remote_path() {
        let paths = vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()];

        assert_eq!(active_index_after_open(&paths, "/tmp/a.txt"), 0);
    }

    #[test]
    fn keeps_active_index_when_closing_tab_after_active_tab() {
        assert_eq!(active_index_after_close(0, 2, 3), Some(0));
    }

    #[test]
    fn shifts_active_index_left_when_closing_tab_before_active_tab() {
        assert_eq!(active_index_after_close(2, 0, 3), Some(1));
    }

    #[test]
    fn activates_left_tab_when_closing_last_active_tab() {
        assert_eq!(active_index_after_close(2, 2, 3), Some(1));
    }

    #[test]
    fn keeps_same_index_when_closing_middle_active_tab_with_right_neighbor() {
        assert_eq!(active_index_after_close(1, 1, 3), Some(1));
    }

    #[test]
    fn returns_none_when_closing_last_remaining_tab() {
        assert_eq!(active_index_after_close(0, 0, 1), None);
    }

    #[test]
    fn detects_any_dirty_tab() {
        assert!(has_dirty_tabs(&[false, true, false]));
        assert!(!has_dirty_tabs(&[false, false]));
    }
}
