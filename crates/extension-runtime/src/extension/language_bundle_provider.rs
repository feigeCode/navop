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
    description: String,
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
    let description = if manifest.description.trim().is_empty() {
        format!("{} language bundle", manifest.name)
    } else {
        manifest.description.trim().to_string()
    };
    ExtensionSummary::new(
        ExtensionKind::LanguageBundle,
        manifest.id,
        manifest.version,
        path,
    )
    .with_description(description)
    .with_languages(manifest.languages)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "navop-language-bundle-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn write_bundle(root: &Path, manifest: &str) -> PathBuf {
        let bundle = root.join("tree-sitter-languages");
        std::fs::create_dir_all(&bundle).expect("create bundle dir");
        std::fs::write(bundle.join("manifest.json"), manifest).expect("write manifest");
        bundle
    }

    #[test]
    fn installed_bundle_summary_exposes_languages_and_description() {
        let root = temp_root("summary");
        write_bundle(
            &root,
            r#"{
                "id": "tree-sitter-languages",
                "name": "Tree-sitter Languages",
                "version": "0.1.0",
                "description": "Tree-sitter syntax bundle",
                "languages": ["rust", "typescript"],
                "file_extensions": ["rs", "ts"]
            }"#,
        );

        let summaries = LanguageBundleExtensionProvider
            .list_installed(&root)
            .expect("list installed bundles");

        assert_eq!(1, summaries.len());
        let summary = &summaries[0];
        assert_eq!(ExtensionKind::LanguageBundle, summary.kind);
        assert_eq!("Tree-sitter syntax bundle", summary.description);
        assert_eq!(
            vec!["rust".to_string(), "typescript".to_string()],
            summary.languages
        );
        assert_eq!(
            vec!["rs".to_string(), "ts".to_string()],
            summary.file_extensions
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bundle_without_description_falls_back_to_its_name() {
        let root = temp_root("fallback");
        let bundle = write_bundle(
            &root,
            r#"{
                "id": "tree-sitter-languages",
                "name": "Tree-sitter Languages",
                "version": "0.1.0",
                "languages": ["rust"]
            }"#,
        );

        let manifest = read_manifest(&bundle).expect("read manifest");
        let summary = summary_from_manifest(manifest, bundle);

        assert_eq!("Tree-sitter Languages language bundle", summary.description);
        assert_eq!(vec!["rust".to_string()], summary.languages);

        let _ = std::fs::remove_dir_all(&root);
    }
}
