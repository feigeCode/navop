use crate::compare::type_mapping::model::{CanonicalColumnType, ParsedTypeDeclaration};

pub(super) fn parse_character(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    let base = parsed.base.as_str();
    if is_varying_character(base) {
        return parse_varying_character(parsed);
    }
    if is_fixed_character(base) {
        return parse_fixed_character(parsed);
    }
    is_text(base).then(|| CanonicalColumnType::Text {
        national: matches!(base, "NCLOB" | "NTEXT"),
    })
}

fn parse_varying_character(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    let national = parsed.base.starts_with('N') || parsed.base.starts_with("NATIONAL");
    if parsed
        .args
        .first()
        .is_some_and(|value| value.eq_ignore_ascii_case("MAX"))
    {
        return (parsed.args.len() == 1).then_some(CanonicalColumnType::Text { national });
    }
    Some(CanonicalColumnType::Character {
        varying: true,
        length: validated_length(parsed)?,
        national,
    })
}

fn parse_fixed_character(parsed: &ParsedTypeDeclaration) -> Option<CanonicalColumnType> {
    Some(CanonicalColumnType::Character {
        varying: false,
        length: validated_length(parsed)?.or(Some(1)),
        national: parsed.base.starts_with('N'),
    })
}

fn validated_length(parsed: &ParsedTypeDeclaration) -> Option<Option<u32>> {
    if parsed.args.is_empty() {
        return Some(None);
    }
    if parsed.args.len() != 1 {
        return None;
    }
    parsed.args[0]
        .parse::<u32>()
        .ok()
        .filter(|length| *length > 0)
        .map(Some)
}

fn is_varying_character(base: &str) -> bool {
    matches!(
        base,
        "VARCHAR"
            | "VARCHAR2"
            | "CHARACTER VARYING"
            | "NVARCHAR"
            | "NVARCHAR2"
            | "NATIONAL VARCHAR"
            | "NATIONAL CHARACTER VARYING"
    )
}

fn is_fixed_character(base: &str) -> bool {
    matches!(
        base,
        "CHAR" | "CHARACTER" | "NCHAR" | "NATIONAL CHAR" | "NATIONAL CHARACTER" | "BPCHAR"
    )
}

fn is_text(base: &str) -> bool {
    matches!(
        base,
        "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "CLOB" | "NCLOB" | "NTEXT" | "STRING"
    )
}
