#[cfg(any(feature = "wasm-components", test))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use connection_import_protocol::{
    CandidateFile, ImportRecord, ImportRecordKind, ImporterCapabilities, ImporterDescriptor,
    Platform,
};
#[cfg(feature = "wasm-components")]
use connection_import_protocol::{DirectoryEntry, HostAccessError, SecretQuery, SecretResult};
#[cfg(feature = "wasm-components")]
use extension_component::{CandidateFileAccess, ExtensionConnectionImportHost, PermissionSet};
#[cfg(feature = "wasm-components")]
use extension_wasm::{ConnectionImportComponentRuntime, ConnectionImportHostState};

use crate::extension::manifest::{Manifest, contributes::ConnectionImporterContrib, load_from_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestConnectionImporter {
    pub extension_id: String,
    pub extension_dir: PathBuf,
    pub runtime_id: String,
    pub module: String,
    pub candidates: Vec<CandidateFile>,
    pub permissions: Vec<String>,
    pub descriptor: ImporterDescriptor,
}

pub fn list_manifest_connection_importers(
    composite_root: &Path,
) -> Result<Vec<ManifestConnectionImporter>> {
    if !composite_root.exists() {
        return Ok(Vec::new());
    }

    let mut importers = Vec::new();
    for entry in std::fs::read_dir(composite_root)
        .with_context(|| format!("读取 composite 扩展目录 {}", composite_root.display()))?
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let manifest = match load_from_dir(&entry.path()) {
            Ok(manifest) => manifest,
            Err(error) => {
                tracing::warn!("connection import manifest load failed: {error:?}");
                continue;
            }
        };
        importers.extend(manifest_importers(&manifest)?);
    }
    Ok(importers)
}

fn manifest_importers(manifest: &Manifest) -> Result<Vec<ManifestConnectionImporter>> {
    manifest
        .contributes
        .connection_importers
        .iter()
        .map(|contrib| manifest_importer(manifest, contrib))
        .collect()
}

fn manifest_importer(
    manifest: &Manifest,
    contrib: &ConnectionImporterContrib,
) -> Result<ManifestConnectionImporter> {
    let runtime_id = runtime_id(manifest, contrib);
    let module = manifest
        .runtime
        .wasm
        .iter()
        .find(|runtime| runtime.id == runtime_id)
        .map(|runtime| runtime.module.clone())
        .with_context(|| format!("connection importer runtime not found: {runtime_id}"))?;

    Ok(ManifestConnectionImporter {
        extension_id: manifest.id.clone(),
        extension_dir: manifest.manifest_dir.clone(),
        runtime_id,
        module,
        candidates: contrib
            .candidate_files
            .iter()
            .map(|candidate| CandidateFile {
                id: candidate.id.clone(),
                platform: parse_optional_platform(&candidate.platform),
                path: candidate.path.clone(),
            })
            .collect(),
        permissions: manifest.permissions.clone(),
        descriptor: descriptor(manifest, contrib),
    })
}

#[cfg(feature = "wasm-components")]
pub async fn preview_manifest_connection_importers(
    composite_root: &Path,
    importer_ids: &[String],
    include_passwords: bool,
) -> Result<Vec<ImportRecord>> {
    let importers = list_manifest_connection_importers(composite_root)?;
    let mut records = Vec::new();
    for importer in importers
        .into_iter()
        .filter(|importer| importer_ids.contains(&importer.descriptor.id))
    {
        let module = importer.extension_dir.join(&importer.module);
        let runtime =
            ConnectionImportComponentRuntime::from_file(importer.descriptor.id.clone(), &module)
                .with_context(|| format!("加载连接导入 Wasm 失败: {}", module.display()))?;
        let host = ManifestConnectionImportHost::new(
            importer.candidates.clone(),
            importer.permissions.clone(),
        );
        let state = ConnectionImportHostState::new(
            importer.extension_id,
            importer.descriptor.id,
            host,
            PermissionSet::new(importer.permissions),
        );
        records.extend(runtime.preview(state, include_passwords).await?);
    }
    Ok(records)
}

#[cfg(not(feature = "wasm-components"))]
pub async fn preview_manifest_connection_importers(
    _composite_root: &Path,
    _importer_ids: &[String],
    _include_passwords: bool,
) -> Result<Vec<ImportRecord>> {
    Err(anyhow::anyhow!("wasm component runtime is disabled"))
}

fn runtime_id(manifest: &Manifest, contrib: &ConnectionImporterContrib) -> String {
    if !contrib.runtime_id.is_empty() {
        return contrib.runtime_id.clone();
    }
    manifest
        .runtime
        .wasm
        .first()
        .map(|runtime| runtime.id.clone())
        .unwrap_or_default()
}

fn descriptor(manifest: &Manifest, contrib: &ConnectionImporterContrib) -> ImporterDescriptor {
    ImporterDescriptor {
        id: format!("{}/{}", manifest.id, contrib.id),
        display_name: contrib.display_name.clone(),
        description: contrib.description.clone(),
        icon: contrib.icon.clone(),
        vendor: (!manifest.publisher.is_empty()).then(|| manifest.publisher.clone()),
        supported_platforms: contrib
            .platforms
            .iter()
            .filter_map(|platform| parse_platform(platform))
            .collect(),
        output_kinds: contrib
            .output_kinds
            .iter()
            .filter_map(|kind| parse_output_kind(kind))
            .collect(),
        capabilities: ImporterCapabilities {
            supports_scan: true,
            supports_password_import: false,
            supports_manual_file_pick: !contrib.candidate_files.is_empty(),
            supports_incremental_preview: false,
        },
    }
}

