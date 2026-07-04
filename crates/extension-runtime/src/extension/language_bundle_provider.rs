use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanguageBundleManifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    file_extensions: Vec<String>,
}

pub struct LanguageBundleExtensionProvider;

impl ExtensionProvider for LanguageBundleExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::LanguageBundle
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let manifest = read_manifest(&entry.path())?;
            out.push(summary_from_manifest(manifest, entry.path()));
        }
        out.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(out)
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let manifest = read_manifest(dir)?;
        Ok(summary_from_manifest(manifest, dir.to_path_buf()))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let manifest = read_manifest(dir)?;
        remove_tracked_languages(dir, &manifest.languages)?;
        std::fs::remove_dir_all(dir).with_context(|| format!("remove {}", dir.display()))?;
        Ok(manifest.id)
    }
}

fn read_manifest(dir: &Path) -> Result<LanguageBundleManifest> {
    let manifest_path = dir.join("manifest.json");
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: LanguageBundleManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    if manifest.id.trim().is_empty() {
        return Err(anyhow!("language bundle manifest missing id"));
    }
    if manifest.name.trim().is_empty() {
        return Err(anyhow!("language bundle manifest missing name"));
    }
    Ok(manifest)
}

fn summary_from_manifest(manifest: LanguageBundleManifest, path: PathBuf) -> ExtensionSummary {
    ExtensionSummary::new(
        ExtensionKind::LanguageBundle,
        manifest.id,
        manifest.version,
        path,
    )
    .with_description(format!("{} language bundle", manifest.name))
    .with_file_extensions(manifest.file_extensions)
}

fn remove_tracked_languages(marker_dir: &Path, languages: &[String]) -> Result<()> {
    let languages_root = extensions_root_for_marker(marker_dir)
        .ok_or_else(|| {
            anyhow!(
                "invalid language bundle marker path: {}",
                marker_dir.display()
            )
        })?
        .join(ExtensionKind::Language.dir_name());
    for language in languages {
        let install_name = validate_tracked_language_name(language)?;
        let dir = languages_root.join(install_name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
        }
    }
    Ok(())
}

fn extensions_root_for_marker(marker_dir: &Path) -> Option<PathBuf> {
    marker_dir.parent()?.parent().map(Path::to_path_buf)
}

fn validate_tracked_language_name(name: &str) -> Result<&str> {
    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        anyhow::bail!("language bundle tracked language cannot contain path separators: {name}");
    }
    Ok(name)
}
