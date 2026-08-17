use crate::compare::type_mapping::model::ParsedTypeDeclaration;

pub(super) fn parse_type_declaration(declaration: &str) -> Option<ParsedTypeDeclaration> {
    let normalized = strip_wrappers(normalized_type_declaration(declaration));
    if normalized.is_empty() {
        return None;
    }
    let (normalized, unsigned) = strip_attributes(&normalized);
    let (base, args) = split_base_and_args(&normalized)?;
    Some(ParsedTypeDeclaration {
        base: normalized_base(&base),
        args,
        unsigned,
    })
}

fn normalized_base(base: &str) -> String {
    base.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

fn strip_wrappers(mut declaration: String) -> String {
    loop {
        let Some(open_index) = wrapper_open_index(&declaration) else {
            return declaration;
        };
        if matching_closing_parenthesis(&declaration, open_index) != Some(declaration.len() - 1) {
            return declaration;
        }
        declaration = declaration[open_index + 1..declaration.len() - 1]
            .trim()
            .to_string();
    }
}

fn wrapper_open_index(declaration: &str) -> Option<usize> {
    let upper = declaration.to_ascii_uppercase();
    ["NULLABLE", "LOWCARDINALITY"]
        .into_iter()
        .find(|wrapper| upper.starts_with(&format!("{wrapper}(")))
        .map(str::len)
}

fn strip_attributes(declaration: &str) -> (String, bool) {
    let mut unsigned = false;
    let tokens = declaration.split_whitespace().filter(|token| {
        if token.eq_ignore_ascii_case("UNSIGNED") {
            unsigned = true;
            return false;
        }
        !matches!(
            token.to_ascii_uppercase().as_str(),
            "SIGNED" | "ZEROFILL" | "AUTO_INCREMENT"
        )
    });
    (tokens.collect::<Vec<_>>().join(" "), unsigned)
}

fn split_base_and_args(declaration: &str) -> Option<(String, Vec<String>)> {
    let Some(open_index) = declaration.find('(') else {
        return Some((declaration.to_string(), Vec::new()));
    };
    let close_index = matching_closing_parenthesis(declaration, open_index)?;
    let before = declaration[..open_index].trim();
    let after = declaration[close_index + 1..].trim();
    let base = if after.is_empty() {
        before.to_string()
    } else {
        format!("{before} {after}")
    };
    Some((
        base,
        split_type_arguments(&declaration[open_index + 1..close_index]),
    ))
}

fn matching_closing_parenthesis(value: &str, open_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in value
        .char_indices()
        .skip_while(|(index, _)| *index < open_index)
    {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_type_arguments(value: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quoted = None;
    for (index, ch) in value.char_indices() {
        if update_quote(ch, &mut quoted) {
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(value[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    push_final_argument(value, start, &mut args);
    args
}

fn push_final_argument(value: &str, start: usize, args: &mut Vec<String>) {
    let final_arg = value[start..].trim();
    if !final_arg.is_empty() {
        args.push(final_arg.to_string());
    }
}

fn update_quote(ch: char, quoted: &mut Option<char>) -> bool {
    if let Some(quote) = *quoted {
        if ch == quote {
            *quoted = None;
        }
        return true;
    }
    if matches!(ch, '\'' | '"') {
        *quoted = Some(ch);
        return true;
    }
    false
}

pub(super) fn is_complex_type(base: &str) -> bool {
    [
        "ARRAY",
        "LIST",
        "MAP",
        "STRUCT",
        "TUPLE",
        "ENUM",
        "ENUM8",
        "ENUM16",
        "SET",
        "XML",
        "GEOMETRY",
        "GEOGRAPHY",
        "VECTOR",
        "INTERVAL",
        "OBJECT",
        "VARIANT",
    ]
    .into_iter()
    .any(|complex| base == complex || base.starts_with(&format!("{complex} ")))
}

pub(super) fn normalized_type_declaration(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
