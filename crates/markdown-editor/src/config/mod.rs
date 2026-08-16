//! Shared user-configuration helpers for app preferences and imported themes.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use directories::ProjectDirs;
use serde_json::{Map, Value};

pub(crate) mod preferences;

pub(crate) use preferences::{EditorSettings, ImagePasteBehavior, read_app_preferences};

/// Cross-platform configuration directories used for imported themes and the
/// small embedded-editor preference file.
#[derive(Debug, Clone)]
pub(crate) struct VelotypeConfigDirs {
    root: PathBuf,
}

impl VelotypeConfigDirs {
    pub(crate) fn from_system() -> anyhow::Result<Self> {
        let dirs = ProjectDirs::from("com", "manyougz", "Velotype")
            .context("failed to resolve the Velotype config directory")?;
        Ok(Self {
            root: dirs.config_dir().to_path_buf(),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn themes_dir(&self) -> PathBuf {
        self.root.join("themes")
    }

    pub(crate) fn app_config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }
}

pub(crate) fn is_supported_config_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("json") || extension.eq_ignore_ascii_case("jsonc")
        })
        .unwrap_or(false)
}

pub(crate) fn read_json_or_jsonc(path: &Path) -> anyhow::Result<Value> {
    if !is_supported_config_file(path) {
        bail!("configuration files must use the .json or .jsonc extension");
    }

    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;
    let parsed = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("jsonc"))
        .unwrap_or(false)
    {
        parse_jsonc_value(&text)?
    } else {
        serde_json::from_str(&text)?
    };
    Ok(parsed)
}

pub(crate) fn parse_jsonc_value(text: &str) -> anyhow::Result<Value> {
    let stripped = strip_jsonc_comments(text)?;
    Ok(serde_json::from_str(&stripped)?)
}

pub(crate) fn strip_jsonc_comments(input: &str) -> anyhow::Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut closed = false;
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            closed = true;
                            break;
                        }
                        previous = next;
                    }
                    if !closed {
                        bail!("unterminated block comment in JSONC file");
                    }
                    continue;
                }
                _ => {}
            }
        }

        output.push(ch);
    }

    Ok(output)
}

pub(crate) fn sanitize_config_file_stem(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;
    for ch in value.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_separator && !output.is_empty() {
                output.push('_');
                last_was_separator = true;
            }
        } else if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            output.push(ch);
            last_was_separator = false;
        }
    }

    let output = output.trim_matches(['_', '.']).to_string();
    if output.is_empty() {
        "custom".into()
    } else {
        output
    }
}

pub(crate) fn prune_empty_json_values(value: &mut Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => {
            items.retain_mut(|item| !prune_empty_json_values(item));
            items.is_empty()
        }
        Value::Object(object) => {
            object.retain(|_, item| !prune_empty_json_values(item));
            object.is_empty()
        }
        Value::Bool(_) | Value::Number(_) => false,
    }
}

pub(crate) fn merge_non_empty_json_values(base: &mut Value, patch: &Value) {
    if is_empty_json_value(patch) {
        return;
    }

    match (base, patch) {
        (Value::Object(base_object), Value::Object(patch_object)) => {
            for (key, patch_value) in patch_object {
                if is_empty_json_value(patch_value) {
                    continue;
                }
                match base_object.get_mut(key) {
                    Some(base_value) => merge_non_empty_json_values(base_value, patch_value),
                    None => {
                        base_object.insert(key.clone(), patch_value.clone());
                    }
                }
            }
        }
        (base_value, patch_value) => {
            *base_value = patch_value.clone();
        }
    }
}

pub(crate) fn object_without_empty_values(mut object: Map<String, Value>) -> Map<String, Value> {
    object.retain(|_, value| !prune_empty_json_values(value));
    object
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.iter().all(is_empty_json_value),
        Value::Object(object) => object.values().all(is_empty_json_value),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_jsonc_value, prune_empty_json_values, sanitize_config_file_stem};
    use serde_json::json;

    #[test]
    fn jsonc_comments_are_stripped_without_touching_strings() {
        let parsed = parse_jsonc_value(
            r#"{
                // comment
                "url": "https://example.com/a//b",
                /* block comment */
                "value": 1
            }"#,
        )
        .unwrap();
        assert_eq!(parsed["url"], "https://example.com/a//b");
        assert_eq!(parsed["value"], 1);
    }

    #[test]
    fn imported_config_values_are_sanitized_and_pruned() {
        assert_eq!(sanitize_config_file_stem(" My Theme! "), "My_Theme");
        let mut value = json!({"name": "", "colors": {"text": "#fff", "empty": null}});
        assert!(!prune_empty_json_values(&mut value));
        assert_eq!(value, json!({"colors": {"text": "#fff"}}));
    }
}
