use super::*;

#[test]
fn multiline_non_empty_line_count_ignores_blank_lines() {
    assert_eq!(multiline_non_empty_line_count("echo 1\n\n echo 2\n"), 2);
    assert_eq!(multiline_non_empty_line_count("echo 1"), 1);
}
#[test]
fn bracketed_paste_bytes_normalize_crlf_to_single_newlines() {
    let bytes = terminal_paste_bytes("alpha\r\nbeta\r\n", TermMode::BRACKETED_PASTE);

    assert_eq!(b"\x1b[200~alpha\nbeta\n\x1b[201~".to_vec(), bytes);
}

#[test]
fn detect_unbracketed_paste_hazard_matches_heredoc() {
    let text = "cat <<EOF\nhello\nEOF";
    assert_eq!(
        detect_unbracketed_paste_hazard(text),
        Some(UnbracketedPasteHazard::HereDoc)
    );
}

#[test]
fn detect_unbracketed_paste_hazard_does_not_block_line_continuation() {
    // 反斜杠续行是合法 shell 写法（如多行 wget/curl），不应被硬阻断；
    // 无 bracketed paste 时仅由普通多行粘贴确认流程兜底。
    assert_eq!(
        detect_unbracketed_paste_hazard("echo hello \\\nworld"),
        None
    );
}

#[test]
fn detect_unbracketed_paste_hazard_matches_unterminated_quote() {
    assert!(has_unterminated_shell_quote("printf 'hello\nworld"));
    assert_eq!(
        detect_unbracketed_paste_hazard("printf 'hello\nworld"),
        Some(UnbracketedPasteHazard::UnterminatedQuote)
    );
}

#[test]
fn detect_unbracketed_paste_hazard_ignores_plain_text() {
    assert_eq!(
        detect_unbracketed_paste_hazard("printf '%s\\n' hello"),
        None
    );
    assert!(!has_unterminated_shell_quote("printf '%s\\n' hello"));
}

#[test]
fn is_large_paste_triggers_on_line_count_or_bytes() {
    let two_hundred_lines = (0..200)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!is_large_paste(&two_hundred_lines));

    let over_lines = (0..201)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(is_large_paste(&over_lines));

    let over_bytes = "a".repeat(32 * 1024 + 1);
    assert!(is_large_paste(&over_bytes));

    assert!(!is_large_paste("echo hello"));
}

#[test]
fn strip_dangerous_control_chars_keeps_newline_and_tab() {
    assert_eq!(
        strip_dangerous_control_chars("echo\x07 hi\nworld\t!"),
        "echo hi\nworld\t!"
    );
    // ESC 单字符被移除；其后的序列字面不属于控制字符，由远端程序自行处理。
    assert_eq!(strip_dangerous_control_chars("echo\x1b[A hi"), "echo[A hi");
    assert_eq!(strip_dangerous_control_chars("plain text"), "plain text");
}

#[test]
fn terminal_paste_bytes_filters_control_chars_when_unbracketed() {
    let bytes = terminal_paste_bytes("echo\x07 bell\r\nline", TermMode::empty());
    assert_eq!(b"echo bell\nline".to_vec(), bytes);
}

#[test]
fn terminal_paste_bytes_keeps_bracketed_wrap_untouched() {
    let bytes = terminal_paste_bytes("echo hi", TermMode::BRACKETED_PASTE);
    assert_eq!(b"\x1b[200~echo hi\x1b[201~".to_vec(), bytes);
}

#[test]
fn join_paste_as_single_line_collapses_blank_lines() {
    assert_eq!(
        join_paste_as_single_line("  echo 1 \n\n  echo 2  \n"),
        "echo 1 echo 2"
    );
    assert_eq!(join_paste_as_single_line("single"), "single");
}
