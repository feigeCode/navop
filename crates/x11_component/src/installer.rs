use std::path::{Path, PathBuf};

use rand::{RngCore as _, rngs::OsRng};

use crate::bundle::EmbeddedArchive;
use crate::lock::ComponentLock;
use crate::manifest::{Artifact, ComponentManifest};
use crate::paths::{component_root, prune_versions_preserving, read_installed, write_current};
use crate::process::active_versions;
use crate::rollout::should_use_embedded;
use crate::state::{active_version, disable_embedded, update_status};
use crate::{InstallConfig, X11ComponentError, X11ServerMode};

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug)]
pub(crate) struct InstalledComponent {
    pub version: String,
    pub executable: PathBuf,
    pub min_vulkan: String,
}

/// 显式安装内嵌组件，始终强制 SHA256 与 Ed25519 校验。
pub async fn install_with_config(config: InstallConfig) -> Result<String, X11ComponentError> {
    install_with_config_for_mode(config, X11ServerMode::Embedded).await
}

/// 供解析器传递 Auto 灰度语义；显式 Embedded 不受灰度比例限制。
pub async fn install_with_config_for_mode(
    config: InstallConfig,
    mode: X11ServerMode,
) -> Result<String, X11ComponentError> {
    prepare(config, mode)
        .await
        .map(|component| component.version)
}

/// 使用同一份内嵌归档重新安装；不存在任何运行时网络回退。
pub async fn reinstall_with_config(config: InstallConfig) -> Result<String, X11ComponentError> {
    let _guard = INSTALL_LOCK.lock().await;
    prepare_locked(config, InstallPurpose::Repair)
        .await
        .map(|component| component.version)
}

pub(crate) async fn prepare(
    config: InstallConfig,
    mode: X11ServerMode,
) -> Result<InstalledComponent, X11ComponentError> {
    let _guard = INSTALL_LOCK.lock().await;
    prepare_locked(config, InstallPurpose::Resolve(mode)).await
}

async fn prepare_locked(
    config: InstallConfig,
    purpose: InstallPurpose,
) -> Result<InstalledComponent, X11ComponentError> {
    let archive = crate::bundle::archive()
        .ok_or_else(|| X11ComponentError::Config("当前构建未内嵌 X11 组件，无法离线安装".into()))?;
    install_embedded_manifest(&config, purpose, crate::bundle::PINNED_MANIFEST, archive).await
}

enum InstallPurpose {
    Resolve(X11ServerMode),
    Repair,
}

impl InstallPurpose {
    fn mode(&self) -> X11ServerMode {
        match self {
            Self::Resolve(mode) => *mode,
            Self::Repair => X11ServerMode::Embedded,
        }
    }

    fn force(&self) -> bool {
        matches!(self, Self::Repair)
    }
}

async fn install_embedded_manifest(
    config: &InstallConfig,
    purpose: InstallPurpose,
    manifest_text: &str,
    archive: EmbeddedArchive,
) -> Result<InstalledComponent, X11ComponentError> {
    if archive.target != config.target {
        return Err(X11ComponentError::MissingArtifact(format!(
            "内嵌组件目标 {} 与当前目标 {} 不一致",
            archive.target, config.target
        )));
    }
    let manifest = ComponentManifest::parse(manifest_text, &config.navop_version)?;
    install_selected_artifact(config, purpose, &manifest, archive).await
}

async fn install_selected_artifact(
    config: &InstallConfig,
    purpose: InstallPurpose,
    manifest: &ComponentManifest,
    archive: EmbeddedArchive,
) -> Result<InstalledComponent, X11ComponentError> {
    if config.public_key_base64.is_empty() {
        return Err(X11ComponentError::Config(
            "X11 组件发布公钥尚未配置，已 fail-closed".into(),
        ));
    }
    let artifact = manifest.artifact(&config.target)?;
    let root = component_root(&config.data_dir);
    if !should_use_embedded(&root, purpose.mode(), manifest.rollout_percent())? {
        return Err(X11ComponentError::EmbeddedDisabled(
            "当前持久灰度编号未纳入 Auto 发布范围".into(),
        ));
    }
    if !purpose.force() {
        if let Some(component) = already_installed(&root, &artifact)? {
            return Ok(component);
        }
    }
    let destination = root.join("versions").join(&artifact.version);
    if destination.exists() {
        let _lock = ComponentLock::acquire_async(root.clone())
            .await
            .map_err(install_error)?;
        ensure_replacement_allowed(&root, &artifact.version)?;
    }
    let staging = StagingDirectory::new(&root)?;
    let result = embedded_install(config, &artifact, &staging.path, archive.bytes).await;
    preserve_untrusted_disable_reason(&result);
    if result.is_ok() {
        staging.disarm();
    } else {
        staging.cleanup().await;
    }
    result
}

