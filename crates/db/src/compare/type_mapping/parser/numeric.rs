use super::{
    DatabaseFamily,
    arguments::{NumericArgument, numeric_argument, optional_number},
};
use crate::compare::type_mapping::model::{CanonicalColumnType, ParsedTypeDeclaration};

pub(super) fn parse_integer(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    if !valid_integer_arguments(parsed, family) {
        return None;
    }
    if family == DatabaseFamily::Oracle && parsed.base == "INTEGER" {
        return Some(CanonicalColumnType::Decimal {
            precision: Some(38),
            scale: Some(0),
        });
    }
    let (bits, inherent_unsigned) = integer_spec(parsed, family)?;
    let unsigned = inherent_unsigned
        || parsed.unsigned
        || family == DatabaseFamily::SqlServer && parsed.base == "TINYINT";
    Some(integer(bits, unsigned))
}

fn valid_integer_arguments(parsed: &ParsedTypeDeclaration, family: DatabaseFamily) -> bool {
    if parsed.args.is_empty() {
        return true;
    }
    family == DatabaseFamily::MySql
        && parsed.args.len() == 1
        && matches!(numeric_argument(parsed, 0), NumericArgument::Value(0..=255))
}

fn integer_spec(parsed: &ParsedTypeDeclaration, family: DatabaseFamily) -> Option<(u16, bool)> {
    match parsed.base.as_str() {
        "INT1" | "TINYINT" => Some((8, false)),
        "INT2" | "SMALLINT" | "SMALLSERIAL" | "INT16" => Some((16, false)),
        "MEDIUMINT" => Some((24, false)),
        "INT" | "INT4" | "INTEGER" | "SERIAL" | "INT32" => {
            let bits = if family == DatabaseFamily::Sqlite {
                64
            } else {
                32
            };
            Some((bits, false))
        }
        "INT8" | "BIGINT" | "BIGSERIAL" | "INT64" => Some((64, false)),
        "UTINYINT" | "UINT8" => Some((8, true)),
        "USMALLINT" | "UINT16" => Some((16, true)),
        "UINTEGER" | "UINT" | "UINT32" => Some((32, true)),
        "UBIGINT" | "UINT64" => Some((64, true)),
        _ => None,
    }
}

fn integer(bits: u16, unsigned: bool) -> CanonicalColumnType {
    CanonicalColumnType::Integer { bits, unsigned }
}

pub(super) fn parse_decimal(
    parsed: &ParsedTypeDeclaration,
    _family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    if !matches!(
        parsed.base.as_str(),
        "DECIMAL" | "DEC" | "NUMERIC" | "NUMBER"
    ) || parsed.args.len() > 2
    {
        return None;
    }
    let precision = optional_number(parsed, 0)?;
    if precision == Some(0) {
        return None;
    }
    let scale = decimal_scale(parsed, precision)?;
    if precision.zip(scale).is_some_and(|(p, s)| s > p) {
        return None;
    }
    Some(CanonicalColumnType::Decimal { precision, scale })
}

fn decimal_scale(parsed: &ParsedTypeDeclaration, precision: Option<u32>) -> Option<Option<u32>> {
    match numeric_argument(parsed, 1) {
        NumericArgument::Missing => Some(precision.map(|_| 0)),
        NumericArgument::Value(value) => Some(Some(value)),
        NumericArgument::Invalid => None,
    }
}

pub(super) fn parse_float(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    let bits = match parsed.base.as_str() {
        "REAL" | "FLOAT4" | "FLOAT32" | "BINARY_FLOAT" => {
            return float_without_arguments(parsed, 32);
        }
        "DOUBLE" | "DOUBLE PRECISION" | "FLOAT8" | "FLOAT64" | "BINARY_DOUBLE" => {
            return float_without_arguments(parsed, 64);
        }
        "FLOAT" => float_bits(parsed, family)?,
        _ => return None,
    };
    Some(CanonicalColumnType::Float { bits })
}

fn float_without_arguments(
    parsed: &ParsedTypeDeclaration,
    bits: u16,
) -> Option<CanonicalColumnType> {
    parsed
        .args
        .is_empty()
        .then_some(CanonicalColumnType::Float { bits })
}

fn float_bits(parsed: &ParsedTypeDeclaration, family: DatabaseFamily) -> Option<u16> {
    if parsed.args.len() > 1 {
        return None;
    }
    let precision = match numeric_argument(parsed, 0) {
        NumericArgument::Missing => default_float_precision(family)?,
        NumericArgument::Value(value) if valid_float_precision(value, family) => value,
        NumericArgument::Value(_) | NumericArgument::Invalid => return None,
    };
    Some(if precision <= 24 { 32 } else { 64 })
}

fn default_float_precision(family: DatabaseFamily) -> Option<u32> {
    match family {
        DatabaseFamily::MySql => Some(24),
        DatabaseFamily::PostgreSql | DatabaseFamily::SqlServer | DatabaseFamily::Oracle => Some(53),
        DatabaseFamily::Sqlite | DatabaseFamily::DuckDb | DatabaseFamily::ClickHouse => Some(24),
        DatabaseFamily::Other => None,
    }
}

fn valid_float_precision(precision: u32, family: DatabaseFamily) -> bool {
    match family {
        DatabaseFamily::MySql => precision <= 53,
        DatabaseFamily::PostgreSql | DatabaseFamily::SqlServer | DatabaseFamily::Oracle => {
            (1..=53).contains(&precision)
        }
        DatabaseFamily::Sqlite
        | DatabaseFamily::DuckDb
        | DatabaseFamily::ClickHouse
        | DatabaseFamily::Other => false,
    }
}
