use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use connection_import_protocol::{
    CandidateFile, ImportRecord, ImportRecordKind, ImportScanReport, ImporterAvailability,
    ImporterCapabilities, ImporterDescriptor, Platform,
};
#[cfg(feature = "wasm-components")]
use extension_component::PermissionSet;
#[cfg(feature = "wasm-components")]
use extension_wasm::{ConnectionImportComponentRuntime, ConnectionImportHostState};

use crate::extension::manifest::{Manifest, contributes::ConnectionImporterContrib, load_from_dir};

#[cfg(feature = "wasm-components")]
mod host;

#[cfg(feature = "wasm-components")]
pub(crate) use host::ManifestConnectionImportHost;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualConnectionImportFile {
    pub importer_id: String,
    pub path: PathBuf,
}

impl ManualConnectionImportFile {
    pub fn new(importer_id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            importer_id: importer_id.into(),
            path: path.into(),
        }
    }
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
pub async fn scan_manifest_connection_importers(
    composite_root: &Path,
    importer_ids: &[String],
) -> Result<Vec<ImportScanReport>> {
    let importers = list_manifest_connection_importers(composite_root)?;
    let mut reports = Vec::new();
    for importer in importers
        .into_iter()
        .filter(|importer| importer_ids.contains(&importer.descriptor.id))
    {
        let descriptor_id = importer.descriptor.id.clone();
        let module = importer.extension_dir.join(&importer.module);
        let result = async {
            let runtime = ConnectionImportComponentRuntime::from_file(
                importer.descriptor.id.clone(),
                &module,
            )
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
            runtime.scan(state).await.map_err(anyhow::Error::from)
        }
        .await;
        match result {
            Ok(mut report) => {
                report.importer_id = descriptor_id;
                reports.push(report);
            }
            Err(error) => reports.push(scan_error_report(descriptor_id, error.to_string())),
        }
    }
    Ok(reports)
}

#[cfg(not(feature = "wasm-components"))]
pub async fn scan_manifest_connection_importers(
    _composite_root: &Path,
    _importer_ids: &[String],
) -> Result<Vec<ImportScanReport>> {
    Err(anyhow::anyhow!("wasm component runtime is disabled"))
}

#[cfg(feature = "wasm-components")]
pub async fn preview_manifest_connection_importers(
    composite_root: &Path,
    importer_ids: &[String],
    include_passwords: bool,
) -> Result<Vec<ImportRecord>> {
    preview_manifest_connection_importers_with_files(
        composite_root,
        importer_ids,
        include_passwords,
        &[],
    )
    .await
}

#[cfg(feature = "wasm-components")]
pub async fn preview_manifest_connection_importers_with_files(
    composite_root: &Path,
    importer_ids: &[String],
    include_passwords: bool,
    manual_files: &[ManualConnectionImportFile],
) -> Result<Vec<ImportRecord>> {
    let importers = list_manifest_connection_importers(composite_root)?;
    let mut records = Vec::new();
    for importer in importers
        .into_iter()
        .filter(|importer| importer_ids.contains(&importer.descriptor.id))
    {
        let descriptor_id = importer.descriptor.id.clone();
        let module = importer.extension_dir.join(&importer.module);
        let result = async {
            let runtime = ConnectionImportComponentRuntime::from_file(
                importer.descriptor.id.clone(),
                &module,
            )
            .with_context(|| format!("加载连接导入 Wasm 失败: {}", module.display()))?;
            let (candidates, permissions) =
                connection_import_inputs(&importer, &descriptor_id, manual_files);
            let host = ManifestConnectionImportHost::new(candidates, permissions.clone());
            let state = ConnectionImportHostState::new(
                importer.extension_id,
                importer.descriptor.id,
                host,
                PermissionSet::new(permissions),
            );
            runtime
                .preview(state, include_passwords)
                .await
                .map_err(anyhow::Error::from)
        }
        .await;
        match result {
            Ok(mut preview) => {
                for record in &mut preview {
                    record.importer_id = descriptor_id.clone();
                }
                records.extend(preview);
            }
            Err(error) => {
                tracing::warn!(
                    importer_id = %descriptor_id,
                    "connection import preview failed: {error:?}"
                );
            }
        }
    }
    Ok(records)
}

fn connection_import_inputs(
    importer: &ManifestConnectionImporter,
    descriptor_id: &str,
    manual_files: &[ManualConnectionImportFile],
) -> (Vec<CandidateFile>, Vec<String>) {
    let mut candidates = importer.candidates.clone();
    let mut permissions = importer.permissions.clone();
    let (manual_candidates, manual_permissions) =
        manual_file_candidates(descriptor_id, manual_files);
    candidates.extend(manual_candidates);
    permissions.extend(manual_permissions);
    (candidates, permissions)
}

pub(crate) fn manual_file_candidates(
    importer_id: &str,
    manual_files: &[ManualConnectionImportFile],
) -> (Vec<CandidateFile>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut permissions = Vec::new();
    for (index, file) in manual_files
        .iter()
        .filter(|file| file.importer_id == importer_id)
        .enumerate()
    {
        let path = file.path.to_string_lossy().to_string();
        candidates.push(CandidateFile {
            id: format!("manual-file-{index}"),
            platform: None,
            path: path.clone(),
        });
        permissions.push(format!("fs:read:{path}"));
    }
    (candidates, permissions)
}

fn scan_error_report(importer_id: String, message: String) -> ImportScanReport {
    ImportScanReport {
        importer_id,
        availability: ImporterAvailability::Error { message },
        discovered_files: Vec::new(),
        warnings: Vec::new(),
    }
}

#[cfg(not(feature = "wasm-components"))]
pub async fn preview_manifest_connection_importers(
    _composite_root: &Path,
    _importer_ids: &[String],
    _include_passwords: bool,
) -> Result<Vec<ImportRecord>> {
    Err(anyhow::anyhow!("wasm component runtime is disabled"))
}

#[cfg(not(feature = "wasm-components"))]
pub async fn preview_manifest_connection_importers_with_files(
    _composite_root: &Path,
    _importer_ids: &[String],
    _include_passwords: bool,
    _manual_files: &[ManualConnectionImportFile],
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
    let manual_file_pick_prompt = manual_file_pick_prompt(contrib);
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
            supports_manual_file_pick: manual_file_pick_prompt.is_some(),
            manual_file_pick_prompt,
            supports_incremental_preview: false,
        },
    }
}

fn manual_file_pick_prompt(contrib: &ConnectionImporterContrib) -> Option<String> {
    contrib
        .manual_file_pick
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .map(str::to_string)
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
        "port-forwarding" | "port_forwarding" => Some(ImportRecordKind::PortForwarding),
        _ => None,
    }
}