fn preserve_untrusted_disable_reason(result: &Result<InstalledComponent, X11ComponentError>) {
    let reason = match result {
        Err(
            error @ (X11ComponentError::SignatureInvalid(_)
            | X11ComponentError::Sha256Mismatch { .. }),
        ) => Some(error.to_string()),
        _ => None,
    };
    if let Some(reason) = reason {
        let reason = disable_embedded(reason);
        update_status(crate::X11ForwardingStatus::Disabled {
            reason: format!("{reason}；本次运行禁用内置"),
        });
    }
}

fn already_installed(
    root: &Path,
    artifact: &Artifact,
) -> Result<Option<InstalledComponent>, X11ComponentError> {
    let executable = root
        .join("versions")
        .join(&artifact.version)
        .join(artifact.executable_relative_path());
    if !executable.is_file() {
        return Ok(None);
    }
    write_current(root, &artifact.version)?;
    let installed = read_installed(root).filter(|version| version == &artifact.version);
    Ok(installed.map(|version| InstalledComponent {
        version,
        executable,
        min_vulkan: artifact.min_vulkan.clone(),
    }))
}

async fn embedded_install(
    config: &InstallConfig,
    artifact: &Artifact,
    staging: &Path,
    payload: &'static [u8],
) -> Result<InstalledComponent, X11ComponentError> {
    crate::install_work::unpack_blocking(
        payload,
        artifact.clone(),
        config.public_key_base64.clone(),
        staging.to_path_buf(),
    )
    .await?;
    finish_install(config, artifact, staging).await
}

async fn finish_install(
    config: &InstallConfig,
    artifact: &Artifact,
    staging: &Path,
) -> Result<InstalledComponent, X11ComponentError> {
    let root = component_root(&config.data_dir);
    commit(staging, &root, &artifact.version).await?;
    Ok(InstalledComponent {
        version: artifact.version.clone(),
        executable: root
            .join("versions")
            .join(&artifact.version)
            .join(artifact.executable_relative_path()),
        min_vulkan: artifact.min_vulkan.clone(),
    })
}

async fn commit(staging: &Path, root: &Path, version: &str) -> Result<(), X11ComponentError> {
    crate::install_work::commit_blocking(
        staging.to_path_buf(),
        root.to_path_buf(),
        version.to_string(),
    )
    .await
}

pub(crate) fn commit_sync(
    staging: &Path,
    root: &Path,
    version: &str,
) -> Result<(), X11ComponentError> {
    let _lock = ComponentLock::acquire(root).map_err(install_error)?;
    ensure_replacement_allowed(root, version)?;
    let versions = root.join("versions");
    std::fs::create_dir_all(&versions).map_err(install_error)?;
    crate::install_work::replace_destination_sync(staging, &versions.join(version))?;
    write_current(root, version)?;
    let mut protected = active_versions(root)?;
    protected.insert(version.to_string());
    prune_versions_preserving(root, &protected)
}

fn ensure_replacement_allowed(root: &Path, version: &str) -> Result<(), X11ComponentError> {
    if active_version().as_deref() == Some(version) || active_versions(root)?.contains(version) {
        return Err(X11ComponentError::Install(
            "内置组件正在运行，请重启应用后再重新安装".into(),
        ));
    }
    Ok(())
}

struct StagingDirectory {
    path: PathBuf,
    armed: bool,
}

impl StagingDirectory {
    fn new(root: &Path) -> Result<Self, X11ComponentError> {
        std::fs::create_dir_all(root).map_err(install_error)?;
        let mut random = [0; 8];
        OsRng.fill_bytes(&mut random);
        let path = root.join(format!(".tmp-{}", hex::encode(random)));
        std::fs::create_dir(&path).map_err(install_error)?;
        Ok(Self { path, armed: true })
    }

    fn disarm(mut self) {
        self.armed = false;
    }

    async fn cleanup(mut self) {
        self.armed = false;
        crate::install_work::cleanup_blocking(self.path.clone()).await;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        // 取消或 panic 的兜底清理也不能阻塞 async worker。
        if self.armed {
            crate::install_work::schedule_cleanup(self.path.clone());
        }
    }
}

#[cfg(test)]
mod tests;

fn install_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Install(error.to_string())
}
