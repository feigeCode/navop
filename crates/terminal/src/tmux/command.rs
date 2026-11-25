pub fn encode_command(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_tmux_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn quote_tmux_arg(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_./:@%".contains(ch))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn quote_shell_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn validate_literal_keys(keys: &str) -> Result<(), String> {
    if keys.is_empty() {
        return Err("keys must not be empty".to_string());
    }
    if keys
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return Err("keys must not contain newlines or control characters".to_string());
    }
    Ok(())
}

pub fn send_keys_command(pane_id: &str, keys: &str) -> Result<String, String> {
    validate_literal_keys(keys)?;
    Ok(encode_command(&[
        "send-keys".to_string(),
        "-l".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
        "--".to_string(),
        keys.to_string(),
    ]))
}

pub fn send_enter_command(pane_id: &str) -> String {
    encode_command(&[
        "send-keys".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
        "Enter".to_string(),
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmuxSplitPlacement {
    Left,
    Right,
    Top,
    Bottom,
}

pub fn split_window_command(pane_id: &str, placement: TmuxSplitPlacement) -> String {
    let horizontal = matches!(
        placement,
        TmuxSplitPlacement::Left | TmuxSplitPlacement::Right
    );
    let before = matches!(
        placement,
        TmuxSplitPlacement::Left | TmuxSplitPlacement::Top
    );
    let mut args = vec!["split-window".to_string()];
    args.push(if horizontal { "-h" } else { "-v" }.to_string());
    if before {
        args.push("-b".to_string());
    }
    args.extend(["-t".to_string(), pane_id.to_string()]);
    encode_command(&args)
}

pub fn select_window_command(window_id: &str) -> String {
    encode_command(&[
        "select-window".to_string(),
        "-t".to_string(),
        window_id.to_string(),
    ])
}

pub fn select_pane_command(pane_id: &str) -> String {
    encode_command(&[
        "select-pane".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        encode_command, quote_shell_arg, quote_tmux_arg, send_keys_command, validate_literal_keys,
    };

    #[test]
    fn tmux_arguments_are_quoted_and_escaped() {
        assert_eq!(quote_tmux_arg("%2"), "%2");
        assert_eq!(quote_tmux_arg("echo hi; rm -rf /"), "'echo hi; rm -rf /'");
        assert_eq!(quote_tmux_arg("a'b"), "'a'\\''b'");
        assert_eq!(
            encode_command(&["display-message".into(), "-p".into(), "a b".into()]),
            "display-message -p 'a b'"
        );
    }

    #[test]
    fn literal_send_keys_rejects_control_but_allows_tab() {
        assert!(validate_literal_keys("echo hi").is_ok());
        assert!(validate_literal_keys("echo\thi").is_ok());
        assert!(validate_literal_keys("echo\nhi").is_err());
        assert!(
            send_keys_command("%1", "echo 'x'; true")
                .unwrap()
                .contains("'echo '\\''x'\\''; true'")
        );
    }

    #[test]
    fn shell_quote_for_ssh_exec_handles_apostrophe() {
        assert_eq!(quote_shell_arg("a'b"), "'a'\\''b'");
    }

    #[test]
    fn split_window_maps_navop_placements_to_tmux_flags() {
        assert_eq!(
            super::split_window_command("%2", super::TmuxSplitPlacement::Left),
            "split-window -h -b -t %2"
        );
        assert_eq!(
            super::split_window_command("%2", super::TmuxSplitPlacement::Right),
            "split-window -h -t %2"
        );
        assert_eq!(
            super::split_window_command("%2", super::TmuxSplitPlacement::Top),
            "split-window -v -b -t %2"
        );
        assert_eq!(
            super::split_window_command("%2", super::TmuxSplitPlacement::Bottom),
            "split-window -v -t %2"
        );
    }

    #[test]
    fn selection_commands_use_explicit_targets() {
        assert_eq!(super::select_window_command("@2"), "select-window -t @2");
        assert_eq!(super::select_pane_command("%4"), "select-pane -t %4");
    }
}
