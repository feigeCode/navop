use super::{MappingTarget, format_temporal_type};
use crate::compare::type_mapping::{
    family::DatabaseFamily,
    model::{MappedColumnType, TypeCompatibility},
};

pub(super) struct TemporalSpec {
    precision: Option<u32>,
    with_timezone: bool,
}

impl TemporalSpec {
    pub(super) fn new(precision: Option<u32>, with_timezone: bool) -> Self {
        Self {
            precision,
            with_timezone,
        }
    }
}

struct MappedPrecision {
    value: Option<u32>,
    warning: Option<String>,
}

pub(super) fn map_date(target_family: DatabaseFamily) -> MappedColumnType {
    match target_family {
        DatabaseFamily::Oracle => MappedColumnType::new("DATE", TypeCompatibility::Widening),
        DatabaseFamily::Sqlite => MappedColumnType::new("TEXT", TypeCompatibility::Lossy)
            .with_warning("SQLite 使用文本保存日期，目标列不会保留源数据库的日期类型约束"),
        DatabaseFamily::ClickHouse => MappedColumnType::new("Date", TypeCompatibility::Equivalent),
        DatabaseFamily::MySql
        | DatabaseFamily::PostgreSql
        | DatabaseFamily::SqlServer
        | DatabaseFamily::DuckDb => MappedColumnType::new("DATE", TypeCompatibility::Equivalent),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    }
}

pub(super) fn map_time(spec: TemporalSpec, target: &MappingTarget<'_>) -> MappedColumnType {
    match target.family {
        DatabaseFamily::Oracle => {
            return target.unsupported("Oracle 没有与独立 TIME 完全对应的通用列类型");
        }
        DatabaseFamily::ClickHouse => {
            return MappedColumnType::new("String", TypeCompatibility::Lossy)
                .with_warning("ClickHouse 使用 String 保存独立时间值，时间类型约束将丢失");
        }
        DatabaseFamily::Sqlite => {
            return MappedColumnType::new("TEXT", TypeCompatibility::Lossy)
                .with_warning("SQLite 使用文本保存时间，时间类型约束将丢失");
        }
        DatabaseFamily::SqlServer if spec.with_timezone => {
            return target.unsupported("SQL Server 没有独立的带时区 TIME 列类型");
        }
        _ => {}
    }

    let precision = mapped_precision(spec.precision, target.family);
    let base = time_base(target.family, spec.with_timezone);
    let semantic_warning = (spec.with_timezone && target.family == DatabaseFamily::MySql)
        .then_some("MySQL TIME 不保留源字段的时区语义");
    finish_temporal(
        format_temporal_type(base, precision.value),
        precision.warning,
        semantic_warning,
    )
}

fn time_base(target_family: DatabaseFamily, with_timezone: bool) -> &'static str {
    match (target_family, with_timezone) {
        (DatabaseFamily::PostgreSql, true) => "TIME WITH TIME ZONE",
        (DatabaseFamily::DuckDb, true) => "TIMETZ",
        _ => "TIME",
    }
}

pub(super) fn map_datetime(spec: TemporalSpec, target: &MappingTarget<'_>) -> MappedColumnType {
    if target.family == DatabaseFamily::Sqlite {
        return MappedColumnType::new("TEXT", TypeCompatibility::Lossy)
            .with_warning("SQLite 使用文本保存日期时间，日期时间类型约束将丢失");
    }

    let precision = mapped_precision(spec.precision, target.family);
    let target_type = datetime_type(target.family, spec.with_timezone, precision.value);
    let semantic_warning = (spec.with_timezone && target.family == DatabaseFamily::MySql)
        .then_some("MySQL DATETIME 不保留源字段的时区语义");
    finish_temporal(target_type, precision.warning, semantic_warning)
}

fn datetime_type(
    target_family: DatabaseFamily,
    with_timezone: bool,
    precision: Option<u32>,
) -> String {
    match (target_family, with_timezone) {
        (DatabaseFamily::MySql, _) => format_temporal_type("DATETIME", precision),
        (DatabaseFamily::PostgreSql, false) => format_temporal_type("TIMESTAMP", precision),
        (DatabaseFamily::PostgreSql, true) => format_temporal_type("TIMESTAMPTZ", precision),
        (DatabaseFamily::SqlServer, false) => format_temporal_type("DATETIME2", precision),
        (DatabaseFamily::SqlServer, true) => format_temporal_type("DATETIMEOFFSET", precision),
        (DatabaseFamily::Oracle, false) => format_temporal_type("TIMESTAMP", precision),
        (DatabaseFamily::Oracle, true) => {
            format!(
                "{} WITH TIME ZONE",
                format_temporal_type("TIMESTAMP", precision)
            )
        }
        (DatabaseFamily::DuckDb, false) => format_temporal_type("TIMESTAMP", precision),
        (DatabaseFamily::DuckDb, true) => format_temporal_type("TIMESTAMPTZ", precision),
        (DatabaseFamily::ClickHouse, false) => clickhouse_datetime(precision, None),
        (DatabaseFamily::ClickHouse, true) => clickhouse_datetime(precision, Some("UTC")),
        _ => unreachable!("unknown targets are rejected before mapping"),
    }
}

fn clickhouse_datetime(precision: Option<u32>, timezone: Option<&str>) -> String {
    match (precision, timezone) {
        (Some(precision), Some(timezone)) => {
            format!("DateTime64({precision}, '{timezone}')")
        }
        (Some(precision), None) => format!("DateTime64({precision})"),
        (None, Some(timezone)) => format!("DateTime('{timezone}')"),
        (None, None) => "DateTime".to_string(),
    }
}

fn mapped_precision(precision: Option<u32>, target_family: DatabaseFamily) -> MappedPrecision {
    let Some(source_precision) = precision else {
        return MappedPrecision {
            value: None,
            warning: None,
        };
    };
    let maximum = temporal_precision_limit(target_family);
    let target_precision = source_precision.min(maximum);
    let warning = (target_precision != source_precision).then(|| {
        format!(
            "源字段时间精度 {source_precision} 超过目标数据库上限 {maximum}，已降为 {target_precision}"
        )
    });
    MappedPrecision {
        value: Some(target_precision),
        warning,
    }
}

fn temporal_precision_limit(target_family: DatabaseFamily) -> u32 {
    match target_family {
        DatabaseFamily::MySql | DatabaseFamily::PostgreSql | DatabaseFamily::DuckDb => 6,
        DatabaseFamily::SqlServer => 7,
        DatabaseFamily::Oracle | DatabaseFamily::ClickHouse => 9,
        DatabaseFamily::Sqlite | DatabaseFamily::Other => {
            unreachable!("targets without temporal precision are handled before mapping")
        }
    }
}

fn finish_temporal(
    target_type: String,
    precision_warning: Option<String>,
    semantic_warning: Option<&str>,
) -> MappedColumnType {
    let warning = [precision_warning, semantic_warning.map(str::to_string)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("；");
    if warning.is_empty() {
        MappedColumnType::new(target_type, TypeCompatibility::Equivalent)
    } else {
        MappedColumnType::new(target_type, TypeCompatibility::Lossy).with_warning(warning)
    }
}
