#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnbracketedPasteHazard {
    HereDoc,
    UnterminatedQuote,
    LineContinuation,
}

pub(super) fn multiline_non_empty_line_count(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

fn contains_heredoc_operator(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        !line.is_empty() && !line.starts_with('#') && line.contains("<<")
    })
}

pub(super) fn has_trailing_line_continuation(text: &str) -> bool {
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if lines.peek().is_none() {
            break;
        }

        let trimmed = line.trim_end();
        if !trimmed.is_empty() && trimmed.ends_with('\\') {
            return true;
        }
    }

    false
}

pub(super) fn has_unterminated_shell_quote(text: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in text.chars() {
        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '\'' => in_single_quote = true,
            '"' => in_double_quote = !in_double_quote,
            _ => {}
        }
    }

    in_single_quote || in_double_quote
}

pub(super) fn detect_unbracketed_paste_hazard(text: &str) -> Option<UnbracketedPasteHazard> {
    if contains_heredoc_operator(text) {
        return Some(UnbracketedPasteHazard::HereDoc);
    }

    if has_trailing_line_continuation(text) {
        return Some(UnbracketedPasteHazard::LineContinuation);
    }

    if has_unterminated_shell_quote(text) {
        return Some(UnbracketedPasteHazard::UnterminatedQuote);
    }

    None
}
