use one_core::storage::DatabaseType;

use super::{ExpectedMapping, assert_expected, assert_unsupported};
use crate::compare::type_mapping::TypeCompatibility;

#[test]
fn maps_bit_strings_when_target_ddl_is_safe() {
    let cases = [
        ExpectedMapping {
            source_type: "BIT(64)",
            source_database: DatabaseType::PostgreSQL,
            target_database: DatabaseType::MySQL,
            target_type: "BIT(64)",
            compatibility: TypeCompatibility::Equivalent,
        },
        ExpectedMapping {
            source_type: "BIT(10485760)",
            source_database: DatabaseType::MySQL,
            target_database: DatabaseType::PostgreSQL,
            target_type: "BIT(10485760)",
            compatibility: TypeCompatibility::Equivalent,
        },
        ExpectedMapping {
            source_type: "VARBIT(8)",
            source_database: DatabaseType::DuckDB,
            target_database: DatabaseType::PostgreSQL,
            target_type: "BIT VARYING(8)",
            compatibility: TypeCompatibility::Equivalent,
        },
    ];
    cases.into_iter().for_each(assert_expected);
}

#[test]
fn rejects_bit_strings_when_target_ddl_is_unsafe() {
    for (source_type, source_database, target_database) in [
        ("BIT(0)", DatabaseType::PostgreSQL, DatabaseType::MySQL),
        ("BIT(65)", DatabaseType::PostgreSQL, DatabaseType::MySQL),
        (
            "BIT(10485761)",
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
        ),
        ("VARBIT(8)", DatabaseType::PostgreSQL, DatabaseType::MySQL),
    ] {
        assert_unsupported(source_type, source_database, target_database);
    }
}
