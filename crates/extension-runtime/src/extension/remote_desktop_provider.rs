use std::path::Path;

use anyhow::{Result, anyhow};
use remote_desktop::{RemoteDesktopProviderManifest, RemoteDesktopProviderRegistry};

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

pub struct RemoteDesktopProviderExtensionProvider;

impl ExtensionProvider for RemoteDesktopProviderExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::RemoteDesktopProvider
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        Ok(RemoteDesktopProviderRegistry::load_from_dir(root)?
            .providers()
            .iter()
            .map(to_summary)
            .collect())
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let manifest = RemoteDesktopProviderRegistry::load_provider_from_dir(dir)?
            .ok_or_else(|| anyhow!("未在 {} 找到 remote_desktop_provider.json", dir.display()))?;
        Ok(to_summary(&manifest))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("无效的远程桌面插件目录名: {}", dir.display()))?;
        std::fs::remove_dir_all(dir)?;
        Ok(name)
    }
}

fn to_summary(manifest: &RemoteDesktopProviderManifest) -> ExtensionSummary {
    let description = if manifest.description.trim().is_empty() {
        format!("{} remote desktop provider", manifest.name)
    } else {
        manifest.description.clone()
    };
    let version = if manifest.version.trim().is_empty() {
        "0.0.0".to_string()
    } else {
        manifest.version.clone()
    };
    let mut summary = ExtensionSummary::new(
        ExtensionKind::RemoteDesktopProvider,
        manifest.id.clone(),
        version,
        manifest.manifest_dir.clone(),
    )
    .with_description(description);
    if !manifest.ui.icon.trim().is_empty() {
        summary = summary.with_icon(manifest.ui.icon.clone());
    }
    if let Some(port) = manifest.ui.default_port {
        summary = summary.with_default_port(port);
    }
    summary
}
