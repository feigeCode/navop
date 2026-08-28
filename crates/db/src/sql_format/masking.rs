/// 内嵌参数掩码：把 `${...}`、`#{...}`、`{{...}}` 模板片段替换为占位标记，
/// 避免 sqlformat 把动态 SQL 片段当作普通文本排版破坏。
/// 扫描按大括号配平计数，字符串字面量（含反斜杠转义）内的大括号不参与配平。
pub(super) fn mask_embedded_parameters(sql: &str) -> (String, Vec<(String, String)>) {
    let marker_prefix = unique_marker_prefix(sql);
    let mut masked = String::with_capacity(sql.len());
    let mut parameters = Vec::new();
    let mut cursor = 0;

    while cursor < sql.len() {
        let remaining = &sql[cursor..];
        let (initial_depth, opening_len) = match opening_kind(remaining) {
            Some(OpeningKind::Brace) => (1, 2),
            Some(OpeningKind::DoubleBrace) => (2, 2),
            None => {
                push_char(remaining, &mut masked, &mut cursor);
                continue;
            }
        };

        let Some(parameter_len) = scan_balanced(&remaining[opening_len..], initial_depth) else {
            // 未闭合的片段原样保留，交由后续字符逐个透传
            push_char(remaining, &mut masked, &mut cursor);
            continue;
        };
        let parameter_end = opening_len + parameter_len;
        let parameter = &remaining[..parameter_end];
        let marker = format!("{marker_prefix}{}__", parameters.len());
        masked.push_str(&marker);
        parameters.push((marker, parameter.to_owned()));
        cursor += parameter_end;
    }

    (masked, parameters)
}

enum OpeningKind {
    /// `${` 或 `#{`，以单个 `}` 收尾
    Brace,
    /// `{{`，以配平的 `}}` 收尾
    DoubleBrace,
}

fn opening_kind(s: &str) -> Option<OpeningKind> {
    if s.starts_with("${") || s.starts_with("#{") {
        Some(OpeningKind::Brace)
    } else if s.starts_with("{{") {
        Some(OpeningKind::DoubleBrace)
    } else {
        None
    }
}

/// 从起始大括号之后扫描到配平的关闭大括号，返回消耗的字节数（含关闭括号）。
fn scan_balanced(body: &str, initial_depth: usize) -> Option<usize> {
    let mut depth = initial_depth;
    let mut in_quote: Option<u8> = None;
    let mut escaped = false;

    for (index, byte) in body.bytes().enumerate() {
        if let Some(quote) = in_quote {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b if b == quote => in_quote = None,
                _ => {}
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => in_quote = Some(byte),
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn push_char(remaining: &str, masked: &mut String, cursor: &mut usize) {
    let char_len = remaining
        .chars()
        .next()
        .expect("cursor is before the end of the SQL")
        .len_utf8();
    masked.push_str(&remaining[..char_len]);
    *cursor += char_len;
}

fn unique_marker_prefix(sql: &str) -> String {
    const BASE: &str = "__navop_sql_parameter_";
    let mut prefix = BASE.to_owned();

    while sql.contains(&prefix) {
        prefix.push('_');
    }

    prefix
}
