use one_core::storage::QuickCommand;
use std::collections::HashSet;

pub(super) const COMMAND_SUGGESTION_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandSuggestionKind {
    QuickCommand,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandSuggestion {
    pub command: String,
    pub label: String,
    pub detail: Option<String>,
    pub kind: CommandSuggestionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectionDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug)]
pub(super) struct QuickCommandGroup {
    pub name: Option<String>,
    pub color: Option<String>,
    pub commands: Vec<QuickCommand>,
}

pub(super) fn build_command_suggestions(
    query: &str,
    quick_commands: &[QuickCommand],
    history: &[String],
) -> Vec<CommandSuggestion> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();
    for item in quick_commands {
        if !quick_command_matches(item, &query) || !seen.insert(item.command.to_lowercase()) {
            continue;
        }
        suggestions.push(CommandSuggestion {
            command: item.command.clone(),
            label: item.name.clone().unwrap_or_else(|| item.command.clone()),
            detail: item.description.clone().or_else(|| item.group_name.clone()),
            kind: CommandSuggestionKind::QuickCommand,
        });
        if suggestions.len() == COMMAND_SUGGESTION_LIMIT {
            return suggestions;
        }
    }

    for command in history {
        let Some(command) = normalize_history_suggestion(command) else {
            continue;
        };
        if !command.to_lowercase().contains(&query) || !seen.insert(command.to_lowercase()) {
            continue;
        }
        suggestions.push(CommandSuggestion {
            command: command.clone(),
            label: command,
            detail: None,
            kind: CommandSuggestionKind::History,
        });
        if suggestions.len() == COMMAND_SUGGESTION_LIMIT {
            break;
        }
    }
    suggestions
}

pub(super) fn command_inline_suffix(
    query: &str,
    suggestions: &[CommandSuggestion],
) -> Option<String> {
    if query.is_empty() || query.contains(['\r', '\n']) {
        return None;
    }
    suggestions.iter().find_map(|suggestion| {
        suggestion
            .command
            .strip_prefix(query)
            .filter(|suffix| !suffix.is_empty() && !suffix.contains(['\r', '\n']))
            .map(str::to_string)
    })
}

pub(super) fn group_quick_commands(
    commands: &[QuickCommand],
    query: &str,
) -> Vec<QuickCommandGroup> {
    let query = query.trim().to_lowercase();
    let mut groups = Vec::<QuickCommandGroup>::new();
    for command in commands {
        if !query.is_empty() && !quick_command_matches(command, &query) {
            continue;
        }
        let name = command
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        if let Some(group) = groups.iter_mut().find(|group| group.name == name) {
            group.commands.push(command.clone());
        } else {
            groups.push(QuickCommandGroup {
                name,
                color: command.group_color.clone(),
                commands: vec![command.clone()],
            });
        }
    }
    groups
}

pub(super) fn next_selection(
    item_count: usize,
    current: Option<usize>,
    direction: SelectionDirection,
) -> Option<usize> {
    if item_count == 0 {
        return None;
    }
    match direction {
        SelectionDirection::Next => Some(current.map_or(0, |index| (index + 1) % item_count)),
        SelectionDirection::Previous => Some(current.map_or(item_count - 1, |index| {
            if index == 0 {
                item_count - 1
            } else {
                index - 1
            }
        })),
    }
}

pub(super) fn bounded_selection(
    item_count: usize,
    current: Option<usize>,
    direction: SelectionDirection,
) -> Option<usize> {
    let last = item_count.checked_sub(1)?;
    match direction {
        SelectionDirection::Next => Some(current.map_or(0, |index| (index + 1).min(last))),
        SelectionDirection::Previous => {
            Some(current.map_or(last, |index| index.min(last).saturating_sub(1)))
        }
    }
}

pub(super) fn selected_quick_command(
    commands: &[QuickCommand],
    selected: Option<usize>,
) -> Option<String> {
    commands
        .get(selected.unwrap_or(0))
        .map(|command| command.command.clone())
}

pub(super) fn command_submission_bytes(command: &str) -> Option<Vec<u8>> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    let mut bytes = command.as_bytes().to_vec();
    bytes.push(b'\r');
    Some(bytes)
}

/// Split a possibly multi-line command into individual statements so batch
/// input can be executed line by line.
pub(super) fn command_batch_lines(command: &str) -> Vec<String> {
    command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn quick_command_matches(command: &QuickCommand, query: &str) -> bool {
    [
        Some(command.command.as_str()),
        command.name.as_deref(),
        command.description.as_deref(),
        command.group_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(query))
}

fn normalize_history_suggestion(command: &str) -> Option<String> {
    let mut command = command.trim();
    while let Some(stripped) = command
        .strip_suffix("\\n")
        .or_else(|| command.strip_suffix("\\r"))
    {
        command = stripped.trim_end();
    }
    (!command.is_empty() && !command.contains(['\r', '\n'])).then(|| command.to_string())
}

#[cfg(test)]
#[path = "command_bar_model_tests.rs"]
mod tests;
