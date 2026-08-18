use one_core::storage::DatabaseType;

use super::{
    SchemaTypeMappingContext, TypeCompatibility, column_types_equivalent, map_column_type,
};

mod safety;

struct MappingCase {
    source_type: &'static str,
    source_database: DatabaseType,
    target_database: DatabaseType,
    target_type: &'static str,
    compatibility: TypeCompatibility,
}

#[test]
fn maps_common_cross_database_types() {
    [
        MappingCase {
            source_type: "INT",
            source_database: DatabaseType::MySQL,
            target_database: DatabaseType::PostgreSQL,
            target_type: "INTEGER",
            compatibility: TypeCompatibility::Equivalent,
        },
        MappingCase {
            source_type: "BIGINT UNSIGNED",
            source_database: DatabaseType::MySQL,
            target_database: DatabaseType::PostgreSQL,
            target_type: "NUMERIC(20,0)",
            compatibility: TypeCompatibility::Widening,
        },
        MappingCase {
            source_type: "BYTEA",
            source_database: DatabaseType::PostgreSQL,
            target_database: DatabaseType::MySQL,
            target_type: "LONGBLOB",
            compatibility: TypeCompatibility::Widening,
        },
        MappingCase {
            source_type: "UUID",
            source_database: DatabaseType::PostgreSQL,
            target_database: DatabaseType::MSSQL,
            target_type: "UNIQUEIDENTIFIER",
            compatibility: TypeCompatibility::Equivalent,
        },
        MappingCase {
            source_type: "JSON",
            source_database: DatabaseType::MySQL,
            target_database: DatabaseType::PostgreSQL,
            target_type: "JSONB",
            compatibility: TypeCompatibility::Equivalent,
        },
    ]
    .into_iter()
    .for_each(assert_mapping);
}

#[test]
fn compares_mapped_types_without_hiding_size_changes() {
    let mysql_to_postgres =
        SchemaTypeMappingContext::new(&DatabaseType::MySQL, &DatabaseType::PostgreSQL);
    assert!(column_types_equivalent(
        "INT",
        "INTEGER",
        mysql_to_postgres.clone()
    ));
    assert!(column_types_equivalent(
        "DECIMAL(10,2)",
        "NUMERIC(10,2)",
        mysql_to_postgres.clone()
    ));
    assert!(column_types_equivalent(
        "VARCHAR(255)",
        "CHARACTER VARYING(255)",
        mysql_to_postgres.clone()
    ));
    assert!(!column_types_equivalent(
        "DECIMAL(10,2)",
        "NUMERIC(12,2)",
        mysql_to_postgres
    ));

    let postgres_to_mysql =
        SchemaTypeMappingContext::new(&DatabaseType::PostgreSQL, &DatabaseType::MySQL);
    assert!(column_types_equivalent(
        "BYTEA",
        "LONGBLOB",
        postgres_to_mysql.clone()
    ));
    assert!(!column_types_equivalent("BYTEA", "BLOB", postgres_to_mysql));
}

#[test]
fn rejects_lossy_or_semantically_different_temporal_and_bit_types() {
    let mapped = map_column_type(
        "TIMESTAMPTZ",
        &DatabaseType::PostgreSQL,
        &DatabaseType::MySQL,
    );
    assert_eq!("DATETIME", mapped.target_type);
    assert_eq!(TypeCompatibility::Lossy, mapped.compatibility);

    assert!(!column_types_equivalent(
        "TIMESTAMPTZ",
        "DATETIME",
        SchemaTypeMappingContext::new(&DatabaseType::PostgreSQL, &DatabaseType::MySQL),
    ));
    assert!(!column_types_equivalent(
        "TIMESTAMP",
        "TIMESTAMP",
        SchemaTypeMappingContext::new(&DatabaseType::MSSQL, &DatabaseType::PostgreSQL),
    ));
    assert!(!column_types_equivalent(
        "BIT(8)",
        "BIT",
        SchemaTypeMappingContext::new(&DatabaseType::MySQL, &DatabaseType::MSSQL),
    ));
}

#[test]
fn rejects_complex_and_unknown_external_types() {
    let complex = map_column_type(
        "Array(Int32)",
        &DatabaseType::ClickHouse,
        &DatabaseType::PostgreSQL,
    );
    assert_eq!(TypeCompatibility::Unsupported, complex.compatibility);

    let external = map_column_type(
        "INT",
        &DatabaseType::external("custom-driver"),
        &DatabaseType::PostgreSQL,
    );
    assert_eq!(TypeCompatibility::Unsupported, external.compatibility);
}

#[test]
fn rejects_invalid_character_lengths_instead_of_emitting_target_ddl() {
    for source_type in ["VARCHAR(0)", "VARCHAR(not-a-length)", "CHAR(MAX)"] {
        let mapped = map_column_type(source_type, &DatabaseType::PostgreSQL, &DatabaseType::MySQL);
        assert_eq!(TypeCompatibility::Unsupported, mapped.compatibility);
    }
}

