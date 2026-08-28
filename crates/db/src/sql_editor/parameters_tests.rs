use one_core::storage::DatabaseType;

use super::parameters::{
    SqlParameterBinding, SqlParameterKind, SqlParameterOccurrence, SqlParameterValueType,
    build_parameter_literal, collect_parameters, escape_string_literal, parameter_descriptors,
    preview_sql, substitute_parameters,
};

#[test]
fn collects_anonymous_question_mark_parameters() {
    let occurrences = collect_parameters("SELECT * FROM users WHERE id = ? AND age > ?");
    assert_eq!(2, occurrences.len());
    assert_eq!(SqlParameterKind::QuestionMark, occurrences[0].kind);
    assert_eq!(None, occurrences[0].name);
    assert_eq!(None, occurrences[1].name);
    assert!(occurrences[0].start < occurrences[1].start);
}

#[test]
fn collects_named_colon_parameters() {
    let occurrences = collect_parameters("SELECT * FROM users WHERE id = :user_id");
    assert_eq!(1, occurrences.len());
    assert_eq!(SqlParameterKind::Colon, occurrences[0].kind);
    assert_eq!(Some("user_id".to_string()), occurrences[0].name);
}

#[test]
fn collects_dollar_parameters_with_and_without_braces() {
    let sql = "SELECT * FROM users WHERE id = $id OR name = ${name}";
    let occurrences = collect_parameters(sql);
    assert_eq!(2, occurrences.len());
    assert_eq!(SqlParameterKind::Dollar, occurrences[0].kind);
    assert_eq!(Some("id".to_string()), occurrences[0].name);
    assert_eq!(SqlParameterKind::MyBatisDollar, occurrences[1].kind);
    assert_eq!(Some("name".to_string()), occurrences[1].name);
}

#[test]
fn collects_at_parameters_but_not_at_at_system_variables() {
    let sql = "SELECT @@version, @custom FROM users WHERE id = @uid";
    let occurrences = collect_parameters(sql);
    assert_eq!(2, occurrences.len());
    assert_eq!(SqlParameterKind::At, occurrences[0].kind);
    assert_eq!(Some("custom".to_string()), occurrences[0].name);
    assert_eq!(Some("uid".to_string()), occurrences[1].name);
}

#[test]
fn collects_mybatis_hash_parameters() {
    let occurrences = collect_parameters("SELECT * FROM users WHERE id = #{userId} AND age = #{age}");
    assert_eq!(2, occurrences.len());
    assert_eq!(SqlParameterKind::MyBatisHash, occurrences[0].kind);
    assert_eq!(Some("userId".to_string()), occurrences[0].name);
}

#[test]
fn ignores_parameters_inside_strings_comments_and_quoted_identifiers() {
    let sql = r#"
        -- WHERE id = ? -- comment
        /* WHERE id = :hidden */
        SELECT 'literal ? :notparam', "quoted:ident"
        FROM users
    "#;
    let occurrences = collect_parameters(sql);
    assert!(occurrences.is_empty());
}

#[test]
fn does_not_treat_cast_operator_as_parameter() {
    let occurrences = collect_parameters("SELECT value::text FROM t");
    assert!(occurrences.is_empty());
}

#[test]
fn parameters_inside_strings_are_not_detected() {
    let occurrences = collect_parameters("SELECT 'a?b:c' AS x");
    assert!(occurrences.is_empty());
}

#[test]
fn descriptors_group_duplicate_occurrences_by_name() {
    let sql = "SELECT * FROM t WHERE a = :id OR b = :id OR c = ?";
    let occurrences = collect_parameters(sql);
    let descriptors = parameter_descriptors(&occurrences);
    assert_eq!(2, descriptors.len());
    let id = descriptors.iter().find(|d| d.name == "id").unwrap();
    assert_eq!(2, id.occurrences);
    let anon = descriptors.iter().find(|d| d.name == "?").unwrap();
    assert_eq!(1, anon.occurrences);
}

