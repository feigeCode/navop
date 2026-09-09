mod registry;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use gpui::SharedString;
use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) use registry::{forget, register_manifest, replace_manifests};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub injection_languages: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub sha256_wasm: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledExtension {
    pub manifest: LanguageManifest,
    wasm_bytes: Vec<u8>,
    highlights: String,
    injections: String,
    locals: String,
    source_path: PathBuf,
}

impl InstalledExtension {
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let manifest = read_manifest_only(dir)?;
        if manifest.name.trim().is_empty() {
            anyhow::bail!("manifest.json at {} has empty `name`", dir.display());
        }
        let wasm_path = dir.join("parser.wasm");
        let wasm_bytes =
            fs::read(&wasm_path).with_context(|| format!("read {}", wasm_path.display()))?;
        if let Some(expected) = &manifest.sha256_wasm {
            verify_sha256(&wasm_bytes, expected)
                .with_context(|| format!("verify sha256 for {}", wasm_path.display()))?;
        }
        Ok(Self {
            manifest,
            wasm_bytes,
            highlights: read_optional(&dir.join("highlights.scm"))?,
            injections: read_optional(&dir.join("injections.scm"))?,
            locals: read_optional(&dir.join("locals.scm"))?,
            source_path: dir.to_path_buf(),
        })
    }

    pub fn register(&self, registry: &LanguageRegistry) -> Result<()> {
        let language = registry::load_wasm_language(&self.manifest.name, &self.wasm_bytes)?;
        let injections = self
            .manifest
            .injection_languages
            .iter()
            .cloned()
            .map(SharedString::from)
            .collect();
        let config = LanguageConfig::new(
            self.manifest.name.clone(),
            language,
            injections,
            &self.highlights,
            &self.injections,
            &self.locals,
        );
        registry.register(&self.manifest.name, &config);
        for extension in &self.manifest.file_extensions {
            registry.register(&normalize_extension(extension), &config);
        }
        register_manifest(self.manifest.clone(), self.source_path.clone(), true);
        Ok(())
    }

    pub fn uninstall(dir: &Path) -> Result<String> {
        let manifest = read_manifest_only(dir)?;
        forget(&manifest.name);
        fs::remove_dir_all(dir).with_context(|| format!("remove {}", dir.display()))?;
        Ok(manifest.name)
    }
}

#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub failed: Vec<(String, String)>,
}

pub fn load_extensions_dir(root: &Path, registry: &LanguageRegistry) -> Result<LoadReport> {
    let mut extensions = HashMap::new();
    let mut report = LoadReport::default();
    for dir in list_subdirs(root)? {
        match InstalledExtension::load_from_dir(&dir) {
            Ok(extension) if !extensions.contains_key(&extension.manifest.name) => {
                extensions.insert(extension.manifest.name.clone(), extension);
            }
            Ok(extension) => report
                .failed
                .push((extension.manifest.name, "duplicate".into())),
            Err(error) => report.failed.push((path_id(&dir), format!("{error:?}"))),
        }
    }
    for name in topological_sort(&extensions)? {
        let extension = &extensions[&name];
        match extension.register(registry) {
            Ok(()) => report.loaded.push(name),
            Err(error) => report.failed.push((name, format!("{error:?}"))),
        }
    }
    Ok(report)
}

pub fn register_extension_manifests_dir(root: &Path) -> Result<LoadReport> {
    let mut report = LoadReport::default();
    let mut manifests = Vec::new();
    let mut seen = HashSet::new();
    for dir in list_subdirs(root)? {
        match read_manifest_only(&dir) {
            Ok(manifest) if seen.insert(manifest.name.clone()) => {
                report.loaded.push(manifest.name.clone());
                manifests.push((manifest, dir));
            }
            Ok(manifest) => report.failed.push((manifest.name, "duplicate".into())),
            Err(error) => report.failed.push((path_id(&dir), format!("{error:?}"))),
        }
    }
    replace_manifests(root, manifests);
    Ok(report)
}

pub fn list_installed(root: &Path) -> Result<Vec<InstalledExtensionSummary>> {
    let mut installed = Vec::new();
    for dir in list_subdirs(root)? {
        match read_manifest_only(&dir) {
            Ok(manifest) => installed.push(InstalledExtensionSummary {
                name: manifest.name,
                version: manifest.version,
                file_extensions: manifest.file_extensions,
                path: dir,
            }),
            Err(error) => {
                tracing::warn!("failed to read manifest for {}: {error:?}", dir.display())
            }
        }
    }
    installed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(installed)
}

pub struct InstalledExtensionSummary {
    pub name: String,
    pub version: String,
    pub file_extensions: Vec<String>,
    pub path: PathBuf,
}

pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let expected = expected.trim().trim_start_matches("sha256:").to_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("invalid sha256: expected 64 hex chars");
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        anyhow::bail!("sha256 mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn read_manifest_only(dir: &Path) -> Result<LanguageManifest> {
    let path = dir.join("manifest.json");
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn read_optional(path: &Path) -> Result<String> {
    if path.exists() {
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
    } else {
        Ok(String::new())
    }
}

fn list_subdirs(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn topological_sort(extensions: &HashMap<String, InstalledExtension>) -> Result<Vec<String>> {
    fn visit(
        name: &str,
        extensions: &HashMap<String, InstalledExtension>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            anyhow::bail!("cyclic dependency in language extensions involving {name}");
        }
        if let Some(extension) = extensions.get(name) {
            for dependency in &extension.manifest.requires {
                visit(dependency, extensions, visiting, visited, order)?;
            }
            order.push(name.to_string());
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        Ok(())
    }

    let mut names = extensions.keys().cloned().collect::<Vec<_>>();
    names.sort();
    let mut order = Vec::new();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for name in names {
        visit(&name, extensions, &mut visiting, &mut visited, &mut order)?;
    }
    Ok(order)
}

fn path_id(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

pub fn load_registered_language(identifier: &str) -> Result<Option<LanguageConfig>> {
    if !registry::load_registered(identifier)? {
        return Ok(None);
    }
    Ok(LanguageRegistry::singleton().language(identifier))
}

pub fn registered_language_name(identifier: &str) -> Option<String> {
    registry::registered_language_name(identifier)
}

pub fn forget_language(name: &str) {
    forget(name);
}
