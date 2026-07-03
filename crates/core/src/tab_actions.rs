use gpui::SharedString;
use std::collections::HashSet;

pub const TAB_TITLE_METADATA_KEY: &str = "tab_title";

pub fn resolve_tab_title(
    metadata_title: Option<&str>,
    content_title: SharedString,
) -> SharedString {
    if let Some(title) = metadata_title.and_then(normalize_title) {
        SharedString::from(title)
    } else {
        content_title
    }
}

pub fn normalize_title(title: &str) -> Option<String> {
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabActionAvailability {
    pub rename: bool,
    pub duplicate: bool,
}

pub fn tab_action_availability(can_rename: bool, can_duplicate: bool) -> TabActionAvailability {
    TabActionAvailability {
        rename: can_rename,
        duplicate: can_duplicate,
    }
}

pub fn duplicate_tab_id(source_id: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    const FIRST_DUPLICATE_INDEX: usize = 1;

    let mut index = FIRST_DUPLICATE_INDEX;
    loop {
        let candidate = format!("{source_id}-duplicate-{index}");
        if !exists(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

pub fn mark_tab_activity(
    activity_tabs: &mut HashSet<String>,
    tab_id: &str,
    tab_is_active: bool,
) -> bool {
    if tab_is_active {
        return false;
    }
    activity_tabs.insert(tab_id.to_string())
}

pub fn clear_tab_activity(activity_tabs: &mut HashSet<String>, tab_id: &str) -> bool {
    activity_tabs.remove(tab_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn title_override_uses_trimmed_custom_title() {
        let title = resolve_tab_title(Some("  Ops Shell  "), SharedString::from("Terminal"));

        assert_eq!("Ops Shell", title.as_ref());
    }

    #[test]
    fn blank_title_override_falls_back_to_content_title() {
        let title = resolve_tab_title(Some("   "), SharedString::from("Terminal"));

        assert_eq!("Terminal", title.as_ref());
    }

    #[test]
    fn action_availability_keeps_rename_and_duplicate_independent() {
        let availability = tab_action_availability(true, false);

        assert_eq!(
            TabActionAvailability {
                rename: true,
                duplicate: false,
            },
            availability,
        );
    }

    #[test]
    fn duplicate_tab_id_uses_next_available_suffix() {
        let existing = [
            "terminal-1",
            "terminal-1-duplicate-1",
            "terminal-1-duplicate-2",
        ];

        let id = duplicate_tab_id("terminal-1", |candidate| existing.contains(&candidate));

        assert_eq!("terminal-1-duplicate-3", id);
    }

    #[test]
    fn activity_marker_ignores_active_tab_and_marks_inactive_tab() {
        let mut activity_tabs = HashSet::new();

        assert!(!mark_tab_activity(&mut activity_tabs, "terminal-1", true));
        assert!(activity_tabs.is_empty());

        assert!(mark_tab_activity(&mut activity_tabs, "terminal-2", false));
        assert!(activity_tabs.contains("terminal-2"));
    }

    #[test]
    fn activity_marker_is_cleared_when_tab_is_activated() {
        let mut activity_tabs = HashSet::from(["terminal-2".to_string()]);

        assert!(clear_tab_activity(&mut activity_tabs, "terminal-2"));
        assert!(!activity_tabs.contains("terminal-2"));
        assert!(!clear_tab_activity(&mut activity_tabs, "terminal-2"));
    }
}
