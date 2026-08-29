use super::insert_hints::{SqlInsertValueHint, insert_value_hints};

fn hints(sql: &str, columns: &[&str]) -> Vec<SqlInsertValueHint> {
    let columns: Vec<String> = columns.iter().map(|s| s.to_string()).collect();
    insert_value_hints(sql, &columns)
}

#[test]
fn explicit_columns_map_to_value_slots() {
    let result = hints(
        "INSERT INTO users (name, age, active) VALUES ('a', 1, true)",
        &[],
    );
    assert_eq!(3, result.len());
    assert_eq!("name", result[0].column);
    assert_eq!(0, result[0].row_index);
    assert_eq!("age", result[1].column);
    assert_eq!("active", result[2].column);
}

#[test]
fn multi_row_values_advance_row_index() {
    let result = hints(
        "INSERT INTO users (a, b) VALUES (1, 2), (3, 4), (5, 6)",
        &[],
    );
    assert_eq!(6, result.len());
    assert_eq!(0, result[0].row_index);
    assert_eq!(1, result[2].row_index);
    assert_eq!(2, result[4].row_index);
    assert_eq!("a", result[2].column);
    assert_eq!("b", result[3].column);
}

#[test]
fn nested_expressions_do_not_count_as_slots() {
    let result = hints("INSERT INTO t (a, b) VALUES (1, COALESCE(2, 3))", &[]);
    assert_eq!(2, result.len());
    assert_eq!("a", result[0].column);
    assert_eq!("b", result[1].column);
}

#[test]
fn no_explicit_columns_uses_ordinal() {
    let result = hints("INSERT INTO t VALUES (1, 2, 3)", &["c1", "c2", "c3"]);
    assert_eq!(3, result.len());
    assert_eq!("c1", result[0].column);
    assert_eq!("c2", result[1].column);
    assert_eq!("c3", result[2].column);
}

#[test]
fn missing_ordinals_fall_back_to_column_n() {
    let result = hints("INSERT INTO t VALUES (1, 2)", &[]);
    assert_eq!(2, result.len());
    assert_eq!("column_1", result[0].column);
    assert_eq!("column_2", result[1].column);
}

#[test]
fn offset_points_before_each_value() {
    let sql = "INSERT INTO t (a, b) VALUES (1, 2)";
    let result = hints(sql, &[]);
    assert_eq!(2, result.len());
    let first_value = sql.find("(1,").unwrap() + 1;
    assert_eq!(first_value, result[0].offset);
}

#[test]
fn quoted_identifiers_are_unquoted_in_columns() {
    let result = hints(
        "INSERT INTO \"users\" (\"first name\", age) VALUES (1, 2)",
        &[],
    );
    assert_eq!(2, result.len());
    assert_eq!("first name", result[0].column);
}

#[test]
fn offset_is_byte_based_with_multibyte_string_values() {
    let sql = "INSERT INTO t (a) VALUES ('中文')";
    let result = hints(sql, &[]);
    assert_eq!(1, result.len());
    let expected = sql.find("('").unwrap() + 1;
    assert_eq!(expected, result[0].offset);
}

#[test]
fn insert_select_without_values_produces_no_hints() {
    let result = hints("INSERT INTO t (a, b) SELECT x, y FROM src", &[]);
    assert!(result.is_empty());
}

#[test]
fn non_insert_statement_produces_no_hints() {
    let result = hints("SELECT 1", &[]);
    assert!(result.is_empty());
}

#[test]
fn function_call_within_value_is_not_a_nested_row() {
    let result = hints("INSERT INTO t (a) VALUES (f(1, 2), g(3))", &[]);
    assert_eq!(2, result.len());
}

#[test]
fn string_literal_with_comma_is_not_a_slot_separator() {
    let result = hints("INSERT INTO t (a, b) VALUES ('x,y', 'z')", &[]);
    assert_eq!(2, result.len());
    assert_eq!("a", result[0].column);
    assert_eq!("b", result[1].column);
}
