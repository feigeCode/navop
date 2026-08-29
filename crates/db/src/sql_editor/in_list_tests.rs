use super::in_list::{InListParseError, SqlInListValue, build_in_list_clause, parse_in_list};

#[test]
fn parses_comma_separated_values() {
    let values = parse_in_list("1, 2, 3").unwrap();
    assert_eq!(
        vec![
            SqlInListValue::Raw("1".to_string()),
            SqlInListValue::Raw("2".to_string()),
            SqlInListValue::Raw("3".to_string()),
        ],
        values
    );
}

#[test]
fn parses_newline_separated_values() {
    let values = parse_in_list("apple\nbanana\ncherry").unwrap();
    assert_eq!(3, values.len());
    assert_eq!(SqlInListValue::String("apple".to_string()), values[0]);
}

#[test]
fn parses_tab_separated_values() {
    let values = parse_in_list("10\t20\t30").unwrap();
    assert_eq!(3, values.len());
    assert!(matches!(values[0], SqlInListValue::Raw(_)));
}

#[test]
fn parses_semicolon_separated_values() {
    let values = parse_in_list("a; b; c").unwrap();
    assert_eq!(3, values.len());
}

#[test]
fn handles_mixed_and_trailing_separators() {
    let values = parse_in_list("1, 2,\n3,\n").unwrap();
    assert_eq!(3, values.len());
}

#[test]
fn null_and_numbers_are_raw() {
    let values = parse_in_list("NULL, 42, 3.14").unwrap();
    assert_eq!(SqlInListValue::Raw("NULL".to_string()), values[0]);
    assert_eq!(SqlInListValue::Raw("42".to_string()), values[1]);
    assert_eq!(SqlInListValue::Raw("3.14".to_string()), values[2]);
}

#[test]
fn strings_are_escaped() {
    let values = parse_in_list("O'Reilly").unwrap();
    assert_eq!(SqlInListValue::String("O'Reilly".to_string()), values[0]);
    assert_eq!("'O''Reilly'", values[0].to_sql_fragment());
}

#[test]
fn build_clause_joins_with_commas() {
    let values = parse_in_list("1\n2\n3").unwrap();
    assert_eq!("1, 2, 3", build_in_list_clause(&values));
}

#[test]
fn empty_input_is_rejected() {
    assert_eq!(Err(InListParseError::Empty), parse_in_list(""));
    assert_eq!(Err(InListParseError::Empty), parse_in_list("   "));
    assert_eq!(Err(InListParseError::Empty), parse_in_list(",\n,"));
}

#[test]
fn over_size_limit_is_rejected() {
    let big = "x".repeat(1024 * 1024 + 1);
    assert_eq!(Err(InListParseError::TooLarge), parse_in_list(&big));
}

#[test]
fn over_value_count_is_rejected() {
    let text = (0..10_001)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(Err(InListParseError::TooManyValues), parse_in_list(&text));
}

#[test]
fn single_plain_text_is_rejected() {
    assert_eq!(
        Err(InListParseError::SinglePlainText),
        parse_in_list("the quick brown fox jumps over the lazy dog")
    );
}

#[test]
fn url_and_path_like_single_text_is_rejected() {
    assert_eq!(
        Err(InListParseError::SinglePlainText),
        parse_in_list("https://example.com/path")
    );
    assert_eq!(
        Err(InListParseError::SinglePlainText),
        parse_in_list("/Users/me/file.txt")
    );
}

#[test]
fn single_number_is_accepted() {
    let values = parse_in_list("42").unwrap();
    assert_eq!(vec![SqlInListValue::Raw("42".to_string())], values);
}

#[test]
fn single_null_is_accepted() {
    let values = parse_in_list("NULL").unwrap();
    assert_eq!(vec![SqlInListValue::Raw("NULL".to_string())], values);
}

#[test]
fn boundary_of_ten_thousand_is_accepted() {
    let text = (0..10_000)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let values = parse_in_list(&text).unwrap();
    assert_eq!(10_000, values.len());
}

#[test]
fn sql_fragment_escapes_single_quotes() {
    let value = SqlInListValue::String("it's".to_string());
    assert_eq!("'it''s'", value.to_sql_fragment());
    let raw = SqlInListValue::Raw("42".to_string());
    assert_eq!("42", raw.to_sql_fragment());
}
