//! TSV 编解码测试（issue #355：复制整行后粘贴丢字段）

use super::*;

fn round_trip(rows: &[Vec<&str>]) -> Vec<Vec<String>> {
    parse_tsv_rows(&encode_tsv_rows(rows.iter().map(|row| row.iter().copied())))
}

#[test]
fn plain_values_are_not_escaped() {
    assert_eq!(
        "1\talice\tbeijing",
        encode_tsv_row(["1", "alice", "beijing"])
    );
    assert_eq!(
        "1\talice\n2\tbob",
        encode_tsv_rows([["1", "alice"], ["2", "bob"]])
    );
}

#[test]
fn round_trip_keeps_embedded_separators() {
    let rows = vec![
        vec!["1", "a\tb", "c"],
        vec!["2", "line1\nline2", "tail"],
        vec!["3", "cr\rcr", "tail"],
        vec!["4", "say \"hi\"", "tail"],
    ];

    assert_eq!(rows.clone(), round_trip(&rows));
}

#[test]
fn round_trip_keeps_empty_null_marker_and_quotes() {
    let rows = vec![vec!["", "\\N", ""], vec!["\"quoted\"", "a\"\"b", "\"\"\""]];

    assert_eq!(rows.clone(), round_trip(&rows));
}

#[test]
fn escapes_only_when_needed() {
    assert_eq!("plain", escape_tsv_field("plain"));
    assert_eq!("\"a\tb\"", escape_tsv_field("a\tb"));
    assert_eq!("\"a\nb\"", escape_tsv_field("a\nb"));
    assert_eq!("\"a\rb\"", escape_tsv_field("a\rb"));
    assert_eq!("\"\"\"quoted\"\"\"", escape_tsv_field("\"quoted\""));
}

#[test]
fn single_value_without_separators_stays_verbatim() {
    assert_eq!(vec![vec!["\"json 片段\""]], parse_tsv_rows("\"json 片段\""));
    assert_eq!(vec![vec![r"C:\new\test"]], parse_tsv_rows(r"C:\new\test"));
    assert!(parse_tsv_rows("").is_empty());
}

#[test]
fn unquoted_tab_is_still_a_separator() {
    // 外部复制来的裸制表符依然是分列符（旧行为不变）。
    assert_eq!(
        vec![vec!["a".to_string(), "b".to_string()]],
        parse_tsv_rows("a\tb")
    );
}

#[test]
fn excel_style_quoted_fields_are_understood() {
    // Excel 复制含换行/制表符的单元格时也是这套引号规则。
    assert_eq!(
        vec![vec!["a\tb".to_string(), "c".to_string()]],
        parse_tsv_rows("\"a\tb\"\tc")
    );
    assert_eq!(
        vec![vec!["line1\nline2".to_string(), "tail".to_string()]],
        parse_tsv_rows("\"line1\nline2\"\ttail")
    );
    assert_eq!(
        vec![vec!["say \"hi\"".to_string()]],
        parse_tsv_rows("\"say \"\"hi\"\"\"")
    );
}

#[test]
fn line_splitting_matches_previous_lines_behaviour() {
    // `\r\n` 算一个换行，末尾换行不额外产生空行，空行仍是一个空字段。
    assert_eq!(
        vec![vec!["a".to_string()], vec!["b".to_string()]],
        parse_tsv_rows("a\r\nb\n")
    );
    assert_eq!(
        vec![
            vec!["a".to_string()],
            vec!["".to_string()],
            vec!["b".to_string()]
        ],
        parse_tsv_rows("a\n\nb")
    );
    assert_eq!(vec![vec!["".to_string()]], parse_tsv_rows("\n"));
    assert_eq!(
        vec![vec!["a".to_string(), "b".to_string()]],
        parse_tsv_rows("a\tb")
    );
}

#[test]
fn ragged_rows_and_trailing_tab_are_preserved() {
    assert_eq!(
        vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string()]
        ],
        parse_tsv_rows("a\tb\nc")
    );
    assert_eq!(
        vec![vec!["a".to_string(), "".to_string()]],
        parse_tsv_rows("a\t")
    );
}

/// #355 的修复必须留在表格自己的复制/粘贴链路上：两侧都走本模块，不能又退回
/// `lines()` + `split('\t')`。`include_str!` 会把本测试文件一起读进来，所以只
/// 断言实现区（state.rs）的形态。
#[test]
fn clipboard_sites_use_the_shared_codec() {
    let state = include_str!("state.rs").replace("\r\n", "\n");

    assert!(
        state.contains("encode_tsv_rows"),
        "Ctrl/Cmd+C 必须经 encode_tsv_rows 转义后写剪贴板"
    );
    assert!(
        state.contains("parse_tsv_rows"),
        "Ctrl/Cmd+V 必须经 parse_tsv_rows 解转义"
    );
    assert!(!state.contains(r"split('\t')"), "粘贴侧不能再按制表符裸切");
}
