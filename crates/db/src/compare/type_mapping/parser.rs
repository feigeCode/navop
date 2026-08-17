mod arguments;
mod character;
mod declaration;
mod numeric;
mod temporal;

use super::{
    family::DatabaseFamily,
    model::{CanonicalColumnType, ParsedTypeDeclaration},
};
use arguments::{
    NumericArgument, numeric_argument, positive_optional_number, positive_required_number,
};
use character::parse_character;
use declaration::{is_complex_type, parse_type_declaration};
use numeric::{parse_decimal, parse_float, parse_integer};
use temporal::parse_temporal;

pub(super) fn normalized_type_declaration(value: &str) -> String {
    declaration::normalized_type_declaration(value)
}

pub(super) fn parse_canonical_type(
    declaration: &str,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    let parsed = parse_type_declaration(declaration)?;
    if is_complex_type(&parsed.base) {
        return None;
    }

    parse_boolean_or_rowversion(&parsed, family)
        .or_else(|| parse_bit_string(&parsed))
        .or_else(|| parse_integer(&parsed, family))
        .or_else(|| parse_decimal(&parsed, family))
        .or_else(|| parse_float(&parsed, family))
        .or_else(|| parse_character(&parsed))
        .or_else(|| parse_binary(&parsed))
        .or_else(|| parse_temporal(&parsed, family))
        .or_else(|| parse_json_or_uuid(&parsed))
}

fn parse_boolean_or_rowversion(
    parsed: &ParsedTypeDeclaration,
    family: DatabaseFamily,
) -> Option<CanonicalColumnType> {
    let first_number = numeric_argument(parsed, 0);
    let boolean = matches!(parsed.base.as_str(), "BOOL" | "BOOLEAN") && parsed.args.is_empty()
        || family == DatabaseFamily::SqlServer && parsed.base == "BIT" && parsed.args.is_empty()
        || family == DatabaseFamily::MySql
            && parsed.args.len() <= 1
            && (parsed.base == "TINYINT" && first_number == NumericArgument::Value(1)
                || parsed.base == "BIT"
                    && matches!(
                        first_number,
                        NumericArgument::Missing | NumericArgument::Value(1)
                    ));
    if boolean {
        return Some(CanonicalColumnType::Boolean);
    }
    if family == DatabaseFamily::SqlServer
        && matches!(parsed.base.as_str(), "TIMESTAMP" | "ROWVERSION")
        && parsed.args.is_empty()
    {
        return Some(CanonicalColumnType::RowVersion);
    }
    None
}

fn parse_bit_string(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    if !matches!(parsed.base.as_str(), "BIT" | "BIT VARYING" | "VARBIT") {
        return None;
    }
    Some(CanonicalColumnType::BitString {
        varying: parsed.base != "BIT",
        length: positive_optional_number(parsed)?,
    })
}

fn parse_binary(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    let base = parsed.base.as_str();
    if base == "FIXEDSTRING" {
        return Some(CanonicalColumnType::Binary {
            fixed: true,
            length: Some(positive_required_number(parsed)?),
        });
    }
    if matches!(base, "BINARY" | "VARBINARY" | "RAW") {
        let is_max = parsed
            .args
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case("MAX"));
        if is_max {
            return (base == "VARBINARY" && parsed.args.len() == 1).then_some(
                CanonicalColumnType::Binary {
                    fixed: false,
                    length: None,
                },
            );
        }
        return Some(CanonicalColumnType::Binary {
            fixed: base == "BINARY",
            length: positive_optional_number(parsed)?,
        });
    }
    (matches!(
        base,
        "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BYTEA" | "IMAGE"
    ) && parsed.args.is_empty())
    .then_some(CanonicalColumnType::Binary {
        fixed: false,
        length: None,
    })
}

fn parse_json_or_uuid(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    match parsed.base.as_str() {
        "JSON" | "JSONB" => Some(CanonicalColumnType::Json),
        "UUID" | "GUID" | "UNIQUEIDENTIFIER" => Some(CanonicalColumnType::Uuid),
        _ => None,
    }
}
