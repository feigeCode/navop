use crate::compare::type_mapping::model::ParsedTypeDeclaration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NumericArgument {
    Missing,
    Value(u32),
    Invalid,
}

pub(super) fn numeric_argument(parsed: &ParsedTypeDeclaration, index: usize) -> NumericArgument {
    let Some(value) = parsed.args.get(index) else {
        return NumericArgument::Missing;
    };
    value
        .parse()
        .map(NumericArgument::Value)
        .unwrap_or(NumericArgument::Invalid)
}

pub(super) fn optional_number(parsed: &ParsedTypeDeclaration, index: usize) -> Option<Option<u32>> {
    match numeric_argument(parsed, index) {
        NumericArgument::Missing => Some(None),
        NumericArgument::Value(value) => Some(Some(value)),
        NumericArgument::Invalid => None,
    }
}

pub(super) fn positive_optional_number(parsed: &ParsedTypeDeclaration) -> Option<Option<u32>> {
    if parsed.args.len() > 1 {
        return None;
    }
    match optional_number(parsed, 0)? {
        None => Some(None),
        Some(value) if value > 0 => Some(Some(value)),
        Some(_) => None,
    }
}

pub(super) fn positive_required_number(parsed: &ParsedTypeDeclaration) -> Option<u32> {
    if parsed.args.len() != 1 {
        return None;
    }
    optional_number(parsed, 0)?.filter(|value| *value > 0)
}