#[test]
fn substitute_replaces_anonymous_and_named_parameters() {
    let sql = "SELECT * FROM t WHERE a = ? AND b = :name";
    let occurrences = collect_parameters(sql);
    let bindings: Vec<SqlParameterBinding> = occurrences
        .iter()
        .map(|o| SqlParameterBinding {
            occurrence: o.clone(),
            literal: if o.kind == SqlParameterKind::QuestionMark {
                "'42'".to_string()
            } else {
                "'x'".to_string()
            },
        })
        .collect();
    let substituted = substitute_parameters(sql, &bindings);
    assert_eq!("SELECT * FROM t WHERE a = '42' AND b = 'x'", substituted);
}

#[test]
fn substitute_handles_braced_dollar_parameters() {
    let sql = "SELECT * FROM t WHERE id = ${id}";
    let occurrences = collect_parameters(sql);
    let bindings: Vec<SqlParameterBinding> = occurrences
        .iter()
        .map(|o| SqlParameterBinding {
            occurrence: o.clone(),
            literal: "(1, 2, 3)".to_string(),
        })
        .collect();
    let substituted = substitute_parameters(sql, &bindings);
    assert_eq!("SELECT * FROM t WHERE id = (1, 2, 3)", substituted);
}

#[test]
fn preview_normalizes_parameter_forms() {
    let sql = "SELECT * FROM t WHERE a = ? AND b = :name AND c = $x AND d = #{y} AND e = ${z}";
    let occurrences = collect_parameters(sql);
    let preview = preview_sql(sql, &occurrences);
    assert_eq!(
        "SELECT * FROM t WHERE a = ? AND b = :name AND c = $(x) AND d = #{y} AND e = ${z}",
        preview
    );
}

#[test]
fn build_literal_escapes_strings_and_passes_numbers_through() {
    let db = DatabaseType::PostgreSQL;
    assert_eq!(
        "'O''Reilly'",
        build_parameter_literal(SqlParameterValueType::String, "O'Reilly", &db)
    );
    assert_eq!(
        "42",
        build_parameter_literal(SqlParameterValueType::Number, "42", &db)
    );
    assert_eq!(
        "TRUE",
        build_parameter_literal(SqlParameterValueType::Boolean, "true", &db)
    );
    assert_eq!(
        "NULL",
        build_parameter_literal(SqlParameterValueType::Null, "ignored", &db)
    );
    assert_eq!(
        "raw",
        build_parameter_literal(SqlParameterValueType::Raw, "raw", &db)
    );
}

#[test]
fn escape_string_literal_doubles_single_quotes() {
    assert_eq!("it''s", escape_string_literal("it's", &DatabaseType::PostgreSQL));
    assert_eq!(
        "a''b",
        escape_string_literal("a'b", &DatabaseType::SQLite)
    );
}

#[test]
fn escape_string_literal_escapes_backslashes_for_mysql() {
    assert_eq!(
        "a\\\\b",
        escape_string_literal("a\\b", &DatabaseType::MySQL)
    );
    assert_eq!(
        "a\\b",
        escape_string_literal("a\\b", &DatabaseType::PostgreSQL)
    );
}

#[test]
fn occurrence_offsets_are_byte_based_for_multibyte_text() {
    let sql = "SELECT '中文' AS x WHERE id = :p";
    let occurrences = collect_parameters(sql);
    assert_eq!(1, occurrences.len());
    let occurrence = &occurrences[0];
    assert_eq!(&sql[occurrence.start..occurrence.end], ":p");
}

#[test]
fn occurrence_identity_matches_kind_and_range() {
    let sql = "SELECT * FROM t WHERE a = ? AND b = ?";
    let occurrences = collect_parameters(sql);
    assert_ne!(occurrences[0], occurrences[1]);
    assert_eq!(
        SqlParameterOccurrence {
            kind: SqlParameterKind::QuestionMark,
            name: None,
            start: 0,
            end: 1,
        },
        SqlParameterOccurrence {
            kind: SqlParameterKind::QuestionMark,
            name: None,
            start: 0,
            end: 1,
        }
    );
}

#[test]
fn value_parameters_are_not_raw() {
    assert!(SqlParameterKind::QuestionMark.is_value());
    assert!(SqlParameterKind::Colon.is_value());
    assert!(SqlParameterKind::At.is_value());
    assert!(!SqlParameterKind::MyBatisDollar.is_value());
}
