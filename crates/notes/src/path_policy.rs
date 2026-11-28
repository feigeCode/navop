use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

pub(crate) const MARKDOWN_SUFFIX: &str = ".md";

/// 笔记文档的后缀，按格式区分（#429：`.md` 与 `.txt`）。
const DOCUMENT_SUFFIXES: [&str; 2] = [".md", ".txt"];

fn has_document_suffix(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    DOCUMENT_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

pub fn validate_node_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || matches!(name, "." | "..") {
        bail!("name must not be empty, '.' or '..'");
    }
    if name.contains(['/', '\\', '\0']) || has_document_suffix(name) {
        bail!("name contains a path separator or reserved suffix");
    }
    Ok(name)
}

pub(crate) fn validate_relative_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!("absolute paths are not allowed");
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => clean.push(value),
            Component::CurDir if clean.as_os_str().is_empty() => {}
            _ => bail!("path must remain inside the notes root"),
        }
    }
    Ok(clean)
}

pub(crate) fn document_file_name(name: &str, format: crate::DocumentFormat) -> Result<String> {
    let suffix = format.suffix();
    let name = name.trim();
    // 只剥掉目标格式自己的后缀：用户把「a.md」当文本文档创建时按非法后缀报错，
    // 而不是静默改写成 a.txt。
    let display_name = strip_suffix_ignore_ascii_case(name, suffix).unwrap_or(name);
    Ok(format!("{}{suffix}", validate_node_name(display_name)?))
}

fn strip_suffix_ignore_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let prefix_len = value.len().checked_sub(suffix.len())?;
    value[prefix_len..]
        .eq_ignore_ascii_case(suffix)
        .then(|| &value[..prefix_len])
}

pub(crate) fn document_display_name(file_name: &str) -> Option<(&str, crate::DocumentFormat)> {
    let formats = [
        (crate::DocumentFormat::Markdown, MARKDOWN_SUFFIX),
        (crate::DocumentFormat::Text, ".txt"),
    ];
    formats
        .into_iter()
        .find_map(|(format, suffix)| file_name.strip_suffix(suffix).map(|name| (name, format)))
}

pub(crate) fn remap_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    path.strip_prefix(old)
        .map(|suffix| new.join(suffix))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentFormat;

    #[test]
    fn document_file_name_uses_the_suffix_of_the_format() {
        assert_eq!(
            "Guide.md",
            document_file_name("Guide", DocumentFormat::Markdown).unwrap()
        );
        assert_eq!(
            "Guide.txt",
            document_file_name("Guide", DocumentFormat::Text).unwrap()
        );
        // 用户把后缀一起输进来时不重复拼接。
        assert_eq!(
            "Guide.txt",
            document_file_name("Guide.txt", DocumentFormat::Text).unwrap()
        );
        assert_eq!(
            "Guide.md",
            document_file_name("Guide.MD", DocumentFormat::Markdown).unwrap()
        );
    }

    #[test]
    fn document_file_name_rejects_a_suffix_of_another_format() {
        assert!(document_file_name("Guide.md", DocumentFormat::Text).is_err());
        assert!(document_file_name("Guide.txt", DocumentFormat::Markdown).is_err());
    }

    #[test]
    fn document_display_name_recognizes_both_formats() {
        assert_eq!(
            Some(("Guide", DocumentFormat::Markdown)),
            document_display_name("Guide.md")
        );
        assert_eq!(
            Some(("Guide", DocumentFormat::Text)),
            document_display_name("Guide.txt")
        );
        assert_eq!(None, document_display_name("Guide.docx"));
    }
}
