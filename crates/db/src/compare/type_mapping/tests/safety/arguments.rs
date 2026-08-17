use one_core::storage::DatabaseType;

use super::{ExpectedMapping, assert_expected, assert_unsupported};
use crate::compare::type_mapping::TypeCompatibility;

const INVALID_NUMERIC_ARGUMENTS: [(&str, DatabaseType, DatabaseType); 20] = [
    (
        "DECIMAL(not-a-number)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "DECIMAL(4294967296)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "DECIMAL(10,not-a-number)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "DECIMAL(10,4294967296)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "DECIMAL(5,6)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "TIMESTAMP(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "TIMESTAMP(4294967296)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "TIME(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "DATETIME(not-a-number)",
        DatabaseType::MSSQL,
        DatabaseType::MySQL,
    ),
    (
        "BINARY(not-a-number)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "BINARY(4294967296)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "BIT(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "BIT(4294967296)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "FLOAT(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "FLOAT(4294967296)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "FLOAT(4294967295)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "REAL(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "DOUBLE(not-a-number)",
        DatabaseType::PostgreSQL,
        DatabaseType::MySQL,
    ),
    (
        "INT(not-a-number)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
    (
        "TINYINT(4294967296)",
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ),
];

#[test]
fn rejects_invalid_or_overflowing_numeric_arguments() {
    for (source_type, source_database, target_database) in INVALID_NUMERIC_ARGUMENTS {
        assert_unsupported(source_type, source_database, target_database);
    }
}

#[test]
fn preserves_supported_special_arguments() {
    for expected in [
        ExpectedMapping {
            source_type: "VARBINARY(MAX)",
            source_database: DatabaseType::MSSQL,
            target_database: DatabaseType::PostgreSQL,
            target_type: "BYTEA",
            compatibility: TypeCompatibility::Equivalent,
        },
        ExpectedMapping {
            source_type: "DATETIME64(3, 'UTC')",
            source_database: DatabaseType::ClickHouse,
            target_database: DatabaseType::PostgreSQL,
            target_type: "TIMESTAMPTZ(3)",
            compatibility: TypeCompatibility::Equivalent,
        },
    ] {
        assert_expected(expected);
    }
}

#[test]
fn rejects_invalid_clickhouse_datetime_arguments() {
    for source_type in [
        "DATETIME(123)",
        "DATETIME(foo)",
        "DATETIME64",
        "DATETIME64(10)",
        "DATETIME64(3, foo)",
        "DATETIME64(3, 123)",
    ] {
        assert_unsupported(
            source_type,
            DatabaseType::ClickHouse,
            DatabaseType::PostgreSQL,
        );
    }
}
