mod binary;
mod numeric;
mod temporal;
mod textual;

use one_core::storage::DatabaseType;

use super::{
    family::{DatabaseFamily, database_family},
    model::{CanonicalColumnType, MappedColumnType, TypeCompatibility},
    parser::normalized_type_declaration,
};

pub(super) struct MappingTarget<'a> {
    pub source_type: &'a str,
    pub database_type: &'a DatabaseType,
    pub family: DatabaseFamily,
}

impl<'a> MappingTarget<'a> {
    fn new(source_type: &'a str, database_type: &'a DatabaseType) -> Self {
        Self {
            source_type,
            database_type,
            family: database_family(database_type),
        }
    }

    pub fn unsupported(&self, reason: &str) -> MappedColumnType {
        unsupported_mapping(self.source_type, self.database_type, reason)
    }
}

pub(super) fn map_canonical_type(
    canonical: CanonicalColumnType,
    source_type: &str,
    target_database_type: &DatabaseType,
) -> MappedColumnType {
    let target = MappingTarget::new(source_type, target_database_type);
    match canonical {
        CanonicalColumnType::Boolean => numeric::map_boolean(&target),
        CanonicalColumnType::Integer { bits, unsigned } => {
            numeric::map_integer(bits, unsigned, target.family)
        }
        CanonicalColumnType::Decimal { precision, scale } => {
            numeric::map_decimal(precision, scale, &target)
        }
        CanonicalColumnType::Float { bits } => numeric::map_float(bits, target.family),
        CanonicalColumnType::Character {
            varying,
            length,
            national,
        } => textual::map_character(
            textual::CharacterSpec::new(varying, length, national),
            target.family,
        ),
        CanonicalColumnType::Text { national } => textual::map_text(national, target.family),
        CanonicalColumnType::Binary { fixed, length } => {
            binary::map_binary(fixed, length, target.family)
        }
        CanonicalColumnType::BitString { varying, length } => {
            binary::map_bit_string(varying, length, &target)
        }
        CanonicalColumnType::Date => temporal::map_date(target.family),
        CanonicalColumnType::Time {
            precision,
            with_timezone,
        } => temporal::map_time(
            temporal::TemporalSpec::new(precision, with_timezone),
            &target,
        ),
        CanonicalColumnType::DateTime {
            precision,
            with_timezone,
        } => temporal::map_datetime(
            temporal::TemporalSpec::new(precision, with_timezone),
            &target,
        ),
        CanonicalColumnType::Json => textual::map_json(&target),
        CanonicalColumnType::Uuid => textual::map_uuid(target.family),
        CanonicalColumnType::RowVersion => map_row_version(&target),
    }
}

fn map_row_version(target: &MappingTarget<'_>) -> MappedColumnType {
    target.unsupported("SQL Server rowversion/timestamp 不是日期时间类型，无法安全跨库映射")
}

pub(super) fn unsupported_mapping(
    source_type: &str,
    target_database_type: &DatabaseType,
    reason: &str,
) -> MappedColumnType {
    MappedColumnType::new(
        normalized_type_declaration(source_type),
        TypeCompatibility::Unsupported,
    )
    .with_warning(format!(
        "无法将字段类型 `{}` 安全映射到 {}：{}",
        source_type.trim(),
        target_database_type.storage_key(),
        reason
    ))
}

fn format_length(base: &str, length: Option<u32>) -> String {
    length.map_or_else(|| base.to_string(), |length| format!("{base}({length})"))
}

fn format_precision_scale(base: &str, precision: Option<u32>, scale: Option<u32>) -> String {
    match (precision, scale) {
        (Some(precision), Some(scale)) => format!("{base}({precision},{scale})"),
        (Some(precision), None) => format!("{base}({precision})"),
        (None, _) => base.to_string(),
    }
}

fn format_temporal_type(base: &str, precision: Option<u32>) -> String {
    precision.map_or_else(
        || base.to_string(),
        |precision| format!("{base}({precision})"),
    )
}