fn parse_platform(value: &str) -> Option<Platform> {
    match value {
        "macos" => Some(Platform::Macos),
        "windows" => Some(Platform::Windows),
        "linux" => Some(Platform::Linux),
        _ => None,
    }
}

fn parse_optional_platform(value: &str) -> Option<Platform> {
    if value.is_empty() {
        None
    } else {
        parse_platform(value)
    }
}

fn parse_output_kind(value: &str) -> Option<ImportRecordKind> {
    match value {
        "database" => Some(ImportRecordKind::Database),
        "ssh" => Some(ImportRecordKind::Ssh),
        _ => None,
    }
}

#[cfg(feature = "wasm-components")]
pub(crate) struct ManifestConnectionImportHost {
    candidates: Vec<CandidateFile>,
    permissions: PermissionSet,
}

#[cfg(feature = "wasm-components")]
impl ManifestConnectionImportHost {
    pub(crate) fn new<I, S>(candidates: Vec<CandidateFile>, permissions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            candidates,
            permissions: PermissionSet::new(permissions),
        }
    }

    fn candidate_access(&self) -> CandidateFileAccess {
        CandidateFileAccess::new(self.candidates.clone(), self.permissions.clone())
    }
}

#[cfg(feature = "wasm-components")]
impl ExtensionConnectionImportHost for ManifestConnectionImportHost {
    fn current_platform(&self) -> Platform {
        current_platform()
    }

    fn list_candidate_files(&self, _importer_id: &str) -> Vec<CandidateFile> {
        self.candidates.clone()
    }

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError> {
        let candidate = self.candidate_access().candidate(candidate_id)?.clone();
        std::fs::read(expand_connection_import_path(&candidate.path)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                HostAccessError::NotFound(candidate.path)
            } else {
                HostAccessError::Io(error.to_string())
            }
        })
    }

    fn read_directory(&self, candidate_id: &str) -> Result<Vec<DirectoryEntry>, HostAccessError> {
        let candidate = self.candidate_access().candidate(candidate_id)?.clone();
        let entries = std::fs::read_dir(expand_connection_import_path(&candidate.path))
            .map_err(|error| HostAccessError::Io(error.to_string()))?;
        let mut out = Vec::new();
        for entry in entries.flatten() {
            out.push(DirectoryEntry {
                candidate_id: candidate_id.to_string(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false),
            });
        }
        Ok(out)
    }

    fn read_secret(&self, _query: SecretQuery) -> SecretResult {
        SecretResult::Unsupported
    }

    fn log(&self, level: &str, message: &str) {
        tracing::debug!(target: "connection_import", level, message);
    }
}

#[cfg(feature = "wasm-components")]
fn current_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Macos
    }
}

#[cfg(any(feature = "wasm-components", test))]
fn expand_connection_import_path(path: &str) -> PathBuf {
    expand_connection_import_path_with(path, |name| std::env::var_os(name))
}

#[cfg(any(feature = "wasm-components", test))]
fn expand_connection_import_path_with<F>(path: &str, env_var: F) -> PathBuf
where
    F: FnMut(&str) -> Option<OsString>,
{
    expand_tilde_or_env(path, env_var)
}

#[cfg(any(feature = "wasm-components", test))]
fn expand_tilde_or_env<F>(path: &str, mut env_var: F) -> PathBuf
where
    F: FnMut(&str) -> Option<OsString>,
{
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = env_var("HOME") {
            return join_expanded_path(home, rest);
        }
    }
    if let Some((name, rest)) = windows_env_prefix(path) {
        if let Some(value) = env_var(name) {
            return join_expanded_path(value, rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(any(feature = "wasm-components", test))]
fn windows_env_prefix(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix('%')?;
    let end = rest.find('%')?;
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    let tail = &rest[end + 1..];
    if !tail.is_empty() && !tail.starts_with('/') && !tail.starts_with('\\') {
        return None;
    }
    Some((name, tail))
}

#[cfg(any(feature = "wasm-components", test))]
fn join_expanded_path(base: OsString, rest: &str) -> PathBuf {
    let mut path = PathBuf::from(base);
    let rest = rest.trim_start_matches(['/', '\\']);
    if !rest.is_empty() {
        path.push(rest);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::expand_connection_import_path_with;

    #[test]
    fn expands_windows_style_env_prefix_for_connection_import_candidates() {
        let path = expand_connection_import_path_with("%APPDATA%/DBeaverData/workspace6", |name| {
            (name == "APPDATA").then(|| "C:\\Users\\me\\AppData\\Roaming".into())
        });

        assert_eq!(
            std::path::PathBuf::from("C:\\Users\\me\\AppData\\Roaming")
                .join("DBeaverData/workspace6"),
            path
        );
    }
}
