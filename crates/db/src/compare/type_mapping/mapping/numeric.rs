use super::{MappingTarget, format_precision_scale};
use crate::compare::type_mapping::{
    family::DatabaseFamily,
    model::{MappedColumnType, TypeCompatibility},
};

pub(super) fn map_boolean(target: &MappingTarget<'_>) -> MappedColumnType {
    match target.family {
        DatabaseFamily::MySql | DatabaseFamily::PostgreSql | DatabaseFamily::DuckDb => {
            MappedColumnType::new("BOOLEAN", TypeCompatibility::Equivalent)
        }
        DatabaseFamily::SqlServer => MappedColumnType::new("BIT", TypeCompatibility::Equivalent),
        DatabaseFamily::Oracle => {
            MappedColumnType::new("NUMBER(1,0)", TypeCompatibility::Equivalent)
        }
        DatabaseFamily::ClickHouse => MappedColumnType::new("UInt8", TypeCompatibility::Equivalent),
        DatabaseFamily::Sqlite => MappedColumnType::new("BOOLEAN", TypeCompatibility::Lossy)
            .with_warning("SQLite BOOLEAN 使用类型亲和性保存，不能保留严格布尔约束"),
        DatabaseFamily::Other => target.unsupported("目标数据库不支持已知的布尔类型映射"),
    }
}

pub(super) fn map_integer(
    bits: u16,
    unsigned: bool,
    target_family: DatabaseFamily,
) -> MappedColumnType {
    let (target_type, compatibility) = match target_family {
        DatabaseFamily::MySql => mysql_integer(bits, unsigned),
        DatabaseFamily::PostgreSql => postgres_integer(bits, unsigned),
        DatabaseFamily::SqlServer => sql_server_integer(bits, unsigned),
        DatabaseFamily::Oracle => oracle_integer(bits, unsigned),
        DatabaseFamily::Sqlite => sqlite_integer(bits, unsigned),
        DatabaseFamily::DuckDb => duckdb_integer(bits, unsigned),
        DatabaseFamily::ClickHouse => clickhouse_integer(bits, unsigned),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    };
    let mapped = MappedColumnType::new(target_type, compatibility);
    if target_family == DatabaseFamily::Sqlite {
        mapped.with_warning("SQLite INTEGER 使用动态类型，不能保留严格整数位宽或无符号约束")
    } else {
        mapped
    }
}

fn mysql_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    let base = match bits {
        0..=8 => "TINYINT",
        9..=16 => "SMALLINT",
        17..=32 => "INT",
        _ => "BIGINT",
    };
    let target_type = if unsigned {
        format!("{base} UNSIGNED")
    } else {
        base.to_string()
    };
    (target_type, TypeCompatibility::Equivalent)
}

fn postgres_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    match (bits, unsigned) {
        (0..=8, false) => ("SMALLINT".to_string(), TypeCompatibility::Widening),
        (0..=16, false) => ("SMALLINT".to_string(), TypeCompatibility::Equivalent),
        (17..=32, false) => ("INTEGER".to_string(), TypeCompatibility::Equivalent),
        (_, false) => ("BIGINT".to_string(), TypeCompatibility::Equivalent),
        (0..=8, true) => ("SMALLINT".to_string(), TypeCompatibility::Widening),
        (9..=16, true) => ("INTEGER".to_string(), TypeCompatibility::Widening),
        (17..=32, true) => ("BIGINT".to_string(), TypeCompatibility::Widening),
        (_, true) => ("NUMERIC(20,0)".to_string(), TypeCompatibility::Widening),
    }
}

fn sql_server_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    match (bits, unsigned) {
        (0..=8, true) => ("TINYINT".to_string(), TypeCompatibility::Equivalent),
        (0..=8, false) => ("SMALLINT".to_string(), TypeCompatibility::Widening),
        (9..=16, false) => ("SMALLINT".to_string(), TypeCompatibility::Equivalent),
        (0..=16, true) => ("INT".to_string(), TypeCompatibility::Widening),
        (17..=32, false) => ("INT".to_string(), TypeCompatibility::Equivalent),
        (17..=32, true) => ("BIGINT".to_string(), TypeCompatibility::Widening),
        (_, false) => ("BIGINT".to_string(), TypeCompatibility::Equivalent),
        (_, true) => ("DECIMAL(20,0)".to_string(), TypeCompatibility::Widening),
    }
}

fn oracle_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    let digits = match (bits, unsigned) {
        (0..=8, _) => 3,
        (9..=16, _) => 5,
        (17..=32, _) => 10,
        (_, false) => 19,
        (_, true) => 20,
    };
    (format!("NUMBER({digits},0)"), TypeCompatibility::Widening)
}

fn sqlite_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    let target_type = if unsigned && bits > 63 {
        "NUMERIC"
    } else {
        "INTEGER"
    };
    (target_type.to_string(), TypeCompatibility::Lossy)
}

