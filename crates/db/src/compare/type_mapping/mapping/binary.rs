use super::{MappingTarget, format_length};
use crate::compare::type_mapping::{
    family::DatabaseFamily,
    model::{MappedColumnType, TypeCompatibility},
};

struct AffinityBinaryTarget<'a> {
    target_type: &'a str,
    database_name: &'a str,
}

pub(super) fn map_binary(
    fixed: bool,
    length: Option<u32>,
    target_family: DatabaseFamily,
) -> MappedColumnType {
    match target_family {
        DatabaseFamily::PostgreSql => postgres_binary(fixed, length),
        DatabaseFamily::MySql => mysql_binary(fixed, length),
        DatabaseFamily::SqlServer => sql_server_binary(fixed, length),
        DatabaseFamily::Oracle => oracle_binary(fixed, length),
        DatabaseFamily::Sqlite => affinity_binary(
            AffinityBinaryTarget {
                target_type: "BLOB",
                database_name: "SQLite",
            },
            fixed,
            length,
        ),
        DatabaseFamily::DuckDb => affinity_binary(
            AffinityBinaryTarget {
                target_type: "BLOB",
                database_name: "DuckDB",
            },
            fixed,
            length,
        ),
        DatabaseFamily::ClickHouse => clickhouse_binary(fixed, length),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    }
}

fn postgres_binary(fixed: bool, length: Option<u32>) -> MappedColumnType {
    if fixed || length.is_some() {
        return MappedColumnType::new("BYTEA", TypeCompatibility::Lossy)
            .with_warning("PostgreSQL BYTEA 不保留源字段的定长或最大长度约束");
    }
    MappedColumnType::new("BYTEA", TypeCompatibility::Equivalent)
}

fn mysql_binary(fixed: bool, length: Option<u32>) -> MappedColumnType {
    match (fixed, length) {
        (true, Some(length)) if length <= 255 => {
            MappedColumnType::new(format!("BINARY({length})"), TypeCompatibility::Equivalent)
        }
        (false, Some(length)) if length <= 65_535 => MappedColumnType::new(
            format!("VARBINARY({length})"),
            TypeCompatibility::Equivalent,
        ),
        (true, _) => MappedColumnType::new("LONGBLOB", TypeCompatibility::Lossy)
            .with_warning("MySQL LONGBLOB 不保留源二进制字段的定长约束"),
        (false, Some(_)) => MappedColumnType::new("LONGBLOB", TypeCompatibility::Widening)
            .with_warning("源二进制长度超过 VARBINARY 范围，已使用 LONGBLOB"),
        (false, None) => MappedColumnType::new("LONGBLOB", TypeCompatibility::Widening),
    }
}

fn sql_server_binary(fixed: bool, length: Option<u32>) -> MappedColumnType {
    match (fixed, length) {
        (true, Some(length)) if length <= 8_000 => {
            MappedColumnType::new(format!("BINARY({length})"), TypeCompatibility::Equivalent)
        }
        (false, Some(length)) if length <= 8_000 => MappedColumnType::new(
            format!("VARBINARY({length})"),
            TypeCompatibility::Equivalent,
        ),
        (true, _) => MappedColumnType::new("VARBINARY(MAX)", TypeCompatibility::Lossy)
            .with_warning("SQL Server VARBINARY(MAX) 不保留源二进制字段的定长约束"),
        (false, _) => MappedColumnType::new("VARBINARY(MAX)", TypeCompatibility::Widening),
    }
}

fn oracle_binary(fixed: bool, length: Option<u32>) -> MappedColumnType {
    match (fixed, length) {
        (false, Some(length)) if length <= 2_000 => {
            MappedColumnType::new(format!("RAW({length})"), TypeCompatibility::Equivalent)
        }
        (true, Some(length)) if length <= 2_000 => {
            MappedColumnType::new(format!("RAW({length})"), TypeCompatibility::Lossy)
                .with_warning("Oracle RAW 不保留源二进制字段的定长约束")
        }
        (true, _) => MappedColumnType::new("BLOB", TypeCompatibility::Lossy)
            .with_warning("Oracle BLOB 不保留源二进制字段的定长约束"),
        (false, _) => MappedColumnType::new("BLOB", TypeCompatibility::Widening),
    }
}

fn affinity_binary(
    target: AffinityBinaryTarget<'_>,
    fixed: bool,
    length: Option<u32>,
) -> MappedColumnType {
    if fixed || length.is_some() {
        return MappedColumnType::new(target.target_type, TypeCompatibility::Lossy).with_warning(
            format!(
                "{} {} 不保留源二进制字段的定长或最大长度约束",
                target.database_name, target.target_type
            ),
        );
    }
    MappedColumnType::new(target.target_type, TypeCompatibility::Equivalent)
}

fn clickhouse_binary(fixed: bool, length: Option<u32>) -> MappedColumnType {
    match (fixed, length) {
        (true, Some(length)) => MappedColumnType::new(
            format!("FixedString({length})"),
            TypeCompatibility::Equivalent,
        ),
        (true, None) => MappedColumnType::new("String", TypeCompatibility::Lossy)
            .with_warning("ClickHouse String 不保留源二进制字段的定长约束"),
        (false, Some(_)) => MappedColumnType::new("String", TypeCompatibility::Lossy)
            .with_warning("ClickHouse String 不保留源二进制字段的最大长度约束"),
        (false, None) => MappedColumnType::new("String", TypeCompatibility::Equivalent),
    }
}

pub(super) fn map_bit_string(
    varying: bool,
    length: Option<u32>,
    target: &MappingTarget<'_>,
) -> MappedColumnType {
    match target.family {
        DatabaseFamily::MySql if varying => {
            target.unsupported("MySQL BIT 是定长位串，无法无损表示源数据库的变长位串")
        }
        DatabaseFamily::MySql if length.is_some_and(|length| !(1..=64).contains(&length)) => {
            target.unsupported("MySQL BIT 长度必须在 1 到 64 位之间")
        }
        DatabaseFamily::MySql => {
            MappedColumnType::new(format_length("BIT", length), TypeCompatibility::Equivalent)
        }
        DatabaseFamily::PostgreSql
            if length.is_some_and(|length| !(1..=10_485_760).contains(&length)) =>
        {
            target.unsupported("PostgreSQL 位串长度必须在 1 到 10485760 位之间")
        }
        DatabaseFamily::PostgreSql => MappedColumnType::new(
            format_length(if varying { "BIT VARYING" } else { "BIT" }, length),
            TypeCompatibility::Equivalent,
        ),
        _ => target.unsupported("位串类型在目标数据库中没有明确且无损的通用映射"),
    }
}
