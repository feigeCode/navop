use std::collections::{HashSet, VecDeque};

pub const SESSION_HISTORY_LIMIT: usize = 256;
pub const PERSISTED_HISTORY_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellHistoryFormat {
    Bash,
    Zsh,
}

pub fn normalize_history_command(command: &str) -> Option<String> {
    let normalized = command.trim();
    (!normalized.is_empty()).then(|| normalized.to_string())
}

pub fn parse_shell_history(contents: &str, format: ShellHistoryFormat) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| match format {
            ShellHistoryFormat::Bash => normalize_history_command(line),
            ShellHistoryFormat::Zsh => {
                if let Some((_, command)) = line.split_once(';') {
                    normalize_history_command(command)
                } else {
                    normalize_history_command(line)
                }
            }
        })
        .collect()
}

pub fn push_history_entry(entries: &mut VecDeque<String>, command: &str, limit: usize) -> bool {
    let Some(command) = normalize_history_command(command) else {
        return false;
    };

    if entries.back().is_some_and(|existing| existing == &command) {
        return false;
    }

    entries.push_back(command);
    while entries.len() > limit.max(1) {
        entries.pop_front();
    }
    true
}

pub fn collect_history_suggestions(
    session: &VecDeque<String>,
    persisted: &[String],
    prefix: &str,
    limit: usize,
) -> Vec<String> {
    let limit = limit.max(1);
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut matches = Vec::new();

    for command in session.iter().rev().chain(persisted.iter().rev()) {
        if !prefix.is_empty() && !command.starts_with(prefix) {
            continue;
        }
        if !seen.insert(command.clone()) {
            continue;
        }
        matches.push(command.clone());
        if matches.len() >= limit {
            break;
        }
    }

    matches
}

fn collect_unique_history_commands(session: &VecDeque<String>, persisted: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut commands = Vec::new();

    for command in session.iter().rev().chain(persisted.iter().rev()) {
        if !seen.insert(command.clone()) {
            continue;
        }
        commands.push(command.clone());
    }

    commands
}

fn has_token_prefix(command: &str, query: &str) -> bool {
    command.split_whitespace().any(|token| token.starts_with(query))
}

fn is_subsequence(query: &str, command: &str) -> bool {
    let mut query_chars = query.chars();
    let mut current = query_chars.next();
    if current.is_none() {
        return true;
    }

    for ch in command.chars() {
        if Some(ch) == current {
            current = query_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }

    false
}

fn history_search_rank(command: &str, query: &str) -> Option<u8> {
    if command.starts_with(query) {
        Some(0)
    } else if has_token_prefix(command, query) {
        Some(1)
    } else if command.contains(query) {
        Some(2)
    } else if is_subsequence(query, command) {
        Some(3)
    } else {
        None
    }
}

pub fn collect_history_search_results(
    session: &VecDeque<String>,
    persisted: &[String],
    query: &str,
    limit: usize,
) -> Vec<String> {
    let limit = limit.max(1);
    let query = query.trim();
    let commands = collect_unique_history_commands(session, persisted);
    if query.is_empty() {
        return commands.into_iter().take(limit).collect();
    }

    let lowered_query = query.to_lowercase();
    let mut matches = commands
        .into_iter()
        .enumerate()
        .filter_map(|(recency, command)| {
            let lowered_command = command.to_lowercase();
            let rank = history_search_rank(&lowered_command, &lowered_query)?;
            Some((rank, recency, command))
        })
        .collect::<Vec<_>>();

    matches.sort_by_key(|(rank, recency, _)| (*rank, *recency));
    matches.truncate(limit);
    matches
        .into_iter()
        .map(|(_, _, command)| command)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        collect_history_search_results, collect_history_suggestions, normalize_history_command,
        parse_shell_history, push_history_entry, ShellHistoryFormat,
    };
    use std::collections::VecDeque;

    #[test]
    fn normalize_history_command_trims_and_rejects_blank_input() {
        assert_eq!(
            normalize_history_command("  git status  "),
            Some("git status".to_string())
        );
        assert_eq!(normalize_history_command("   "), None);
        assert_eq!(normalize_history_command("\n\t"), None);
    }

    #[test]
    fn parse_shell_history_supports_zsh_extended_format() {
        let commands = parse_shell_history(
            ": 1710000000:0;git status\n: 1710000001:0;cargo test\n",
            ShellHistoryFormat::Zsh,
        );

        assert_eq!(commands, vec!["git status", "cargo test"]);
    }

    #[test]
    fn push_history_entry_dedupes_adjacent_duplicates() {
        let mut entries = VecDeque::new();
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "cargo test", 5);

        assert_eq!(
            entries.into_iter().collect::<Vec<_>>(),
            vec!["git status", "cargo test"]
        );
    }

    #[test]
    fn collect_history_suggestions_prioritizes_session_history() {
        let session = VecDeque::from([
            "git status".to_string(),
            "git stash".to_string(),
            "cargo test".to_string(),
        ]);
        let persisted = vec![
            "git status".to_string(),
            "git switch main".to_string(),
            "git commit".to_string(),
        ];

        let matches = collect_history_suggestions(&session, &persisted, "git s", 4);

        assert_eq!(
            matches,
            vec![
                "git stash".to_string(),
                "git status".to_string(),
                "git switch main".to_string()
            ]
        );
    }

    #[test]
    fn collect_history_suggestions_skips_empty_prefix() {
        let session = VecDeque::from(["git status".to_string()]);
        let persisted = vec!["git switch".to_string()];

        let matches = collect_history_suggestions(&session, &persisted, "   ", 5);

        assert!(matches.is_empty());
    }

    #[test]
    fn collect_history_search_results_prefers_prefix_then_token_then_subsequence() {
        let session = VecDeque::from([
            "git log".to_string(),
            "git status".to_string(),
            "git stash".to_string(),
            "cargo test".to_string(),
        ]);
        let persisted = vec![
            "status report".to_string(),
            "cargo status".to_string(),
            "gitstatus checkout".to_string(),
        ];

        let matches = collect_history_search_results(&session, &persisted, "status", 5);

        assert_eq!(
            matches,
            vec![
                "status report".to_string(),
                "git status".to_string(),
                "cargo status".to_string(),
                "gitstatus checkout".to_string(),
            ]
        );
    }

    #[test]
    fn collect_history_search_results_breaks_ties_by_session_recency() {
        let session = VecDeque::from([
            "git status".to_string(),
            "git stash".to_string(),
            "go test ./...".to_string(),
        ]);
        let persisted = vec![
            "gh stack sync".to_string(),
            "git switch main".to_string(),
            "cargo test".to_string(),
        ];

        let matches = collect_history_search_results(&session, &persisted, "gsta", 3);

        assert_eq!(
            matches,
            vec![
                "git stash".to_string(),
                "git status".to_string(),
                "git switch main".to_string(),
            ]
        );
    }
}
