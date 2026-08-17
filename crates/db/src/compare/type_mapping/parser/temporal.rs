use super::{DatabaseFamily, arguments::optional_number};
use crate::compare::type_mapping::model::{CanonicalColumnType, ParsedTypeDeclaration};

pub(super) fn parse_temporal(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    let base = parsed.base.as_str();
    if base == "DATE" {
        return parse_date(parsed, family);
    }
    if is_time(base) {
        return Some(CanonicalColumnType::Time {
            precision: standard_precision(parsed)?,
            with_timezone: matches!(base, "TIME WITH TIME ZONE" | "TIMETZ"),
        });
    }
    if !is_datetime(base) {
        return None;
    }
    let (precision, with_timezone) = datetime_spec(parsed, family)?;
    Some(CanonicalColumnType::DateTime {
        precision,
        with_timezone,
    })
}

fn parse_date(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    if !parsed.args.is_empty() {
        return None;
    }
    Some(if family == DatabaseFamily::Oracle {
        CanonicalColumnType::DateTime {
            precision: Some(0),
            with_timezone: false,
        }
    } else {
        CanonicalColumnType::Date
    })
}

fn datetime_spec(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<(Option<u32>, bool)> {
    if family == DatabaseFamily::ClickHouse {
        match parsed.base.as_str() {
            "DATETIME" => return clickhouse_datetime(parsed),
            "DATETIME64" => return clickhouse_datetime64(parsed),
            _ => {}
        }
    }
    Some((
        standard_precision(parsed)?,
        datetime_has_timezone(&parsed.base),
    ))
}

fn clickhouse_datetime(parsed: &ParsedTypeDeclaration) -> Option<(Option<u32>, bool)> {
    match parsed.args.as_slice() {
        [] => Some((None, false)),
        [timezone] if is_quoted_string(timezone) => Some((None, true)),
        _ => None,
    }
}

fn clickhouse_datetime64(parsed: &ParsedTypeDeclaration) -> Option<(Option<u32>, bool)> {
    if !(1..=2).contains(&parsed.args.len()) {
        return None;
    }
    let precision = optional_number(parsed, 0)?.filter(|value| *value <= 9)?;
    let with_timezone = match parsed.args.get(1) {
        None => false,
        Some(timezone) if is_quoted_string(timezone) => true,
        Some(_) => return None,
    };
    Some((Some(precision), with_timezone))
}

fn standard_precision(parsed: &ParsedTypeDeclaration) -> Option<Option<u32>> {
    if parsed.args.len() > 1 {
        return None;
    }
    optional_number(parsed, 0)
}

fn is_quoted_string(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && matches!(
            (bytes.first(), bytes.last()),
            (Some(b'\''), Some(b'\'')) | (Some(b'"'), Some(b'"'))
        )
        && !value[1..value.len() - 1].is_empty()
}

fn is_time(base: &str) -> bool {
    matches!(
        base,
        "TIME" | "TIME WITHOUT TIME ZONE" | "TIME WITH TIME ZONE" | "TIMETZ"
    )
}

fn is_datetime(base: &str) -> bool {
    matches!(
        base,
        "DATETIME"
            | "DATETIME2"
            | "SMALLDATETIME"
            | "TIMESTAMP"
            | "TIMESTAMP WITHOUT TIME ZONE"
            | "TIMESTAMP WITH TIME ZONE"
            | "TIMESTAMP WITH LOCAL TIME ZONE"
            | "TIMESTAMPTZ"
            | "DATETIMEOFFSET"
            | "DATETIME64"
    )
}

fn datetime_has_timezone(base: &str) -> bool {
    matches!(
        base,
        "TIMESTAMP WITH TIME ZONE"
            | "TIMESTAMP WITH LOCAL TIME ZONE"
            | "TIMESTAMPTZ"
            | "DATETIMEOFFSET"
    )
}