fn duckdb_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    let target_type = match (bits, unsigned) {
        (0..=8, false) => "TINYINT",
        (9..=16, false) => "SMALLINT",
        (17..=32, false) => "INTEGER",
        (_, false) => "BIGINT",
        (0..=8, true) => "UTINYINT",
        (9..=16, true) => "USMALLINT",
        (17..=32, true) => "UINTEGER",
        (_, true) => "UBIGINT",
    };
    (target_type.to_string(), TypeCompatibility::Equivalent)
}

fn clickhouse_integer(bits: u16, unsigned: bool) -> (String, TypeCompatibility) {
    let prefix = if unsigned { "UInt" } else { "Int" };
    let width = match bits {
        0..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        _ => 64,
    };
    let compatibility = if width == bits {
        TypeCompatibility::Equivalent
    } else {
        TypeCompatibility::Widening
    };
    (format!("{prefix}{width}"), compatibility)
}

pub(super) fn map_decimal(
    precision: Option<u32>,
    scale: Option<u32>,
    target: &MappingTarget<'_>,
) -> MappedColumnType {
    if precision
        .zip(scale)
        .is_some_and(|(precision, scale)| scale > precision)
    {
        return target.unsupported("字段 scale 大于 precision，无法生成有效的目标类型");
    }
    if precision.is_none() && decimal_precision_limit(target.family).is_some() {
        return target.unsupported("源字段未声明精度，而目标数据库使用有界精度，不能安全推断");
    }
    if precision
        .zip(decimal_precision_limit(target.family))
        .is_some_and(|(precision, max)| precision > max)
    {
        return target.unsupported("源字段精度超过目标数据库的安全精度上限，不能静默缩小");
    }
    if target.family == DatabaseFamily::ClickHouse && precision.is_none() {
        return target.unsupported("ClickHouse Decimal 需要明确的精度，不能从无精度声明安全推断");
    }

    mapped_decimal(precision, scale, target.family)
}

fn mapped_decimal(
    precision: Option<u32>,
    scale: Option<u32>,
    target_family: DatabaseFamily,
) -> MappedColumnType {
    let base = match target_family {
        DatabaseFamily::PostgreSql | DatabaseFamily::Sqlite => "NUMERIC",
        DatabaseFamily::Oracle => "NUMBER",
        DatabaseFamily::MySql | DatabaseFamily::SqlServer | DatabaseFamily::DuckDb => "DECIMAL",
        DatabaseFamily::ClickHouse => "Decimal",
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    };
    let target_type = format_precision_scale(base, precision, scale);
    if target_family == DatabaseFamily::Sqlite {
        return MappedColumnType::new(target_type, TypeCompatibility::Lossy)
            .with_warning("SQLite NUMERIC 仅使用类型亲和性，不能保留精度和小数位约束");
    }
    MappedColumnType::new(target_type, TypeCompatibility::Equivalent)
}

fn decimal_precision_limit(target_family: DatabaseFamily) -> Option<u32> {
    match target_family {
        DatabaseFamily::MySql => Some(65),
        DatabaseFamily::SqlServer | DatabaseFamily::Oracle | DatabaseFamily::DuckDb => Some(38),
        DatabaseFamily::ClickHouse => Some(76),
        DatabaseFamily::PostgreSql | DatabaseFamily::Sqlite => None,
        DatabaseFamily::Other => None,
    }
}

pub(super) fn map_float(bits: u16, target_family: DatabaseFamily) -> MappedColumnType {
    let target_type = match (target_family, bits <= 32) {
        (DatabaseFamily::MySql, true) => "FLOAT",
        (DatabaseFamily::MySql, false) => "DOUBLE",
        (DatabaseFamily::PostgreSql, true) => "REAL",
        (DatabaseFamily::PostgreSql, false) => "DOUBLE PRECISION",
        (DatabaseFamily::SqlServer, true) => "REAL",
        (DatabaseFamily::SqlServer, false) => "FLOAT(53)",
        (DatabaseFamily::Oracle, true) => "BINARY_FLOAT",
        (DatabaseFamily::Oracle, false) => "BINARY_DOUBLE",
        (DatabaseFamily::Sqlite, _) => "REAL",
        (DatabaseFamily::DuckDb, true) => "FLOAT",
        (DatabaseFamily::DuckDb, false) => "DOUBLE",
        (DatabaseFamily::ClickHouse, true) => "Float32",
        (DatabaseFamily::ClickHouse, false) => "Float64",
        (DatabaseFamily::Other, _) => unreachable!("unknown targets are rejected before mapping"),
    };
    let compatibility = if target_family == DatabaseFamily::Sqlite {
        TypeCompatibility::Lossy
    } else {
        TypeCompatibility::Equivalent
    };
    let mapped = MappedColumnType::new(target_type, compatibility);
    if compatibility == TypeCompatibility::Lossy {
        mapped.with_warning("SQLite REAL 使用动态类型，不能保留严格浮点类型约束")
    } else {
        mapped
    }
}
