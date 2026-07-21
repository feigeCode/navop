use super::*;

#[test]
fn suggestions_prioritize_quick_commands_and_deduplicate_history() {
    let quick_commands = vec![
        command("git status", Some("Status"), Some("Git"), true, 1),
        command("git log --oneline", None, Some("Git"), false, 2),
    ];
    let history = vec![
        "git status".to_string(),
        "git switch main".to_string(),
        "pwd".to_string(),
    ];

    let suggestions = build_command_suggestions("git", &quick_commands, &history);

    assert_eq!(3, suggestions.len());
    assert_eq!("git status", suggestions[0].command);
    assert_eq!(CommandSuggestionKind::QuickCommand, suggestions[0].kind);
    assert_eq!("git log --oneline", suggestions[1].command);
    assert_eq!("git switch main", suggestions[2].command);
    assert_eq!(CommandSuggestionKind::History, suggestions[2].kind);
}

#[test]
fn suggestions_match_name_description_and_command_case_insensitively() {
    let mut deploy = command(
        "kubectl rollout status deployment/app",
        Some("Deploy Status"),
        Some("Kubernetes"),
        false,
        0,
    );
    deploy.description = Some("Check production rollout".to_string());

    let suggestions = build_command_suggestions("PRODUCTION", &[deploy], &[]);

    assert_eq!(1, suggestions.len());
    assert_eq!("Deploy Status", suggestions[0].label);
}

#[test]
fn suggestions_normalize_recorded_line_endings_before_display() {
    let history = vec![
        "docker system df\\n".to_string(),
        "df -h\r\n".to_string(),
        "\n\t".to_string(),
    ];

    let suggestions = build_command_suggestions("df", &[], &history);

    assert_eq!(2, suggestions.len());
    assert_eq!("docker system df", suggestions[0].command);
    assert_eq!("docker system df", suggestions[0].label);
    assert_eq!("df -h", suggestions[1].command);
}

#[test]
fn inline_suffix_uses_the_first_prefix_completion_only() {
    let suggestions = build_command_suggestions(
        "df",
        &[command(
            "df -h",
            Some("Disk Usage"),
            Some("System"),
            true,
            0,
        )],
        &["docker system df".to_string()],
    );

    assert_eq!(
        Some(" -h".to_string()),
        command_inline_suffix("df", &suggestions)
    );
    assert_eq!(None, command_inline_suffix("DF", &suggestions));
}

#[test]
fn inline_suffix_is_hidden_for_complete_or_multiline_input() {
    let suggestions = build_command_suggestions(
        "git status",
        &[command("git status", Some("Status"), Some("Git"), true, 0)],
        &[],
    );

    assert_eq!(None, command_inline_suffix("git status", &suggestions));
    assert_eq!(None, command_inline_suffix("git\nstatus", &suggestions));
}

#[test]
fn grouped_quick_commands_keep_named_groups_and_ungrouped_items() {
    let commands = vec![
        command("git status", Some("Status"), Some("Git"), true, 0),
        command("docker ps", None, Some("Docker"), false, 1),
        command("pwd", None, None, false, 2),
    ];

    let groups = group_quick_commands(&commands, "");

    assert_eq!(3, groups.len());
    assert_eq!(Some("Git"), groups[0].name.as_deref());
    assert_eq!(Some("Docker"), groups[1].name.as_deref());
    assert_eq!(None, groups[2].name);
    assert_eq!("pwd", groups[2].commands[0].command);
}

#[test]
fn grouped_quick_commands_filter_across_metadata() {
    let mut command = command(
        "docker compose up -d",
        Some("Start stack"),
        Some("Containers"),
        false,
        0,
    );
    command.description = Some("Launch local services".to_string());

    let groups = group_quick_commands(&[command], "SERVICES");

    assert_eq!(1, groups.len());
    assert_eq!("docker compose up -d", groups[0].commands[0].command);
}

#[test]
fn selection_wraps_in_both_directions() {
    assert_eq!(Some(0), next_selection(3, None, SelectionDirection::Next));
    assert_eq!(
        Some(1),
        next_selection(3, Some(0), SelectionDirection::Next)
    );
    assert_eq!(
        Some(0),
        next_selection(3, Some(2), SelectionDirection::Next)
    );
    assert_eq!(
        Some(2),
        next_selection(3, None, SelectionDirection::Previous)
    );
    assert_eq!(
        Some(2),
        next_selection(3, Some(0), SelectionDirection::Previous)
    );
    assert_eq!(None, next_selection(0, Some(0), SelectionDirection::Next));
}

#[test]
fn quick_command_selection_stops_at_list_boundaries() {
    assert_eq!(
        Some(0),
        bounded_selection(3, None, SelectionDirection::Next)
    );
    assert_eq!(
        Some(2),
        bounded_selection(3, None, SelectionDirection::Previous)
    );
    assert_eq!(
        Some(2),
        bounded_selection(3, Some(2), SelectionDirection::Next)
    );
    assert_eq!(
        Some(0),
        bounded_selection(3, Some(0), SelectionDirection::Previous)
    );
    assert_eq!(
        None,
        bounded_selection(0, Some(0), SelectionDirection::Next)
    );
}

#[test]
fn quick_command_enter_uses_highlight_or_first_visible_command() {
    let commands = vec![
        command("pwd", Some("PWD"), Some("Files"), false, 0),
        command("git status", Some("Status"), Some("Git"), false, 1),
    ];

    assert_eq!(
        Some("pwd".to_string()),
        selected_quick_command(&commands, None)
    );
    assert_eq!(
        Some("git status".to_string()),
        selected_quick_command(&commands, Some(1))
    );
    assert_eq!(None, selected_quick_command(&commands, Some(2)));
    assert_eq!(None, selected_quick_command(&[], None));
}

#[test]
fn command_submission_trims_outer_whitespace_and_appends_enter() {
    assert_eq!(
        Some(b"printf 'hello'\r".to_vec()),
        command_submission_bytes("  printf 'hello' \n")
    );
    assert_eq!(None, command_submission_bytes(" \n\t "));
}

#[test]
fn command_batch_lines_split_trim_and_drop_empty_lines() {
    assert_eq!(
        vec![
            "ls -la".to_string(),
            "echo hi".to_string(),
            "pwd".to_string()
        ],
        command_batch_lines("  ls -la  \n\n echo hi \r\npwd\n  ")
    );
    assert_eq!(vec!["ls".to_string()], command_batch_lines("ls"));
    assert!(command_batch_lines(" \n\t \n").is_empty());
}

fn command(
    value: &str,
    name: Option<&str>,
    group: Option<&str>,
    pinned: bool,
    sort_order: i32,
) -> QuickCommand {
    let mut command = QuickCommand::new(value.to_string());
    command.name = name.map(str::to_string);
    command.group_name = group.map(str::to_string);
    command.group_color = group.map(|_| "blue".to_string());
    command.pinned = pinned;
    command.sort_order = sort_order;
    command
}
