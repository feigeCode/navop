//! 内置编辑器的文档格式化与校验（issue #621 的第一个切片）。
//!
//! 只做「结构化文档 ↔ 文本」这一段：按扩展名识别 JSON / YAML，解析成功时输出
//! 规范化的文本，解析失败时把错误位置（行、列）带回去给 UI 展示。键名补全、
//! WASM 语言扩展等留给后续切片。

use std::path::Path;

/// 支持格式化的文档类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Json,
    Yaml,
}

impl DocumentFormat {
    /// 按文件名（或路径）识别格式；`.jsonc` 里允许注释，直接解析会失败，故不识别。
    pub fn from_file_name(file_name: &str) -> Option<Self> {
        let extension = Path::new(file_name).extension()?.to_str()?;
        match extension.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "yml" | "yaml" => Some(Self::Yaml),
            _ => None,
        }
    }

    /// 展示用的语言名，用于状态提示。
    pub fn language_name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Yaml => "YAML",
        }
    }
}

/// 格式化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatOutcome {
    /// 已重新排版，内容是新的文本。
    Formatted(String),
    /// 输入已经是规范化结果，不需要改动。
    Unchanged,
}

/// 解析失败的细节，行列均为 1 起始。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

/// 按扩展名格式化文档。
///
/// 返回 `None` 表示该文件类型不支持格式化（调用方据此提示用户）。
pub fn format_document(file_name: &str, text: &str) -> Option<Result<FormatOutcome, FormatError>> {
    let format = DocumentFormat::from_file_name(file_name)?;
    Some(match format {
        DocumentFormat::Json => format_json(text),
        DocumentFormat::Yaml => format_yaml(text),
    })
}

fn format_json(text: &str) -> Result<FormatOutcome, FormatError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| FormatError {
        line: Some(error.line()),
        column: Some(error.column()),
        message: error.to_string(),
    })?;
    let formatted = serde_json::to_string_pretty(&value).map_err(|error| FormatError {
        line: None,
        column: None,
        message: error.to_string(),
    })?;
    // 统一带结尾换行，避免保存后 diff 噪声。
    let formatted = format!("{formatted}\n");
    Ok(format_outcome(text, &formatted))
}

fn format_yaml(text: &str) -> Result<FormatOutcome, FormatError> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|error| {
        let location = error.location();
        FormatError {
            line: location.as_ref().map(|location| location.line()),
            column: location.as_ref().map(|location| location.column()),
            message: error.to_string(),
        }
    })?;
    let formatted = serde_yaml::to_string(&value).map_err(|error| FormatError {
        line: None,
        column: None,
        message: error.to_string(),
    })?;
    Ok(format_outcome(text, &formatted))
}

/// 只比较含义上的文本，忽略末尾换行差异：编辑器里常见「少一个结尾换行」。
fn format_outcome(original: &str, formatted: &str) -> FormatOutcome {
    if original.trim_end() == formatted.trim_end() {
        FormatOutcome::Unchanged
    } else {
        FormatOutcome::Formatted(formatted.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_documents_are_pretty_printed() {
        let outcome = format_document("config.json", r#"{"b":1,"a":[1,2]}"#)
            .expect("json is supported")
            .expect("valid json formats");

        let FormatOutcome::Formatted(text) = outcome else {
            panic!("compact json should be reformatted");
        };
        assert_eq!(text, "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}\n");
    }

    #[test]
    fn already_formatted_json_reports_unchanged() {
        let text = "{\n  \"a\": 1\n}\n";

        assert_eq!(
            format_document("config.json", text).expect("json is supported"),
            Ok(FormatOutcome::Unchanged)
        );
        // 末尾换行差异不应触发重新格式化。
        assert_eq!(
            format_document("config.json", "{\n  \"a\": 1\n}").expect("json is supported"),
            Ok(FormatOutcome::Unchanged)
        );
    }

    #[test]
    fn broken_json_reports_line_and_column() {
        let error = format_document("config.json", "{\n  \"a\": 1,\n  \"b\" 2\n}")
            .expect("json is supported")
            .expect_err("missing colon is invalid");

        assert_eq!(error.line, Some(3));
        assert_eq!(error.column, Some(7));
        assert!(error.message.contains("expected `:`"), "{}", error.message);
    }

    #[test]
    fn yaml_documents_keep_mapping_order_and_expand_flow_sequences() {
        let outcome = format_document("config.yml", "a: 1\nb: [x, y]\n")
            .expect("yaml is supported")
            .expect("valid yaml formats");

        let FormatOutcome::Formatted(text) = outcome else {
            panic!("flow sequence should be expanded");
        };
        let round_trip: serde_yaml::Value = serde_yaml::from_str(&text).expect("output parses");
        let original: serde_yaml::Value =
            serde_yaml::from_str("a: 1\nb: [x, y]\n").expect("input parses");
        assert_eq!(round_trip, original);
        assert!(text.starts_with("a: 1\n"), "{text}");
        assert!(text.find('a').unwrap() < text.find('b').unwrap(), "{text}");
    }

    #[test]
    fn broken_yaml_reports_line_and_column() {
        let error = format_document("config.yaml", "a: 1\n  b: 2\n")
            .expect("yaml is supported")
            .expect_err("bad indentation is invalid");

        assert!(error.line.is_some(), "{error:?}");
        assert!(error.column.is_some(), "{error:?}");
        assert!(!error.message.is_empty());
    }

    #[test]
    fn unsupported_extensions_are_ignored() {
        assert_eq!(DocumentFormat::from_file_name("notes.txt"), None);
        assert_eq!(DocumentFormat::from_file_name("config.jsonc"), None);
        assert_eq!(format_document("notes.txt", "{}"), None);
        assert_eq!(
            DocumentFormat::from_file_name("/tmp/a/b/CONFIG.YAML"),
            Some(DocumentFormat::Yaml)
        );
    }
}
