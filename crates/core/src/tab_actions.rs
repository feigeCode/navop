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

/// 复制标签页时使用的新标题：去掉源标题已有的 "(n)" 后缀后，追加最小的可用序号。
/// 例如 "172.29.13.200" -> "172.29.13.200(1)"，"172.29.13.200(1)" -> "172.29.13.200(2)"。
pub fn next_duplicate_tab_title(
    source_title: &str,
    mut exists: impl FnMut(&str) -> bool,
) -> String {
    let base_title = strip_duplicate_suffix(source_title);
    let mut index = 1;
    loop {
        let candidate = format!("{base_title}({index})");
        if !exists(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 去掉标题末尾的 "(正整数)" 后缀，如 "172.29.13.200(1)" -> "172.29.13.200"。
fn strip_duplicate_suffix(title: &str) -> &str {
    let Some(open) = title.rfind('(') else {
        return title;
    };
    let close = title.len().saturating_sub(1);
    if title.as_bytes().get(close) != Some(&b')') {
        return title;
    }
    let number = &title[open + 1..close];
    if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
        let base = &title[..open];
        if base.is_empty() { title } else { base }
    } else {
        title
    }
}

/// 解析 "base(n)" 形式标题中的序号 n（n >= 1）。
pub fn parse_duplicate_index(title: &str, base_title: &str) -> Option<usize> {
    let suffix = title.strip_prefix(base_title)?;
    let number = suffix.strip_prefix('(')?.strip_suffix(')')?;
    let index: usize = number.parse().ok()?;
    (index >= 1).then_some(index)
}

/// 计算同基础名称的下一个可用标签序号（从 1 开始）。
/// 不存在同名标签时返回 None（首个标签不加序号）。
pub fn next_duplicate_tab_index<'a>(
    base_title: &str,
    titles: impl IntoIterator<Item = &'a str>,
) -> Option<usize> {
    let mut used: HashSet<usize> = HashSet::new();
    let mut found_base = false;
    for title in titles {
        if title == base_title {
            found_base = true;
        } else if let Some(index) = parse_duplicate_index(title, base_title) {
            used.insert(index);
        }
    }
    if !found_base && used.is_empty() {
        return None;
    }
    let mut index = 1;
    while used.contains(&index) {
        index += 1;
    }
    Some(index)
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
    fn duplicate_tab_title_numbers_the_first_duplicate() {
        let existing = ["172.29.13.200"];

        let title =
            next_duplicate_tab_title("172.29.13.200", |candidate| existing.contains(&candidate));

        assert_eq!("172.29.13.200(1)", title);
    }

    #[test]
    fn duplicate_tab_title_increments_sequentially() {
        let existing = ["172.29.13.200", "172.29.13.200(1)"];

        let title =
            next_duplicate_tab_title("172.29.13.200", |candidate| existing.contains(&candidate));

        assert_eq!("172.29.13.200(2)", title);
    }

    #[test]
    fn duplicate_tab_title_uses_next_available_gap() {
        let existing = ["172.29.13.200", "172.29.13.200(1)", "172.29.13.200(3)"];

        let title =
            next_duplicate_tab_title("172.29.13.200", |candidate| existing.contains(&candidate));

        assert_eq!("172.29.13.200(2)", title);
    }

    #[test]
    fn duplicating_numbered_tab_keeps_the_original_base() {
        let existing = ["172.29.13.200", "172.29.13.200(1)", "172.29.13.200(2)"];

        let title = next_duplicate_tab_title("172.29.13.200(2)", |candidate| {
            existing.contains(&candidate)
        });

        assert_eq!("172.29.13.200(3)", title);
    }

    #[test]
    fn strip_duplicate_suffix_ignores_non_numeric_tail() {
        assert_eq!("172.29.13.200", strip_duplicate_suffix("172.29.13.200(1)"));
        assert_eq!("host(prod)", strip_duplicate_suffix("host(prod)"));
        assert_eq!("host(1)x", strip_duplicate_suffix("host(1)x"));
        assert_eq!("host", strip_duplicate_suffix("host"));
    }

    #[test]
    fn parse_duplicate_index_matches_exact_base_with_suffix() {
        assert_eq!(
            Some(1),
            parse_duplicate_index("172.29.13.200(1)", "172.29.13.200")
        );
        assert_eq!(Some(12), parse_duplicate_index("db(12)", "db"));
        assert_eq!(
            None,
            parse_duplicate_index("172.29.13.200", "172.29.13.200")
        );
        assert_eq!(None, parse_duplicate_index("db(0)", "db"));
        assert_eq!(None, parse_duplicate_index("db(x)", "db"));
        assert_eq!(None, parse_duplicate_index("other(1)", "db"));
    }

    #[test]
    fn next_duplicate_tab_index_starts_at_one_and_reuses_freed_numbers() {
        assert_eq!(
            None,
            next_duplicate_tab_index("db", ["other"].iter().copied())
        );

        assert_eq!(
            Some(1),
            next_duplicate_tab_index("db", ["db"].iter().copied())
        );

        assert_eq!(
            Some(1),
            next_duplicate_tab_index("db", ["db", "db(3)"].iter().copied())
        );

        assert_eq!(
            Some(2),
            next_duplicate_tab_index("db", ["db", "db(1)"].iter().copied())
        );
    }

    #[test]
    fn next_duplicate_tab_index_continues_when_original_is_gone() {
        assert_eq!(
            Some(2),
            next_duplicate_tab_index("db", ["db(1)"].iter().copied())
        );
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
