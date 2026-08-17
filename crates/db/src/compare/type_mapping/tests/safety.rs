mod arguments;
mod bit_string;
mod character;

use one_core::storage::DatabaseType;

use super::super::{TypeCompatibility, map_column_type};

pub(super) struct ExpectedMapping {
    pub(super) source_type: &'static str,
    pub(super) source_database: DatabaseType,
    pub(super) target_database: DatabaseType,
    pub(super) target_type: &'static str,
    pub(super) compatibility: TypeCompatibility,
}

pub(super) fn expected(
    source_type: &'static str,
    target_type: &'static str,
    compatibility: TypeCompatibility,
) -> ExpectedMapping {
    ExpectedMapping {
        source_type,
        source_database: DatabaseType::PostgreSQL,
        target_database: DatabaseType::MySQL,
        target_type,
        compatibility,
    }
}

pub(super) fn assert_character_mapping(mut expected: ExpectedMapping, target: DatabaseType) {
    expected.source_database = match &target {
        DatabaseType::PostgreSQL => DatabaseType::MySQL,
        _ => DatabaseType::PostgreSQL,
    };
    expected.target_database = target;
    assert_expected(expected);
}

pub(super) fn assert_expected(expected: ExpectedMapping) {
    let mapped = map_column_type(
        expected.source_type,
        &expected.source_database,
        &expected.target_database,
    );
    assert_eq!(expected.target_type, mapped.target_type);
    assert_eq!(expected.compatibility, mapped.compatibility);
}

pub(super) fn assert_unsupported(
    source_type: &str,
    source_database: DatabaseType,
    target_database: DatabaseType,
) {
    let mapped = map_column_type(source_type, &source_database, &target_database);
    assert_eq!(
        TypeCompatibility::Unsupported,
        mapped.compatibility,
        "{source_type} should not emit target DDL"
    );
    assert!(mapped.warning.is_some());
}
