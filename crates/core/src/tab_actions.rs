use gpui::SharedString;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
