use std::path::Path;

use anyhow::{Context, Result, anyhow};
use db::ipc::{IpcDriverManifest, IpcDriverRegistry};

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

pub struct DatabaseDriverExtensionProvider;

impl ExtensionProvider for DatabaseDriverExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::DatabaseDriver
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        Ok(IpcDriverRegistry::load_from_dir(root)
            .map_err(|error| anyhow!("加载数据库驱动列表失败: {error}"))?
            .drivers()
            .iter()
            .map(to_summary)
            .collect())
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let parent = dir
            .parent()
            .ok_or_else(|| anyhow!("driver 目录无父目录: {}", dir.display()))?;
        let registry = IpcDriverRegistry::load_from_dir(parent)
            .map_err(|error| anyhow!("解析 driver manifest 失败: {error}"))?;
        let manifest = registry
            .drivers()
            .iter()
            .find(|manifest| manifest_belongs_to_install_dir(manifest, dir))
            .ok_or_else(|| anyhow!("未在 {} 找到合法的 driver.json", dir.display()))?;
        Ok(to_summary(manifest))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("无效的 driver 目录名: {}", dir.display()))?;
        std::fs::remove_dir_all(dir).with_context(|| format!("删除驱动目录 {}", dir.display()))?;
        Ok(name)
    }
}

fn manifest_belongs_to_install_dir(manifest: &IpcDriverManifest, dir: &Path) -> bool {
    manifest.manifest_dir == dir || manifest.manifest_dir.parent() == Some(dir)
}

fn to_summary(manifest: &IpcDriverManifest) -> ExtensionSummary {
    let description = if manifest.description.trim().is_empty() {
        format!("{} 数据库驱动", manifest.name)
    } else {
        manifest.description.clone()
    };
    let version = if manifest.version.is_empty() {
        "0.0.0".to_string()
    } else {
        manifest.version.clone()
    };

    let mut summary = ExtensionSummary::new(
        ExtensionKind::DatabaseDriver,
        manifest.id.clone(),
        version,
        manifest.manifest_dir.clone(),
    )
    .with_description(description)
    .with_driver_id(manifest.id.clone());

    if !manifest.ui.icon.is_empty() {
        summary = summary.with_icon(manifest.ui.icon.clone());
    }
    if let Some(port) = manifest.ui.default_port {
        summary = summary.with_default_port(port);
    }
    summary
}
