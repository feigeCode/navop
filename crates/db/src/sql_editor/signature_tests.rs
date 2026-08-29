use super::signature::{SqlRoutineSignature, SqlSignatureHelp, signature_help};

fn routine(name: &str, params: &[&str]) -> SqlRoutineSignature {
    SqlRoutineSignature {
        identity: name.to_string(),
        label: format!("{}({})", name, params.join(", ")),
        parameters: params.iter().map(|s| s.to_string()).collect(),
        return_type: Some("text".to_string()),
        documentation: None,
    }
}

fn cookie_routines() -> Vec<SqlRoutineSignature> {
    vec![
        routine("COALESCE", &["value1", "value2", "value3"]),
        routine("SUBSTRING", &["str", "start", "len"]),
    ]
}

#[test]
fn active_parameter_is_zero_within_first_argument() {
    let sql = "SELECT COALESCE(|x, y) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!(0, result.active_parameter);
    assert_eq!("COALESCE", result.signatures[0].identity);
}

#[test]
fn active_parameter_advances_across_commas() {
    let sql = "SELECT COALESCE(x, |y) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!(1, result.active_parameter);
}

#[test]
fn active_parameter_counts_nested_call_commas_correctly() {
    // 内层调用 SUBSTRING(a, b) 的逗号不计入外层。
    let sql = "SELECT COALESCE(SUBSTRING(a, b), |c) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    // 光标在外层 COALESCE 的第二个参数（c），内层逗号不计。
    assert_eq!(1, result.active_parameter);
    assert_eq!("COALESCE", result.signatures[0].identity);
}

#[test]
fn nested_call_resolves_to_innermost() {
    let sql = "SELECT COALESCE(SUBSTRING(a, |b), c) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!("SUBSTRING", result.signatures[0].identity);
    assert_eq!(1, result.active_parameter);
}

#[test]
fn comma_inside_string_literal_does_not_count() {
    let sql = "SELECT COALESCE('a,b', |x) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!(1, result.active_parameter);
}

#[test]
fn comma_inside_comment_does_not_count() {
    let sql = "SELECT COALESCE(x /* , */, |y) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!(1, result.active_parameter);
}

#[test]
fn cursor_after_closing_paren_returns_none() {
    let sql = "SELECT COALESCE(x, y)| FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines());
    assert!(result.is_none());
}

#[test]
fn unknown_function_returns_none() {
    let sql = "SELECT NOTAFUNC(|x)";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines());
    assert!(result.is_none());
}

#[test]
fn overload_signatures_are_all_returned() {
    let mut routines = cookie_routines();
    let overloaded = routine("SUBSTRING", &["str", "start"]);
    routines.push(overloaded);
    let sql = "SELECT SUBSTRING(|a)";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &routines).unwrap();
    assert_eq!(2, result.signatures.len());
}

#[test]
fn quoted_function_name_is_unquoted_for_matching() {
    let routines = vec![routine("MyFunc", &["a", "b"])];
    let sql = "SELECT \"MyFunc\"(|x)";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &routines).unwrap();
    assert_eq!(1, result.signatures.len());
    assert_eq!("MyFunc", result.signatures[0].identity);
}

#[test]
fn matching_is_case_insensitive() {
    let sql = "SELECT coalesce(|x)";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines()).unwrap();
    assert_eq!("COALESCE", result.signatures[0].identity);
}

#[test]
fn call_range_marks_from_name_to_closing_paren() {
    let sql = "SELECT COALESCE(x, y) FROM t";
    // 光标在 COALESCE 内任意位置。
    let cursor = sql.find("(x").unwrap() + 1;
    let result = signature_help(sql, cursor, &cookie_routines()).unwrap();
    assert_eq!(sql.find("COALESCE").unwrap(), result.call_range.start);
    assert_eq!(sql.find(")").unwrap() + 1, result.call_range.end);
}

#[test]
fn cursor_at_end_within_open_call_works() {
    let sql = "SELECT COALESCE(a, ";
    let cursor = sql.len();
    let result = signature_help(sql, cursor, &cookie_routines()).unwrap();
    assert_eq!(1, result.active_parameter);
}

#[test]
fn cursor_after_unmatched_open_paren_in_non_call_context_returns_none() {
    // `(` 前不是函数名（例如分组括号），不应触发 signature。
    let sql = "SELECT (|a + b) FROM t";
    let cursor = sql.find('|').unwrap();
    let clean = sql.replace('|', "");
    let result = signature_help(&clean, cursor, &cookie_routines());
    assert!(result.is_none());
}

#[test]
fn signature_help_struct_is_comparable() {
    let a = SqlSignatureHelp {
        call_range: 7..18,
        active_parameter: 0,
        signatures: cookie_routines(),
    };
    let b = SqlSignatureHelp {
        call_range: 7..18,
        active_parameter: 0,
        signatures: cookie_routines(),
    };
    assert_eq!(a, b);
}
