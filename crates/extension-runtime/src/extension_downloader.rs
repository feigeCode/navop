use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, anyhow};

use crate::extension::manifest::{build_permission_review, load_from_dir};
use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};
use crate::extension_package_layout::{detect_kind_in_package, direct_package_kind, package_root};

mod marketplace;
mod transfer;

static INSTALL_BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub use marketplace::MarketplaceEntry;
#[cfg(test)]
pub use marketplace::MarketplaceManifest;
pub use transfer::{
    DEFAULT_EXTENSION_MANIFEST_URL, DownloadProgress, DownloadProgressCallback,
    GITHUB_EXTENSION_MANIFEST_URL, download_marketplace_entry_to_staging,
    download_marketplace_entry_to_staging_with_progress, fetch_default_manifest_url,
    fetch_manifest_url, fetch_manifest_url_with_fallback, install_marketplace_entry_generic,
    manifest_urls_for_configured_url, manifest_urls_for_configured_url_with_github_fallback,
};

pub fn detect_package_kind(staging_dir: &Path) -> Result<ExtensionKind> {
    detect_kind_in_package(staging_dir)
}

pub fn install_from_staging_generic(
    staging_dir: &Path,
    registry: &ExtensionRegistry,
    requested_kind: Option<ExtensionKind>,
) -> Result<ExtensionSummary> {
    install_from_staging_with_policy(staging_dir, registry, requested_kind, false)
}

pub fn install_from_staging_with_high_risk_permissions(
    staging_dir: &Path,
    registry: &ExtensionRegistry,
    requested_kind: Option<ExtensionKind>,
) -> Result<ExtensionSummary> {
    install_from_staging_with_policy(staging_dir, registry, requested_kind, true)
}

