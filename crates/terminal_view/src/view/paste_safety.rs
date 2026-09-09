#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnbracketedPasteHazard {
    HereDoc,
    UnterminatedQuote,
}

use std::borrow::Cow;

pub(super) fn multiline_non_empty_line_count(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

fn contains_heredoc_operator(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        !line.is_empty() && !line.starts_with('#') && line.contains("<<")
    })
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

    if has_unterminated_shell_quote(text) {
        return Some(UnbracketedPasteHazard::UnterminatedQuote);
    }

    None
}

/// 大体量粘贴确认阈值：非空行数或字节数任一超限即弹确认。
const LARGE_PASTE_MAX_LINES: usize = 200;
const LARGE_PASTE_MAX_BYTES: usize = 32 * 1024;

pub(super) fn is_large_paste(text: &str) -> bool {
    multiline_non_empty_line_count(text) > LARGE_PASTE_MAX_LINES
        || text.len() > LARGE_PASTE_MAX_BYTES
}

/// 非 bracketed 粘贴前过滤危险控制字符（保留换行与制表符），
/// 与 bracketed 路径移除 ESC 的做法保持一致的防护语义。
pub(super) fn strip_dangerous_control_chars(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Cow::Borrowed(text);
    }

    Cow::Owned(
        text.chars()
            .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
            .collect(),
    )
}
