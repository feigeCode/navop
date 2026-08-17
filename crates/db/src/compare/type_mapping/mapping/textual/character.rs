use super::mapped_text_like;
use crate::compare::type_mapping::{
    family::DatabaseFamily,
    mapping::format_length,
    model::{MappedColumnType, TypeCompatibility},
};

const MYSQL_SAFE_VARCHAR_LENGTH: u32 = 16_383;
const POSTGRES_CHARACTER_LIMIT: u32 = 10_485_760;

pub(in crate::compare::type_mapping::mapping) struct CharacterSpec {
    varying: bool,
    length: Option<u32>,
    national: bool,
}

struct CharacterNames<'a> {
    varying: &'a str,
    fixed: &'a str,
    national_varying: &'a str,
    national_fixed: &'a str,
}

struct CharacterFallback<'a> {
    target_type: &'a str,
    database_name: &'a str,
    preserves_national: bool,
}

impl CharacterSpec {
    pub(in crate::compare::type_mapping::mapping) fn new(
        varying: bool,
        length: Option<u32>,
        national: bool,
    ) -> Self {
        Self {
            varying,
            length,
            national,
        }
    }
}

pub(in crate::compare::type_mapping::mapping) fn map_character(
    spec: CharacterSpec,
    target_family: DatabaseFamily,
) -> MappedColumnType {
    match target_family {
        DatabaseFamily::MySql => mysql_character(spec),
        DatabaseFamily::PostgreSql => postgres_character(spec),
        DatabaseFamily::SqlServer => sql_server_character(spec),
        DatabaseFamily::Oracle => oracle_character(spec),
        DatabaseFamily::Sqlite => MappedColumnType::new("TEXT", TypeCompatibility::Lossy)
            .with_warning("SQLite TEXT 不保留字符字段的长度、定长或 national 约束"),
        DatabaseFamily::DuckDb => duckdb_character(spec),
        DatabaseFamily::ClickHouse => clickhouse_character(spec),
        DatabaseFamily::Other => unreachable!("unknown targets are rejected before mapping"),
    }
}

fn mysql_character(spec: CharacterSpec) -> MappedColumnType {
    if spec.varying && spec.length.is_none() {
        return mapped_text_like(
            "LONGTEXT",
            TypeCompatibility::Widening,
            spec.national.then_some("MySQL"),
        );
    }
    let limit = if spec.varying {
        MYSQL_SAFE_VARCHAR_LENGTH
    } else {
        255
    };
    if invalid_length(spec.length, limit) {
        return character_fallback(
            spec,
            CharacterFallback {
                target_type: "LONGTEXT",
                database_name: "MySQL",
                preserves_national: false,
            },
        );
    }
    let base = character_base(
        &spec,
        CharacterNames {
            varying: "VARCHAR",
            fixed: "CHAR",
            national_varying: "NVARCHAR",
            national_fixed: "NCHAR",
        },
    );
    equivalent_character(base, spec.length)
}

fn postgres_character(spec: CharacterSpec) -> MappedColumnType {
    if invalid_length(spec.length, POSTGRES_CHARACTER_LIMIT) {
        return character_fallback(
            spec,
            CharacterFallback {
                target_type: "TEXT",
                database_name: "PostgreSQL",
                preserves_national: false,
            },
        );
    }
    let base = if spec.varying { "VARCHAR" } else { "CHAR" };
    mapped_text_like(
        format_length(base, spec.length),
        TypeCompatibility::Equivalent,
        spec.national.then_some("PostgreSQL"),
    )
}

fn sql_server_character(spec: CharacterSpec) -> MappedColumnType {
    let limit = if spec.national { 4_000 } else { 8_000 };
    let fallback = if spec.national {
        "NVARCHAR(MAX)"
    } else {
        "VARCHAR(MAX)"
    };
    if invalid_length(spec.length, limit) {
        return character_fallback(
            spec,
            CharacterFallback {
                target_type: fallback,
                database_name: "SQL Server",
                preserves_national: true,
            },
        );
    }
    let base = character_base(
        &spec,
        CharacterNames {
            varying: "VARCHAR",
            fixed: "CHAR",
            national_varying: "NVARCHAR",
            national_fixed: "NCHAR",
        },
    );
    let target_type = if spec.varying && spec.length.is_none() {
        format!("{base}(MAX)")
    } else {
        format_length(base, spec.length)
    };
    MappedColumnType::new(target_type, TypeCompatibility::Equivalent)
}

fn oracle_character(spec: CharacterSpec) -> MappedColumnType {
    let fallback = if spec.national { "NCLOB" } else { "CLOB" };
    if spec.varying && spec.length.is_none() {
        return MappedColumnType::new(fallback, TypeCompatibility::Widening);
    }
    let limit = if spec.varying && !spec.national {
        4_000
    } else {
        2_000
    };
    if invalid_length(spec.length, limit) {
        return character_fallback(
            spec,
            CharacterFallback {
                target_type: fallback,
                database_name: "Oracle",
                preserves_national: true,
            },
        );
    }
    let base = character_base(
        &spec,
        CharacterNames {
            varying: "VARCHAR2",
            fixed: "CHAR",
            national_varying: "NVARCHAR2",
            national_fixed: "NCHAR",
        },
    );
    equivalent_character(base, spec.length)
}

fn duckdb_character(spec: CharacterSpec) -> MappedColumnType {
    let base = if spec.varying { "VARCHAR" } else { "CHAR" };
    mapped_text_like(
        format_length(base, spec.length),
        TypeCompatibility::Equivalent,
        spec.national.then_some("DuckDB"),
    )
}

fn clickhouse_character(spec: CharacterSpec) -> MappedColumnType {
    if !spec.varying {
        if let Some(length) = spec.length {
            return MappedColumnType::new(
                format!("FixedString({length})"),
                TypeCompatibility::Lossy,
            )
            .with_warning("ClickHouse FixedString 按字节而非字符计量长度，多字节字符可能无法保留");
        }
    }
    if spec.length.is_some() || !spec.varying {
        return MappedColumnType::new("String", TypeCompatibility::Lossy)
            .with_warning("ClickHouse String 不保留源字符字段的长度或定长约束");
    }
    mapped_text_like(
        "String",
        TypeCompatibility::Equivalent,
        spec.national.then_some("ClickHouse"),
    )
}

fn equivalent_character(base: &str, length: Option<u32>) -> MappedColumnType {
    MappedColumnType::new(format_length(base, length), TypeCompatibility::Equivalent)
}

fn invalid_length(length: Option<u32>, limit: u32) -> bool {
    length.is_some_and(|length| length == 0 || length > limit)
}

fn character_fallback(spec: CharacterSpec, fallback: CharacterFallback<'_>) -> MappedColumnType {
    let lossy = !spec.varying || spec.national && !fallback.preserves_national;
    let compatibility = if lossy {
        TypeCompatibility::Lossy
    } else {
        TypeCompatibility::Widening
    };
    let warning = if lossy {
        format!(
            "{} {} 无法完整保留源字段的定长、长度或 national 字符语义",
            fallback.database_name, fallback.target_type
        )
    } else {
        format!(
            "源字符长度超过 {} 的安全声明上限，已使用 {}",
            fallback.database_name, fallback.target_type
        )
    };
    MappedColumnType::new(fallback.target_type, compatibility).with_warning(warning)
}

fn character_base<'a>(spec: &CharacterSpec, names: CharacterNames<'a>) -> &'a str {
    match (spec.national, spec.varying) {
        (true, true) => names.national_varying,
        (true, false) => names.national_fixed,
        (false, true) => names.varying,
        (false, false) => names.fixed,
    }
}