fn install_from_staging_with_policy(
    staging_dir: &Path,
    registry: &ExtensionRegistry,
    requested_kind: Option<ExtensionKind>,
    allow_high_risk_permissions: bool,
) -> Result<ExtensionSummary> {
    let package_root = package_root(staging_dir)?;
    let kind = requested_kind
        .or_else(|| direct_package_kind(&package_root))
        .ok_or_else(|| anyhow!("无法识别扩展包类型: {}", staging_dir.display()))?;
    enforce_install_security_policy(&package_root, kind, allow_high_risk_permissions)?;
    if kind == ExtensionKind::LanguageBundle {
        return install_language_bundle_from_package_root(&package_root, registry);
    }

    let install_name = package_install_name(&package_root, kind)?;
    let provider = registry
        .provider(kind)
        .ok_or_else(|| anyhow!("no provider for {:?}", kind))?;
    let root = registry.root_for(kind);
    std::fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;

    let target_dir = root.join(&install_name);
    let backup_dir = backup_existing_target(&root, &install_name, &target_dir)?;
    if let Err(err) = copy_dir_recursive(&package_root, &target_dir) {
        if let Err(restore_err) = restore_failed_install(&target_dir, backup_dir.as_deref()) {
            return Err(err).context(format!("restore previous extension failed: {restore_err}"));
        }
        return Err(err);
    }
    match provider.install_from_dir(&target_dir) {
        Ok(summary) => {
            remove_install_backup(backup_dir.as_deref());
            Ok(summary)
        }
        Err(err) => {
            if let Err(restore_err) = restore_failed_install(&target_dir, backup_dir.as_deref()) {
                return Err(err)
                    .context(format!("restore previous extension failed: {restore_err}"))
                    .with_context(|| format!("install {:?} from {}", kind, target_dir.display()));
            }
            Err(err).with_context(|| format!("install {:?} from {}", kind, target_dir.display()))
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct LanguageBundleInstallManifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    file_extensions: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LanguageBundleChildManifest {
    #[serde(default)]
    file_extensions: Vec<String>,
}

fn install_language_bundle_from_package_root(
    package_root: &Path,
    registry: &ExtensionRegistry,
) -> Result<ExtensionSummary> {
    let mut manifest = read_language_bundle_install_manifest(package_root)?;
    manifest.languages = validated_bundle_languages(&manifest.languages)?;
    manifest.file_extensions =
        bundle_file_extensions(package_root, &manifest.languages, &manifest.file_extensions)?;

    let language_root = registry.root_for(ExtensionKind::Language);
    let marker_root = registry.root_for(ExtensionKind::LanguageBundle);
    std::fs::create_dir_all(&language_root)
        .with_context(|| format!("create {}", language_root.display()))?;
    std::fs::create_dir_all(&marker_root)
        .with_context(|| format!("create {}", marker_root.display()))?;

    let mut installed = Vec::new();
    if let Err(err) = install_bundle_language_children(
        package_root,
        &language_root,
        &manifest.languages,
        &mut installed,
    )
    .and_then(|_| install_bundle_marker(&marker_root, &manifest, &mut installed))
    {
        restore_bundle_targets(&installed)?;
        return Err(err);
    }
    remove_bundle_backups(&installed);
    Ok(language_bundle_summary(
        &manifest,
        marker_root.join(&manifest.id),
    ))
}

fn read_language_bundle_install_manifest(
    package_root: &Path,
) -> Result<LanguageBundleInstallManifest> {
    let manifest_path = package_root.join("manifest.json");
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: LanguageBundleInstallManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    if manifest.id.trim().is_empty() {
        anyhow::bail!("language bundle manifest missing id");
    }
    if manifest.name.trim().is_empty() {
        anyhow::bail!("language bundle manifest missing name");
    }
    if manifest.version.trim().is_empty() {
        anyhow::bail!("language bundle manifest missing version");
    }
    if manifest.languages.is_empty() {
        anyhow::bail!("language bundle manifest missing languages");
    }
    Ok(manifest)
}

fn validated_bundle_languages(languages: &[String]) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    for language in languages {
        out.insert(validate_install_name(language)?.to_string());
    }
    Ok(out.into_iter().collect())
}

fn bundle_file_extensions(
    package_root: &Path,
    languages: &[String],
    manifest_extensions: &[String],
) -> Result<Vec<String>> {
    let mut extensions = BTreeSet::new();
    for language in languages {
        let child_dir = package_root.join(language);
        ensure_bundle_child_language(&child_dir)?;
        let manifest = read_bundle_child_manifest(&child_dir)?;
        extensions.extend(manifest.file_extensions);
    }
    if extensions.is_empty() {
        extensions.extend(manifest_extensions.iter().cloned());
    }
    Ok(extensions.into_iter().collect())
}

fn ensure_bundle_child_language(child_dir: &Path) -> Result<()> {
    let manifest_path = child_dir.join("manifest.json");
    if !manifest_path.exists() {
        anyhow::bail!("missing language manifest: {}", manifest_path.display());
    }
    let parser_path = child_dir.join("parser.wasm");
    if !parser_path.exists() {
        anyhow::bail!("missing language parser.wasm: {}", parser_path.display());
    }
    Ok(())
}

fn read_bundle_child_manifest(child_dir: &Path) -> Result<LanguageBundleChildManifest> {
    let manifest_path = child_dir.join("manifest.json");
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", manifest_path.display()))
}

fn install_bundle_language_children(
    package_root: &Path,
    language_root: &Path,
    languages: &[String],
    installed: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> Result<()> {
    for language in languages {
        let source = package_root.join(language);
        let target = language_root.join(language);
        let backup = backup_existing_target(language_root, language, &target)?;
        if let Err(err) = copy_dir_recursive(&source, &target) {
            restore_failed_install(&target, backup.as_deref())?;
            return Err(err);
        }
        installed.push((target, backup));
    }
    Ok(())
}

fn install_bundle_marker(
    marker_root: &Path,
    manifest: &LanguageBundleInstallManifest,
    installed: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> Result<()> {
    let marker_dir = marker_root.join(&manifest.id);
    let backup = backup_existing_target(marker_root, &manifest.id, &marker_dir)?;
    if let Err(err) = write_bundle_marker(&marker_dir, manifest) {
        restore_failed_install(&marker_dir, backup.as_deref())?;
        return Err(err);
    }
    installed.push((marker_dir, backup));
    Ok(())
}

fn write_bundle_marker(dir: &Path, manifest: &LanguageBundleInstallManifest) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let mut json = serde_json::to_string_pretty(manifest)?;
    json.push('\n');
    std::fs::write(dir.join("manifest.json"), json)
        .with_context(|| format!("write {}", dir.join("manifest.json").display()))
}

fn restore_bundle_targets(installed: &[(PathBuf, Option<PathBuf>)]) -> Result<()> {
    for (target, backup) in installed.iter().rev() {
        restore_failed_install(target, backup.as_deref())?;
    }
    Ok(())
}

fn remove_bundle_backups(installed: &[(PathBuf, Option<PathBuf>)]) {
    for (_, backup) in installed {
        remove_install_backup(backup.as_deref());
    }
}

fn language_bundle_summary(
    manifest: &LanguageBundleInstallManifest,
    marker_dir: PathBuf,
) -> ExtensionSummary {
    ExtensionSummary::new(
        ExtensionKind::LanguageBundle,
        manifest.id.clone(),
        manifest.version.clone(),
        marker_dir,
    )
    .with_description(format!("{} language bundle", manifest.name))
    .with_file_extensions(manifest.file_extensions.clone())
}

fn backup_existing_target(
    root: &Path,
    install_name: &str,
    target_dir: &Path,
) -> Result<Option<PathBuf>> {
    if !target_dir.exists() {
        return Ok(None);
    }
    let backup_dir = make_backup_dir(root, install_name);
    std::fs::rename(target_dir, &backup_dir).with_context(|| {
        format!(
            "backup existing extension {} -> {}",
            target_dir.display(),
            backup_dir.display()
        )
    })?;
    Ok(Some(backup_dir))
}

fn restore_failed_install(target_dir: &Path, backup_dir: Option<&Path>) -> Result<()> {
    let _ = std::fs::remove_dir_all(target_dir);
    if let Some(backup_dir) = backup_dir {
        std::fs::rename(backup_dir, target_dir).with_context(|| {
            format!(
                "restore existing extension {} -> {}",
                backup_dir.display(),
                target_dir.display()
            )
        })?;
    }
    Ok(())
}

fn remove_install_backup(backup_dir: Option<&Path>) {
    if let Some(backup_dir) = backup_dir {
        if let Err(err) = std::fs::remove_dir_all(backup_dir) {
            tracing::warn!(
                "failed to remove extension install backup {}: {err:?}",
                backup_dir.display()
            );
        }
    }
}

fn make_backup_dir(root: &Path, install_name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let seq = INSTALL_BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    root.join(format!(".{install_name}.install-backup-{now}-{seq}"))
}

pub fn stage_local_tarball(source: &Path) -> Result<PathBuf> {
    let tarball = std::fs::read(source).with_context(|| format!("read {}", source.display()))?;
    let staging = make_staging_dir()?;
    let result = extract_tarball_to(&tarball, &staging).map(|_| staging.clone());
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn package_install_name(staging_dir: &Path, kind: ExtensionKind) -> Result<String> {
    let manifest_file = match kind {
        ExtensionKind::Language => "manifest.json",
        ExtensionKind::LanguageBundle => "manifest.json",
        ExtensionKind::DatabaseDriver => "driver.json",
        ExtensionKind::RemoteDesktopProvider => "remote_desktop_provider.json",
        ExtensionKind::McpHelper => "mcp_helper.json",
        ExtensionKind::AcpAgent => "acp_agent.json",
        ExtensionKind::Composite => "extension.json",
    };
    let manifest_path = staging_dir.join(manifest_file);
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    let field = if kind == ExtensionKind::Language {
        "name"
    } else {
        "id"
    };
    manifest
        .get(field)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(validate_install_name)
        .transpose()?
        .ok_or_else(|| anyhow!("{manifest_file} 缺少 {field} 字段"))
}

fn validate_install_name(name: &str) -> Result<String> {
    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        anyhow::bail!("扩展安装名不能包含路径分隔符或相对路径: {name}");
    }
    Ok(name.to_string())
}

fn enforce_install_security_policy(
    staging_dir: &Path,
    kind: ExtensionKind,
    allow_high_risk_permissions: bool,
) -> Result<()> {
    if kind != ExtensionKind::Composite {
        return Ok(());
    }
    let manifest = load_from_dir(staging_dir)
        .with_context(|| format!("load security metadata from {}", staging_dir.display()))?;
    let review = build_permission_review(&manifest.permissions)?;
    if review.high_risk_count > 0 && !allow_high_risk_permissions {
        anyhow::bail!(
            "扩展 {} 声明了 {} 个高危权限,当前需要权限确认 UI 后才能安装:\n{}",
            manifest.id,
            review.high_risk_count,
            review.summary
        );
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("metadata {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!("refuse to copy symlink from staging: {}", path.display());
        }
        if metadata.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)
                .with_context(|| format!("copy {} -> {}", path.display(), target.display()))?;
        }
    }
    Ok(())
}

fn extract_tarball_to(tarball_bytes: &[u8], target_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("create {}", target_dir.display()))?;
    let decoder = flate2::read::GzDecoder::new(tarball_bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        validate_tar_entry(&entry)?;
        let unpacked = entry
            .unpack_in(target_dir)
            .with_context(|| format!("extract tar.gz to {}", target_dir.display()))?;
        if !unpacked {
            anyhow::bail!("tar entry refused outside target directory");
        }
    }
    Ok(())
}

fn validate_tar_entry<R: std::io::Read>(entry: &tar::Entry<'_, R>) -> Result<()> {
    let path = entry.path().context("read tar entry path")?;
    if path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("tar entry escapes target directory: {}", path.display());
    }
    let entry_type = entry.header().entry_type();
    if entry_type.is_symlink() || entry_type.is_hard_link() {
        anyhow::bail!(
            "tar entry symlink or hard link is not allowed: {}",
            path.display()
        );
    }
    Ok(())
}

fn make_staging_dir() -> Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("onetcli-ext-{}-{now}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create staging dir {}", dir.display()))?;
    Ok(dir)
}
