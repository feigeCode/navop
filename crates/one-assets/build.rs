use std::{
    env,
    fmt::Write,
    fs,
    path::PathBuf,
};

/// Convert an SVG filename to PascalCase identifier (see the fork's
/// `icon_named!` macro for the original behavior).
fn pascal_case(filename: &str) -> String {
    filename
        .strip_suffix(".svg")
        .unwrap_or(filename)
        .split(|c: char| c == '-' || c == '_' || c == '.')
        .filter(|part| !part.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) if first.is_ascii_digit() => word.to_string(),
                Some(first) => {
                    let mut result = String::with_capacity(word.len());
                    result.extend(first.to_uppercase());
                    result.push_str(&chars.as_str().to_lowercase());
                    result
                }
            }
        })
        .collect()
}

/// Whether the SVG paints with intrinsic colors rather than `currentColor`.
fn svg_uses_intrinsic_color(svg: &str) -> bool {
    let svg = strip_xml_comments(svg);
    if css_uses_intrinsic_color(&svg) {
        return true;
    }
    let mut in_tag = false;
    let mut tag = String::new();

    for character in svg.chars() {
        match (in_tag, character) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                if tag.trim_start().to_ascii_lowercase().starts_with("image") {
                    return true;
                }
                if tag_has_intrinsic_paint(&tag) {
                    return true;
                }
                in_tag = false;
            }
            (true, _) => tag.push(character),
            (false, _) => {}
        }
    }

    false
}

fn css_uses_intrinsic_color(svg: &str) -> bool {
    let svg = svg.to_ascii_lowercase();
    for property in ["fill", "stroke", "stop-color"] {
        let mut remaining = svg.as_str();
        while let Some(start) = remaining.find(property) {
            let property_start = &remaining[..start];
            remaining = &remaining[start + property.len()..];
            if !property_start
                .chars()
                .next_back()
                .map_or(true, |character| {
                    character.is_ascii_whitespace() || matches!(character, '{' | ';')
                })
            {
                continue;
            }
            let Some(value) = remaining.trim_start().strip_prefix(':') else {
                continue;
            };
            let end = value
                .find(|character: char| matches!(character, ';' | '}' | '"' | '\''))
                .unwrap_or(value.len());
            if is_intrinsic_paint(&value[..end]) {
                return true;
            }
        }
    }

    false
}

fn strip_xml_comments(svg: &str) -> String {
    let mut result = String::with_capacity(svg.len());
    let mut remaining = svg;
    while let Some(start) = remaining.find("<!--") {
        result.push_str(&remaining[..start]);
        let Some(end) = remaining[start + 4..].find("-->") else {
            return result;
        };
        remaining = &remaining[start + 4 + end + 3..];
    }
    result.push_str(remaining);
    result
}

fn tag_has_intrinsic_paint(tag: &str) -> bool {
    let tag = tag.to_ascii_lowercase();
    for attribute in ["fill", "stroke", "stop-color", "style"] {
        let mut remaining = tag.as_str();
        while let Some(start) = remaining.find(attribute) {
            let attribute_start = &remaining[..start];
            remaining = &remaining[start + attribute.len()..];
            if !attribute_start
                .chars()
                .next_back()
                .map_or(true, |character| character.is_ascii_whitespace())
            {
                continue;
            }
            let after_attribute = remaining.trim_start();
            let Some(value) = after_attribute.strip_prefix('=') else {
                continue;
            };
            let value = value.trim_start();
            let Some(quote) = value.chars().next() else {
                break;
            };
            if quote != '\'' && quote != '"' {
                continue;
            }
            let value = &value[quote.len_utf8()..];
            let Some(end) = value.find(quote) else { break };
            let value = &value[..end];
            if attribute == "style" {
                if value.split(';').any(|declaration| {
                    declaration
                        .split_once(':')
                        .is_some_and(|(property, value)| {
                            matches!(
                                property.trim(),
                                "fill" | "stroke" | "stop-color" | "flood-color"
                            ) && is_intrinsic_paint(value)
                        })
                }) {
                    return true;
                }
            } else if is_intrinsic_paint(value) {
                return true;
            }
        }
    }

    false
}

fn is_intrinsic_paint(value: &str) -> bool {
    !matches!(
        value.trim(),
        "none" | "currentcolor" | "inherit" | "context-fill" | "context-stroke"
    )
}

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icons_dir = manifest_dir.join("assets/icons");
    println!("cargo:rerun-if-changed=assets/icons");
    println!("cargo:rerun-if-changed=build.rs");

    let mut entries: Vec<(String, String, bool)> = Vec::new();
    for entry in fs::read_dir(&icons_dir).expect("icons directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("svg") {
            continue;
        }
        let filename = path.file_name().unwrap().to_str().unwrap().to_string();
        let variant = pascal_case(&filename);
        let contents = fs::read_to_string(&path).expect("read svg");
        entries.push((
            variant,
            format!("icons/{filename}"),
            svg_uses_intrinsic_color(&contents),
        ));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut code = String::from(
        "// Generated by build.rs. Do not edit.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, gpui::IntoElement)]\n\
         pub enum IconName {\n",
    );
    for (variant, _, _) in &entries {
        writeln!(code, "    {variant},").unwrap();
    }
    code.push_str("}\n");

    code.push_str("impl IconName {\n    pub const ALL: &'static [Self] = &[\n");
    for (variant, _, _) in &entries {
        writeln!(code, "        Self::{variant},").unwrap();
    }
    code.push_str("    ];\n}\n");

    code.push_str("impl gpui_component::IconNamed for IconName {\n    fn path(self) -> gpui::SharedString {\n        match self {\n");
    for (variant, path, _) in &entries {
        writeln!(code, "            Self::{variant} => {path:?},").unwrap();
    }
    code.push_str("        }.into()\n    }\n\n    fn color_mode(&self) -> gpui_component::IconColorMode {\n        match self {\n");
    for (variant, _, uses_color) in &entries {
        let mode = if *uses_color {
            "gpui_component::IconColorMode::Color"
        } else {
            "gpui_component::IconColorMode::Mono"
        };
        writeln!(code, "            Self::{variant} => {mode},").unwrap();
    }
    code.push_str("        }\n    }\n}\n");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    fs::write(out_dir.join("icon_name.rs"), code).expect("write icon_name.rs");
}
