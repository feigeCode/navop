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
        "port-forwarding" | "port_forwarding" => Some(ImportRecordKind::PortForwarding),
        _ => None,
    }
}
