use sqlformat::{FormatOptions, QueryParams, format};

/// SQL 美化：将 SQL 格式化为可读性更好的多行形式
pub fn format_sql(sql: &str) -> String {
    let (masked_sql, parameters) = mask_embedded_parameters(sql);
    let options = FormatOptions {
        uppercase: Some(false),
        ..FormatOptions::default()
    };
    let mut formatted = format(&masked_sql, &QueryParams::None, &options);

    for (marker, parameter) in parameters {
        formatted = formatted.replace(&marker, &parameter);
    }

    formatted
}

/// SQL 压缩：将 SQL 压缩为单行形式
pub fn compress_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn mask_embedded_parameters(sql: &str) -> (String, Vec<(String, String)>) {
    let marker_prefix = unique_marker_prefix(sql);
    let mut masked = String::with_capacity(sql.len());
    let mut parameters = Vec::new();
    let mut cursor = 0;

    while cursor < sql.len() {
        let remaining = &sql[cursor..];
        let (closing, closing_len) = if remaining.starts_with("{{") {
            ("}}", 2)
        } else if remaining.starts_with("${") || remaining.starts_with("#{") {
            ("}", 1)
        } else {
            let char_len = remaining
                .chars()
                .next()
                .expect("cursor is before the end of the SQL")
                .len_utf8();
            masked.push_str(&remaining[..char_len]);
            cursor += char_len;
            continue;
        };

        let search_start = 2;
        let Some(relative_end) = remaining[search_start..].find(closing) else {
            let char_len = remaining
                .chars()
                .next()
                .expect("cursor is before the end of the SQL")
                .len_utf8();
            masked.push_str(&remaining[..char_len]);
            cursor += char_len;
            continue;
        };
        let parameter_end = search_start + relative_end + closing_len;
        let parameter = &remaining[..parameter_end];
        let marker = format!("{marker_prefix}{}__", parameters.len());
        masked.push_str(&marker);
        parameters.push((marker, parameter.to_owned()));
        cursor += parameter_end;
    }

    (masked, parameters)
}

fn unique_marker_prefix(sql: &str) -> String {
    let base = "__navop_sql_parameter_";
    let mut prefix = base.to_owned();

    while sql.contains(&prefix) {
        prefix.push('_');
    }

    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sql_normalizes_keywords_to_lowercase() {
        let sql = "selecT id, name frOM users WHERe id = 1";
        let formatted = format_sql(sql);
        assert!(formatted.starts_with("select"));
        assert!(formatted.contains("\nfrom\n"));
        assert!(formatted.contains("\nwhere\n"));
        assert!(!formatted.contains("selecT"));
        assert!(!formatted.contains("frOM"));
        assert!(!formatted.contains("WHERe"));
    }

    #[test]
    fn format_sql_preserves_embedded_parameters() {
        let sql = "selecT * frOM ${table_name} WHERe ds = '${bizdate}' and id = #{userId} and created_at >= {{ params.start_date }}";
        let formatted = format_sql(sql);

        assert!(formatted.starts_with("select"));
        assert!(formatted.contains("${table_name}"));
        assert!(formatted.contains("'${bizdate}'"));
        assert!(formatted.contains("#{userId}"));
        assert!(formatted.contains("{{ params.start_date }}"));
    }

    #[test]
    fn format_sql_preserves_unclosed_parameter_like_text() {
        let sql = "select '${unfinished' as value";
        let formatted = format_sql(sql);

        assert!(formatted.contains("'${unfinished'"));
    }

    #[test]
    fn format_sql_handles_parameter_marker_text_in_input() {
        let sql = "select '__navop_sql_parameter_0__', ${actual}";
        let formatted = format_sql(sql);

        assert!(formatted.contains("'__navop_sql_parameter_0__'"));
        assert!(formatted.contains("${actual}"));
    }

    #[test]
    fn test_compress_sql() {
        let sql = "SELECT\n  id,\n  name\nFROM\n  users\nWHERE\n  id = 1";
        let compressed = compress_sql(sql);
        assert_eq!(compressed, "SELECT id, name FROM users WHERE id = 1");
    }
}
