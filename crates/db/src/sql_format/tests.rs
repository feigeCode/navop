use super::*;

#[test]
fn format_sql_preserves_keyword_case_by_default() {
    let sql = "selecT id, name frOM users WHERe id = 1";
    let formatted = format_sql(sql);
    assert!(formatted.starts_with("selecT"));
    assert!(formatted.contains("\nfrom\n") || formatted.contains("frOM\n") || formatted.contains("\nfrOM"));
    assert!(formatted.contains("WHERe"));
    assert!(!formatted.contains("SELECT"));
}

#[test]
fn format_sql_supports_upper_and_lower_keyword_case() {
    let sql = "select id from users where id = 1";
    let upper = format_sql_with_options(
        sql,
        SqlFormatOptions {
            keyword_case: SqlKeywordCase::Upper,
            ..SqlFormatOptions::default()
        },
    );
    assert!(upper.contains("SELECT"));
    assert!(upper.contains("FROM"));
    assert!(upper.contains("WHERE"));

    let lower = format_sql_with_options(
        "SELECT id FROM users WHERE id = 1",
        SqlFormatOptions {
            keyword_case: SqlKeywordCase::Lower,
            ..SqlFormatOptions::default()
        },
    );
    assert!(lower.contains("select"));
    assert!(lower.contains("where"));
    assert!(!lower.contains("SELECT"));
}

#[test]
fn format_sql_applies_indent_options() {
    let sql = "select id, name from users where id = 1";
    let tabs = format_sql_with_options(
        sql,
        SqlFormatOptions {
            indent: SqlIndentStyle::Tabs,
            ..SqlFormatOptions::default()
        },
    );
    assert!(tabs.contains("\n\t"), "expected tab indentation: {tabs}");

    let four_spaces = format_sql_with_options(
        sql,
        SqlFormatOptions {
            indent: SqlIndentStyle::FourSpaces,
            ..SqlFormatOptions::default()
        },
    );
    assert!(
        four_spaces.contains("\n    "),
        "expected 4-space indentation: {four_spaces}"
    );
}

#[test]
fn format_sql_preserves_embedded_parameters() {
    let sql = "selecT * frOM ${table_name} WHERe ds = '${bizdate}' and id = #{userId} and created_at >= {{ params.start_date }}";
    let formatted = format_sql(sql);

    assert!(formatted.contains("${table_name}"));
    assert!(formatted.contains("'${bizdate}'"));
    assert!(formatted.contains("#{userId}"));
    assert!(formatted.contains("{{ params.start_date }}"));
}

#[test]
fn format_sql_preserves_nested_braces_inside_dynamic_script() {
    // issue #127：动态 SQL 片段内含嵌套大括号与引号，掩码必须整段保护
    let script = "${if(len(actual_controller_nm)==0,\"\",\" AND actual_controller_nm LIKE '%公司%' \")}";
    let sql = format!("SELECT * FROM t WHERE status = 1 {script} ORDER BY id");
    let formatted = format_sql(&sql);

    assert!(
        formatted.contains(script),
        "dynamic script should be preserved verbatim: {formatted}"
    );
}

#[test]
fn format_sql_preserves_braces_inside_parameter_quotes() {
    let sql = "select * from t where id = ${if(x==0,\"} freaky\",\"ok\")}";
    let formatted = format_sql(sql);
    assert!(formatted.contains("${if(x==0,\"} freaky\",\"ok\")}"));
}

#[test]
fn format_sql_preserves_nested_braces_in_double_brace_block() {
    let sql = "select * from t where created_at >= {{ wrap { inner } end }}";
    let formatted = format_sql(sql);
    assert!(formatted.contains("{{ wrap { inner } end }}"));
}

#[test]
fn format_sql_preserves_custom_wrappers() {
    let sql = "SELECT * FROM t WHERE status = 1 <% if (deleted) { %> AND deleted = 0 <% } %> ORDER BY id";
    let formatted = format_sql_with_options(
        sql,
        SqlFormatOptions {
            custom_wrappers: vec![("<%".to_string(), "%>".to_string())],
            ..SqlFormatOptions::default()
        },
    );

    assert!(formatted.contains("<% if (deleted) { %>"), "{formatted}");
    assert!(formatted.contains("AND deleted = 0"));
    assert!(formatted.contains("<% } %>"));
}

#[test]
fn custom_wrapper_takes_precedence_over_builtin_parameters() {
    let sql = "select * from t <% ${inner} and #{x} %>";
    let formatted = format_sql_with_options(
        sql,
        SqlFormatOptions {
            custom_wrappers: vec![("<%".to_string(), "%>".to_string())],
            ..SqlFormatOptions::default()
        },
    );

    assert!(formatted.contains("<% ${inner} and #{x} %>"), "{formatted}");
}

#[test]
fn custom_wrapper_with_longer_start_is_matched_first() {
    // ${ 与自定义起始符 ${! 前缀重叠时，优先匹配更长的自定义起始符
    let sql = "select * from t ${! keep !}";
    let formatted = format_sql_with_options(
        sql,
        SqlFormatOptions {
            custom_wrappers: vec![("${!".to_string(), "!}".to_string())],
            ..SqlFormatOptions::default()
        },
    );

    assert!(formatted.contains("${! keep !}"), "{formatted}");
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
    assert_eq!("SELECT id, name FROM users WHERE id = 1", compressed);
}
