use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

pub(crate) const MARKDOWN_SUFFIX: &str = ".md";

pub fn validate_node_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || matches!(name, "." | "..") {
        bail!("name must not be empty, '.' or '..'");
    }
    if name.contains(['/', '\\', '\0']) || name.ends_with(MARKDOWN_SUFFIX) {
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

pub(crate) fn document_file_name(name: &str, _format: crate::DocumentFormat) -> Result<String> {
    let suffix = MARKDOWN_SUFFIX;
    let name = name.trim();
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
    file_name
        .strip_suffix(MARKDOWN_SUFFIX)
        .map(|name| (name, crate::DocumentFormat::Markdown))
}

pub(crate) fn remap_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    path.strip_prefix(old)
        .map(|suffix| new.join(suffix))
        .unwrap_or_else(|_| path.to_path_buf())
}
