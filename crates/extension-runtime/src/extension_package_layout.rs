use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};

use crate::extension::ExtensionKind;

pub(crate) fn detect_kind_in_package(staging_dir: &Path) -> Result<ExtensionKind> {
    let root = package_root(staging_dir)?;
    direct_package_kind(&root).ok_or_else(|| unrecognized_package_kind(staging_dir))
}

pub(crate) fn package_root(staging_dir: &Path) -> Result<PathBuf> {
    if direct_package_kind(staging_dir).is_some() {
        return Ok(staging_dir.to_path_buf());
    }
    if let Some(root) = single_wrapped_package_root(staging_dir)? {
        return Ok(root);
    }
    Err(unrecognized_package_kind(staging_dir))
}

pub(crate) fn direct_package_kind(dir: &Path) -> Option<ExtensionKind> {
    if dir.join("extension.json").exists() {
        return Some(ExtensionKind::Composite);
    }
    if dir.join("driver.json").exists() {
        return Some(ExtensionKind::DatabaseDriver);
    }
    if dir.join("remote_desktop_provider.json").exists() {
        return Some(ExtensionKind::RemoteDesktopProvider);
    }
    if dir.join("manifest.json").exists() && dir.join("parser.wasm").exists() {
        return Some(ExtensionKind::Language);
    }
    None
}

fn single_wrapped_package_root(staging_dir: &Path) -> Result<Option<PathBuf>> {
    let Some(dir) = single_significant_dir(staging_dir)? else {
        return Ok(None);
    };
    if direct_package_kind(&dir).is_some() {
        return Ok(Some(dir));
    }
    Ok(None)
}

fn single_significant_dir(staging_dir: &Path) -> Result<Option<PathBuf>> {
    let mut found_dir = None;
    for entry in std::fs::read_dir(staging_dir)? {
        let entry = entry?;
        if ignored_archive_metadata(&entry.file_name()) {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            return Ok(None);
        }
        if found_dir.replace(entry.path()).is_some() {
            return Ok(None);
        }
    }
    Ok(found_dir)
}

fn ignored_archive_metadata(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name == ".DS_Store" || name == "__MACOSX" || name.starts_with("._")
}

fn unrecognized_package_kind(staging_dir: &Path) -> anyhow::Error {
    anyhow!(
        "无法识别扩展包类型,缺少 extension.json / driver.json / remote_desktop_provider.json / manifest.json+parser.wasm: {}",
        staging_dir.display()
    )
}