#[test]
fn marks_national_and_character_byte_semantics_as_lossy() {
    let national_text = map_column_type(
        "NVARCHAR(MAX)",
        &DatabaseType::MSSQL,
        &DatabaseType::PostgreSQL,
    );
    assert_eq!("TEXT", national_text.target_type);
    assert_eq!(TypeCompatibility::Lossy, national_text.compatibility);
    assert!(
        national_text
            .warning
            .is_some_and(|warning| warning.contains("national"))
    );

    let fixed_character = map_column_type(
        "CHAR(16)",
        &DatabaseType::PostgreSQL,
        &DatabaseType::ClickHouse,
    );
    assert_eq!("FixedString(16)", fixed_character.target_type);
    assert_eq!(TypeCompatibility::Lossy, fixed_character.compatibility);
    assert!(
        fixed_character
            .warning
            .is_some_and(|warning| warning.contains("字节"))
    );
}

#[test]
fn rejects_unbounded_decimal_for_bounded_targets() {
    for target in [
        DatabaseType::MySQL,
        DatabaseType::MSSQL,
        DatabaseType::Oracle,
        DatabaseType::DuckDB,
        DatabaseType::ClickHouse,
    ] {
        let mapped = map_column_type("NUMERIC", &DatabaseType::PostgreSQL, &target);
        assert_eq!(TypeCompatibility::Unsupported, mapped.compatibility);
        assert!(
            mapped
                .warning
                .is_some_and(|warning| warning.contains("精度"))
        );
    }
}

#[test]
fn clamps_unsupported_temporal_precision_as_lossy() {
    let mysql = map_column_type(
        "TIMESTAMP(9)",
        &DatabaseType::PostgreSQL,
        &DatabaseType::MySQL,
    );
    assert_eq!("DATETIME(6)", mysql.target_type);
    assert_eq!(TypeCompatibility::Lossy, mysql.compatibility);
    assert!(
        mysql
            .warning
            .is_some_and(|warning| warning.contains("精度"))
    );

    let postgres = map_column_type(
        "TIMESTAMP(9)",
        &DatabaseType::Oracle,
        &DatabaseType::PostgreSQL,
    );
    assert_eq!("TIMESTAMP(6)", postgres.target_type);
    assert_eq!(TypeCompatibility::Lossy, postgres.compatibility);
}

#[test]
fn marks_sqlite_constraint_loss_as_lossy() {
    for (source_type, source_database) in [
        ("DECIMAL(10,2)", DatabaseType::PostgreSQL),
        ("VARCHAR(32)", DatabaseType::PostgreSQL),
        ("BINARY(16)", DatabaseType::MySQL),
        ("JSON", DatabaseType::PostgreSQL),
    ] {
        let mapped = map_column_type(source_type, &source_database, &DatabaseType::SQLite);
        assert_eq!(TypeCompatibility::Lossy, mapped.compatibility);
        assert!(mapped.warning.is_some());
    }
}

#[test]
fn marks_binary_constraint_loss_and_uses_safe_mysql_blob_capacity() {
    let constrained = map_column_type(
        "BINARY(16)",
        &DatabaseType::MySQL,
        &DatabaseType::PostgreSQL,
    );
    assert_eq!("BYTEA", constrained.target_type);
    assert_eq!(TypeCompatibility::Lossy, constrained.compatibility);

    let unbounded = map_column_type("BYTEA", &DatabaseType::PostgreSQL, &DatabaseType::MySQL);
    assert_eq!("LONGBLOB", unbounded.target_type);
    assert_eq!(TypeCompatibility::Widening, unbounded.compatibility);
}

#[test]
fn unknown_external_families_only_compare_with_the_same_driver() {
    let first = DatabaseType::external("unknown-a");
    let same = DatabaseType::external("unknown-a");
    let different = DatabaseType::external("unknown-b");

    assert!(column_types_equivalent(
        "CUSTOM(10)",
        "custom(10)",
        SchemaTypeMappingContext::new(&first, &same),
    ));
    assert!(!column_types_equivalent(
        "CUSTOM(10)",
        "CUSTOM(10)",
        SchemaTypeMappingContext::new(&first, &different),
    ));
    let mapped = map_column_type("CUSTOM(10)", &first, &different);
    assert_eq!(TypeCompatibility::Unsupported, mapped.compatibility);
}

#[test]
fn known_external_families_are_mapped_between_concrete_drivers() {
    let mariadb = DatabaseType::external("mariadb");
    let mapped = map_column_type("INT(11) UNSIGNED", &mariadb, &DatabaseType::MySQL);
    assert_eq!("INT UNSIGNED", mapped.target_type);
    assert_eq!(TypeCompatibility::Equivalent, mapped.compatibility);

    let context = SchemaTypeMappingContext::new(&mariadb, &DatabaseType::MySQL);
    assert!(column_types_equivalent("INT(11)", "INT", context.clone()));
    assert!(!column_types_equivalent(
        "ENUM('draft','published')",
        "ENUM('draft','published')",
        context,
    ));
    let unsupported = map_column_type("ENUM('draft','published')", &mariadb, &DatabaseType::MySQL);
    assert_eq!(TypeCompatibility::Unsupported, unsupported.compatibility);
}

fn assert_mapping(case: MappingCase) {
    let mapped = map_column_type(
        case.source_type,
        &case.source_database,
        &case.target_database,
    );
    assert_eq!(case.target_type, mapped.target_type);
    assert_eq!(case.compatibility, mapped.compatibility);
}
