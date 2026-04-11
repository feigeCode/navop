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
