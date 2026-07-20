use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::extension::{
    ExtensionKind, ExtensionProvider, ExtensionSummary, is_active_install_dir_name,
    manifest::{
        HostApiVersions, Manifest, ManifestError, current_host_version, load_and_check,
        load_from_dir,
    },
};

pub struct CompositeExtensionProvider;

impl ExtensionProvider for CompositeExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Composite
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let host_version = current_host_version();
        let host_apis = HostApiVersions::current();
        let mut summaries = Vec::new();

        for entry in std::fs::read_dir(root)
            .with_context(|| format!("读取 composite 目录 {}", root.display()))?
        {
            let Ok(entry) = entry else {
                continue;
            };
            if !is_candidate_composite_dir(&entry) {
                continue;
            }
            match load_and_check(&entry.path(), &host_version, &host_apis) {
                Ok(manifest) => summaries.push(to_summary(&manifest)),
                Err(ManifestError::NotFound(_)) => {}
                Err(error) => {
                    tracing::warn!(
                        "composite 扩展 {} 加载失败: {error:?}",
                        entry.path().display()
                    );
                }
            }
        }
        Ok(summaries)
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let host_version = current_host_version();
        let host_apis = HostApiVersions::current();
        let manifest = load_and_check(dir, &host_version, &host_apis)
            .map_err(|error| anyhow!("composite 扩展 {} 加载失败: {error}", dir.display()))?;
        Ok(to_summary(&manifest))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let name = load_from_dir(dir)
            .map(|manifest| manifest.id)
            .unwrap_or_else(|_| {
                dir.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<unknown>")
                    .to_string()
            });
        std::fs::remove_dir_all(dir)
            .with_context(|| format!("删除 composite 扩展目录 {}", dir.display()))?;
        Ok(name)
    }
}

fn is_candidate_composite_dir(entry: &std::fs::DirEntry) -> bool {
    let Ok(file_type) = entry.file_type() else {
        return false;
    };
    file_type.is_dir() && is_active_install_dir_name(&entry.file_name())
}

fn to_summary(manifest: &Manifest) -> ExtensionSummary {
    let description = if !manifest.description.is_empty() {
        manifest.description.clone()
    } else if manifest.contributes.total_count() > 0 {
        format!("{} 项贡献点", manifest.contributes.total_count())
    } else {
        format!("{} 扩展", manifest.name)
    };

    let mut summary = ExtensionSummary::new(
        ExtensionKind::Composite,
        manifest.id.clone(),
        manifest.version.clone(),
        manifest.manifest_dir.clone(),
    )
    .with_description(description);

    if !manifest.icon.is_empty() {
        summary = summary.with_icon(manifest.icon.clone());
    }
    summary
}
