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

        let Some(parameter_end) = scan_builtin(remaining) else {
            // 未闭合的片段原样保留，交由后续字符逐个透传
            push_char(remaining, &mut masked, &mut cursor);
            continue;
        };

        let parameter = &remaining[..parameter_end];
        let marker = format!("{marker_prefix}{}__", parameters.len());
        masked.push_str(&marker);
        parameters.push((marker, parameter.to_owned()));
        cursor += parameter_end;
    }

    (masked, parameters)
}

/// 匹配内置模板片段（`${`/`#{` 单括号配平，`{{` 双括号配平），返回消耗的字节数
fn scan_builtin(remaining: &str) -> Option<usize> {
    let (initial_depth, opening_len) = if remaining.starts_with("${") || remaining.starts_with("#{")
    {
        (1, 2)
    } else if remaining.starts_with("{{") {
        (2, 2)
    } else {
        return None;
    };

    let body = &remaining[opening_len..];
    let relative_len = scan_balanced(body, initial_depth)?;
    Some(opening_len + relative_len)
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
