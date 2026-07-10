use std::cmp::Ordering;

use extension_runtime::RegisteredRemoteFileEditorContribution;

pub fn matches_file_mask(file_name: &str, mask: &str) -> bool {
    let name: Vec<_> = file_name.to_lowercase().chars().collect();
    let pattern: Vec<_> = mask.to_lowercase().chars().collect();
    let (mut name_index, mut pattern_index) = (0, 0);
    let (mut star_index, mut star_match) = (None, 0);

    while name_index < name.len() {
        if pattern
            .get(pattern_index)
            .is_some_and(|value| *value == '?' || *value == name[name_index])
        {
            name_index += 1;
            pattern_index += 1;
        } else if pattern.get(pattern_index) == Some(&'*') {
            star_index = Some(pattern_index);
            star_match = name_index;
            pattern_index += 1;
        } else if let Some(star) = star_index {
            star_match += 1;
            name_index = star_match;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }

    pattern[pattern_index..].iter().all(|value| *value == '*')
}

pub fn editor_supports_current_platform(platforms: &[String]) -> bool {
    let current = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    platforms.is_empty()
        || platforms
            .iter()
            .any(|value| value.eq_ignore_ascii_case(current))
}

pub fn editor_matches_file(file_name: &str, masks: &[String]) -> bool {
    masks.is_empty() || masks.iter().any(|mask| matches_file_mask(file_name, mask))
}

pub fn matching_editors(
    editors: &[RegisteredRemoteFileEditorContribution],
    file_name: &str,
    default_editor: Option<&str>,
) -> Vec<RegisteredRemoteFileEditorContribution> {
    let mut matches: Vec<_> = editors
        .iter()
        .filter(|editor| editor_supports_current_platform(&editor.platforms))
        .filter(|editor| editor_matches_file(file_name, &editor.file_masks))
        .cloned()
        .collect();
    matches.sort_by(|left, right| compare_editors(left, right, default_editor));
    matches
}

fn compare_editors(
    left: &RegisteredRemoteFileEditorContribution,
    right: &RegisteredRemoteFileEditorContribution,
    default_editor: Option<&str>,
) -> Ordering {
    let left_default = default_editor == Some(left.editor_key.as_str());
    let right_default = default_editor == Some(right.editor_key.as_str());
    right_default
        .cmp(&left_default)
        .then_with(|| right.priority.cmp(&left.priority))
        .then_with(|| left.display_name.cmp(&right.display_name))
}

#[cfg(test)]
mod tests {
    use extension_runtime::{
        RegisteredRemoteFileEditorCommand, RegisteredRemoteFileEditorContribution,
    };

    use super::{matches_file_mask, matching_editors};

    #[test]
    fn wildcard_mask_matches_file_extension() {
        assert!(matches_file_mask("app.log", "*.log"));
        assert!(!matches_file_mask("app.txt", "*.log"));
    }

    #[test]
    fn question_mark_matches_single_character() {
        assert!(matches_file_mask("app1.log", "app?.log"));
        assert!(!matches_file_mask("app10.log", "app?.log"));
    }

    #[test]
    fn star_mask_matches_any_file() {
        assert!(matches_file_mask("Dockerfile", "*"));
    }

    #[test]
    fn matching_editors_puts_default_before_priority() {
        let editors = vec![
            editor("com.a::low", "Low", 10),
            editor("com.b::high", "High", 100),
        ];

        let ordered = matching_editors(&editors, "app.rs", Some("com.a::low"));

        assert_eq!("com.a::low", ordered[0].editor_key);
        assert_eq!("com.b::high", ordered[1].editor_key);
    }

    fn editor(
        editor_key: &str,
        display_name: &str,
        priority: i32,
    ) -> RegisteredRemoteFileEditorContribution {
        RegisteredRemoteFileEditorContribution {
            extension_id: editor_key.split("::").next().unwrap().to_string(),
            id: editor_key.split("::").nth(1).unwrap().to_string(),
            editor_key: editor_key.to_string(),
            display_name: display_name.to_string(),
            platforms: Vec::new(),
            file_masks: vec!["*".to_string()],
            priority,
            command: RegisteredRemoteFileEditorCommand {
                program_candidates: vec!["editor".to_string()],
                args: Vec::new(),
            },
        }
    }
}
