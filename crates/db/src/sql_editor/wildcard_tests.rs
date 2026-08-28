use std::collections::HashMap;

use super::wildcard::{
    SqlObjectRef, SqlWildcardExpansion, SqlWildcardQualifier, WildcardExpansionError,
    apply_wildcard_expansion, expand_multi_table_wildcard, expand_wildcard,
};

struct FakeMetadata {
    tables: HashMap<String, Vec<String>>,
}

impl FakeMetadata {
    fn new(tables: &[(&str, &[&str])]) -> Self {
        let mut map = HashMap::new();
        for (name, columns) in tables {
            map.insert(
                name.to_string(),
                columns.iter().map(|s| s.to_string()).collect(),
            );
        }
        Self { tables: map }
    }
}

impl super::wildcard::SqlWildcardMetadata for FakeMetadata {
    fn columns(&self, object: &SqlObjectRef) -> Option<Vec<String>> {
        self.tables.get(&object.name).cloned()
    }
}

fn star_offset(sql: &str) -> usize {
    sql.find('*').unwrap()
}

#[test]
fn single_table_select_star_expands_to_columns() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name", "email"])]);
    let sql = "SELECT * FROM users";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id, name, email", expansion.replacement);
    assert_eq!(star_offset(sql), expansion.range.start_byte);
    assert_eq!(star_offset(sql) + 1, expansion.range.end_byte);
    assert_eq!(1, expansion.required_tables.len());
}

#[test]
fn alias_qualifier_expands_only_that_table() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "SELECT u.* FROM users u";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id, name", expansion.replacement);
}

#[test]
fn alias_qualifier_with_always_uses_prefix() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "SELECT u.* FROM users u";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::Always).unwrap();
    assert_eq!("u.id, u.name", expansion.replacement);
}

#[test]
fn two_table_join_wildcard_is_ambiguous_without_qualifier() {
    let metadata = FakeMetadata::new(&[
        ("users", &["id", "name"]),
        ("orders", &["id", "user_id"]),
    ]);
    let sql = "SELECT * FROM users u JOIN orders o ON u.id = o.user_id";
    let result = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None);
    assert_eq!(Err(WildcardExpansionError::AmbiguousSource), result);
}

#[test]
fn multi_table_wildcard_expands_in_source_order() {
    let _metadata = FakeMetadata::new(&[
        ("users", &["id", "name"]),
        ("orders", &["id", "user_id", "total"]),
    ]);
    let sql = "SELECT * FROM users u JOIN orders o ON u.id = o.user_id";
    let sources = vec![
        SqlObjectRef {
            database: None,
            schema: None,
            name: "users".to_string(),
        },
        SqlObjectRef {
            database: None,
            schema: None,
            name: "orders".to_string(),
        },
    ];
    let columns = vec![
        vec!["id".to_string(), "name".to_string()],
        vec!["id".to_string(), "user_id".to_string(), "total".to_string()],
    ];
    let expansion = expand_multi_table_wildcard(
        sql,
        0,
        &sources,
        &columns,
        SqlWildcardQualifier::OnConflict,
    )
    .unwrap();
    // id 冲突 -> 加前缀；name / user_id / total 不加。
    assert_eq!(
        "users.id, name, orders.id, user_id, total",
        expansion.replacement
    );
}

#[test]
fn join_wildcard_with_alias_qualifier_uses_qualifier() {
    let metadata = FakeMetadata::new(&[
        ("users", &["id", "name"]),
        ("orders", &["id", "user_id"]),
    ]);
    let sql = "SELECT o.* FROM users u JOIN orders o ON u.id = o.user_id";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id, user_id", expansion.replacement);
}

#[test]
fn duplicate_columns_are_qualified_on_conflict() {
    let metadata = FakeMetadata::new(&[("users", &["id", "id"])]);
    let sql = "SELECT u.* FROM users u";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::OnConflict).unwrap();
    assert_eq!("u.id, u.id", expansion.replacement);
}

#[test]
fn metadata_incomplete_fails_closed() {
    let metadata = FakeMetadata::new(&[]);
    let sql = "SELECT * FROM users";
    let result = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None);
    assert_eq!(Err(WildcardExpansionError::MetadataIncomplete), result);
}

#[test]
fn no_wildcard_returns_no_wildcard_error() {
    let metadata = FakeMetadata::new(&[("users", &["id"])]);
    let sql = "SELECT id FROM users";
    let result = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None);
    assert_eq!(Err(WildcardExpansionError::NoWildcard), result);
}

#[test]
fn cte_projection_expands() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "WITH recent AS (SELECT id, name FROM users) SELECT * FROM recent";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id, name", expansion.replacement);
}

#[test]
fn cte_with_alias_qualifier_expands() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "WITH recent AS (SELECT id, name FROM users) SELECT r.* FROM recent r";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id, name", expansion.replacement);
}

#[test]
fn quoted_identifiers_are_preserved() {
    let metadata = FakeMetadata::new(&[("users", &["id", "first name"])]);
    let sql = "SELECT u.* FROM users u";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::Always).unwrap();
    assert_eq!("u.id, u.\"first name\"", expansion.replacement);
}

#[test]
fn base_byte_shifts_output_range() {
    let metadata = FakeMetadata::new(&[("users", &["id"])]);
    let sql = "SELECT * FROM users";
    let expansion = expand_wildcard(sql, 100, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!(100 + star_offset(sql), expansion.range.start_byte);
}

#[test]
fn stale_apply_is_rejected() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "SELECT * FROM users";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    // 文档内容变化（不再是 `*`）。
    let changed = "SELECT name FROM users";
    let result = apply_wildcard_expansion(changed, &expansion, "*");
    assert_eq!(Err(WildcardExpansionError::StaleSource), result);
}

#[test]
fn apply_with_matching_range_replaces() {
    let metadata = FakeMetadata::new(&[("users", &["id", "name"])]);
    let sql = "SELECT * FROM users";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    let result = apply_wildcard_expansion(sql, &expansion, "*").unwrap();
    assert_eq!("SELECT id, name FROM users", result);
}

#[test]
fn qualified_star_range_covers_only_star() {
    let metadata = FakeMetadata::new(&[("users", &["id"])]);
    let sql = "SELECT u.* FROM users u";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!(sql.find('*').unwrap(), expansion.range.start_byte);
    assert_eq!(sql.find('*').unwrap() + 1, expansion.range.end_byte);
}

#[test]
fn qualified_table_name_without_alias_resolves() {
    let metadata = FakeMetadata::new(&[("users", &["id"])]);
    let sql = "SELECT users.* FROM users";
    let expansion = expand_wildcard(sql, 0, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!("id", expansion.replacement);
}

#[test]
fn expansion_struct_carries_range_replacement_and_tables() {
    let metadata = FakeMetadata::new(&[("users", &["id"])]);
    let sql = "SELECT * FROM users";
    let expansion = expand_wildcard(sql, 5, &metadata, SqlWildcardQualifier::None).unwrap();
    assert_eq!(
        SqlWildcardExpansion {
            range: super::wildcard::SqlTextRange {
                start_byte: 5 + sql.find('*').unwrap(),
                end_byte: 5 + sql.find('*').unwrap() + 1,
            },
            replacement: "id".to_string(),
            required_tables: vec![SqlObjectRef {
                database: None,
                schema: None,
                name: "users".to_string(),
            }],
        },
        expansion
    );
}

