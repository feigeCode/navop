use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

const DOCUMENT_SUFFIX: &str = ".cditor.json";

pub fn validate_node_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || matches!(name, "." | "..") {
        bail!("name must not be empty, '.' or '..'");
    }
    if name.contains(['/', '\\', '\0']) || name.ends_with(DOCUMENT_SUFFIX) {
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

pub(crate) fn document_file_name(name: &str) -> Result<String> {
    Ok(format!("{}{}", validate_node_name(name)?, DOCUMENT_SUFFIX))
}

pub(crate) fn document_display_name(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(DOCUMENT_SUFFIX)
}
