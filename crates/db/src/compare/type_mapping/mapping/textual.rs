mod character;

pub(super) use character::{CharacterSpec, map_character};

use super::MappingTarget;
use crate::compare::type_mapping::{
    family::DatabaseFamily,
    model::{MappedColumnType, TypeCompatibility},
};

pub(super) fn map_text(national: bool, target_family: DatabaseFamily) -> MappedColumnType {
    match target_family {
        DatabaseFamily::MySql => mapped_text_like(
            "LONGTEXT",
            TypeCompatibility::Widening,
            national.then_some("MySQL"),
        ),
        DatabaseFamily::PostgreSql => mapped_text_like(
            "TEXT",
            TypeCompatibility::Equivalent,
            national.then_some("PostgreSQL"),
        ),
        DatabaseFamily::SqlServer => MappedColumnType::new(
            if national {
                "NVARCHAR(MAX)"
            } else {
                "VARCHAR(MAX)"
            },
            TypeCompatibility::Equivalent,
        ),
        DatabaseFamily::Oracle => MappedColumnType::new(
            if national { "NCLOB" } else { "CLOB" },
            TypeCompatibility::Equivalent,
        ),
        DatabaseFamily::Sqlite => mapped_text_like(
            "TEXT",
            TypeCompatibility::Equivalent,
            national.then_some("SQLite"),
        ),
        DatabaseFamily::DuckDb => mapped_text_like(
            "VARCHAR",
            TypeCompatibility::Equivalent,
            national.then_some("DuckDB"),
        ),
        DatabaseFamily::ClickHouse => mapped_text_like(
            "String",
            TypeCompatibility::Equivalent,
            national.then_some("ClickHouse"),
        ),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    }
}

fn mapped_text_like(
    target_type: impl Into<String>,
    compatibility: TypeCompatibility,
    national_loss_database: Option<&str>,
) -> MappedColumnType {
    let target_type = target_type.into();
    if let Some(database_name) = national_loss_database {
        return MappedColumnType::new(target_type, TypeCompatibility::Lossy).with_warning(format!(
            "{database_name} 的目标类型不保留源字段的 national 字符语义"
        ));
    }
    MappedColumnType::new(target_type, compatibility)
}

pub(super) fn map_json(target: &MappingTarget<'_>) -> MappedColumnType {
    match target.family {
        DatabaseFamily::MySql => MappedColumnType::new("JSON", TypeCompatibility::Equivalent),
        DatabaseFamily::PostgreSql => MappedColumnType::new("JSONB", TypeCompatibility::Equivalent),
        DatabaseFamily::Sqlite => MappedColumnType::new("JSON", TypeCompatibility::Lossy)
            .with_warning("SQLite JSON 不保留源数据库的严格 JSON 类型约束"),
        DatabaseFamily::DuckDb => MappedColumnType::new("JSON", TypeCompatibility::Equivalent),
        DatabaseFamily::SqlServer => {
            MappedColumnType::new("NVARCHAR(MAX)", TypeCompatibility::Lossy)
                .with_warning("SQL Server 将 JSON 映射为 NVARCHAR(MAX)，不会自动保留 JSON 校验约束")
        }
        DatabaseFamily::Oracle => MappedColumnType::new("CLOB", TypeCompatibility::Lossy)
            .with_warning("Oracle 将 JSON 映射为 CLOB，不会自动保留 JSON 校验约束"),
        DatabaseFamily::ClickHouse => MappedColumnType::new("String", TypeCompatibility::Lossy)
            .with_warning("ClickHouse 将 JSON 映射为 String，JSON 类型约束将丢失"),
        DatabaseFamily::Other => target.unsupported("目标数据库不支持已知的 JSON 类型映射"),
    }
}

pub(super) fn map_uuid(target_family: DatabaseFamily) -> MappedColumnType {
    match target_family {
        DatabaseFamily::PostgreSql | DatabaseFamily::DuckDb | DatabaseFamily::ClickHouse => {
            MappedColumnType::new("UUID", TypeCompatibility::Equivalent)
        }
        DatabaseFamily::SqlServer => {
            MappedColumnType::new("UNIQUEIDENTIFIER", TypeCompatibility::Equivalent)
        }
        DatabaseFamily::MySql => MappedColumnType::new("CHAR(36)", TypeCompatibility::Lossy)
            .with_warning("UUID 将使用 CHAR(36) 文本保存，目标列不再保留 UUID 类型约束"),
        DatabaseFamily::Oracle => MappedColumnType::new("VARCHAR2(36)", TypeCompatibility::Lossy)
            .with_warning("UUID 将使用 VARCHAR2(36) 文本保存，目标列不再保留 UUID 类型约束"),
        DatabaseFamily::Sqlite => MappedColumnType::new("TEXT", TypeCompatibility::Lossy)
            .with_warning("UUID 将使用 TEXT 文本保存，目标列不再保留 UUID 类型约束"),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    }
}
