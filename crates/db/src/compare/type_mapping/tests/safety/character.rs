use one_core::storage::DatabaseType;

use super::{ExpectedMapping, assert_character_mapping, assert_expected, expected};
use crate::compare::type_mapping::TypeCompatibility;

#[test]
fn maps_mysql_character_lengths_at_safe_boundaries() {
    for expected in [
        expected("CHAR(255)", "CHAR(255)", TypeCompatibility::Equivalent),
        expected("CHAR(256)", "LONGTEXT", TypeCompatibility::Lossy),
        expected(
            "VARCHAR(16383)",
            "VARCHAR(16383)",
            TypeCompatibility::Equivalent,
        ),
        expected("VARCHAR(16384)", "LONGTEXT", TypeCompatibility::Widening),
    ] {
        assert_character_mapping(expected, DatabaseType::MySQL);
    }
}

#[test]
fn maps_postgres_character_lengths_at_safe_boundaries() {
    for expected in [
        expected(
            "VARCHAR(10485760)",
            "VARCHAR(10485760)",
            TypeCompatibility::Equivalent,
        ),
        expected("VARCHAR(10485761)", "TEXT", TypeCompatibility::Widening),
        expected("CHAR(10485761)", "TEXT", TypeCompatibility::Lossy),
    ] {
        assert_character_mapping(expected, DatabaseType::PostgreSQL);
    }
}

#[test]
fn maps_sql_server_character_fallbacks() {
    for expected in [
        sql_server_case(
            "VARCHAR(8000)",
            "VARCHAR(8000)",
            TypeCompatibility::Equivalent,
        ),
        sql_server_case("VARCHAR(8001)", "VARCHAR(MAX)", TypeCompatibility::Widening),
        ExpectedMapping {
            source_type: "NCHAR(4001)",
            source_database: DatabaseType::Oracle,
            target_database: DatabaseType::MSSQL,
            target_type: "NVARCHAR(MAX)",
            compatibility: TypeCompatibility::Lossy,
        },
    ] {
        assert_expected(expected);
    }
}

#[test]
fn maps_oracle_character_fallbacks() {
    for expected in [
        oracle_case(
            "VARCHAR(4000)",
            "VARCHAR2(4000)",
            TypeCompatibility::Equivalent,
        ),
        oracle_case("VARCHAR(4001)", "CLOB", TypeCompatibility::Widening),
        oracle_case("CHAR(2001)", "CLOB", TypeCompatibility::Lossy),
        ExpectedMapping {
            source_type: "NVARCHAR(2001)",
            source_database: DatabaseType::MSSQL,
            target_database: DatabaseType::Oracle,
            target_type: "NCLOB",
            compatibility: TypeCompatibility::Widening,
        },
    ] {
        assert_expected(expected);
    }
}

fn sql_server_case(
    source_type: &'static str,
    target_type: &'static str,
    compatibility: TypeCompatibility,
) -> ExpectedMapping {
    ExpectedMapping {
        source_type,
        source_database: DatabaseType::PostgreSQL,
        target_database: DatabaseType::MSSQL,
        target_type,
        compatibility,
    }
}

fn oracle_case(
    source_type: &'static str,
    target_type: &'static str,
    compatibility: TypeCompatibility,
) -> ExpectedMapping {
    ExpectedMapping {
        source_type,
        source_database: DatabaseType::PostgreSQL,
        target_database: DatabaseType::Oracle,
        target_type,
        compatibility,
    }
}
