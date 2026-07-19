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
fn detect_unbracketed_paste_hazard_matches_line_continuation() {
    assert!(has_trailing_line_continuation("echo hello \\\nworld"));
    assert_eq!(
        detect_unbracketed_paste_hazard("echo hello \\\nworld"),
        Some(UnbracketedPasteHazard::LineContinuation)
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
    assert!(!has_trailing_line_continuation("echo hello\necho world"));
}
