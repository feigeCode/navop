use std::path::{Path, PathBuf};

use thiserror::Error;

use super::schema::Manifest;
use super::security::{path_has_escape, validate_permissions};
use super::versioning::{CompatibilityError, HostApiVersions, check_compatibility};

pub const MANIFEST_FILE_NAME: &str = "extension.json";

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("未找到 extension.json: {0}")]
    NotFound(PathBuf),

    #[error("读取 extension.json 失败 ({path}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("解析 extension.json 失败 ({path}): {message}")]
    Parse { path: PathBuf, message: String },

    #[error("manifest 字段 {field} 非法: {reason}")]
    InvalidField { field: String, reason: String },

    #[error("manifest runtime id 重复: {duplicated:?}")]
    DuplicatedRuntimeId { duplicated: Vec<String> },

    #[error(transparent)]
    Incompatible(#[from] CompatibilityError),
}

pub fn load_from_dir(dir: &Path) -> Result<Manifest, ManifestError> {
    let manifest_path = dir.join(MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return Err(ManifestError::NotFound(manifest_path));
    }

    let content = std::fs::read_to_string(&manifest_path).map_err(|e| ManifestError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;

    let mut manifest: Manifest =
        serde_json::from_str(&content).map_err(|e| ManifestError::Parse {
            path: manifest_path.clone(),
            message: format!("第 {} 行,第 {} 列: {}", e.line(), e.column(), e),
        })?;

    manifest.manifest_dir = dir.to_path_buf();
    validate_structural(&manifest)?;
    Ok(manifest)
}

pub fn load_and_check(
    dir: &Path,
    host_version: &semver::Version,
    host_apis: &HostApiVersions,
) -> Result<Manifest, ManifestError> {
    let manifest = load_from_dir(dir)?;
    check_compatibility(&manifest, host_version, host_apis)?;
    Ok(manifest)
}

fn validate_structural(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.id.trim().is_empty() {
        return invalid_field("/id", "不能为空");
    }
    if !is_valid_id_format(&manifest.id) {
        return invalid_field("/id", "只能包含小写字母、数字、点、下划线、连字符");
    }
    if manifest.id.len() > 128 {
        return invalid_field("/id", "长度不能超过 128");
    }
    if manifest.name.trim().is_empty() {
        return invalid_field("/name", "不能为空");
    }
    if manifest.version.trim().is_empty() {
        return invalid_field("/version", "不能为空");
    }
    if semver::Version::parse(&manifest.version).is_err() {
        return invalid_field(
            "/version",
            format!("不是合法的 SemVer: {}", manifest.version),
        );
    }
    let duplicated = manifest.runtime.duplicated_ids();
    if !duplicated.is_empty() {
        return Err(ManifestError::DuplicatedRuntimeId { duplicated });
    }
    validate_security(manifest)?;
    Ok(())
}

fn validate_security(manifest: &Manifest) -> Result<(), ManifestError> {
    validate_permissions(&manifest.permissions).map_err(|err| ManifestError::InvalidField {
        field: "/permissions".into(),
        reason: err.to_string(),
    })?;
    for runtime in &manifest.runtime.wasm {
        validate_wasm_module_path(&runtime.id, &runtime.module)?;
    }
    for runtime in &manifest.runtime.ipc {
        validate_ipc_command_path(&runtime.id, &runtime.entry.command)?;
    }
    Ok(())
}

fn validate_wasm_module_path(runtime_id: &str, module: &str) -> Result<(), ManifestError> {
    let field = format!("/runtime/wasm/{runtime_id}/module");
    if Path::new(module).is_absolute() {
        return invalid_field(field, "WASM module 不能使用绝对路径");
    }
    if path_has_escape(module) {
        return invalid_field(field, "WASM module 路径不能逃逸扩展目录");
    }
    Ok(())
}

fn validate_ipc_command_path(runtime_id: &str, command: &str) -> Result<(), ManifestError> {
    let field = format!("/runtime/ipc/{runtime_id}/entry/command");
    if path_has_escape(command) {
        return invalid_field(field, "IPC command 路径不能逃逸扩展目录");
    }
    if Path::new(command).is_absolute() && !command.starts_with("/usr/bin/") {
        return invalid_field(field, "IPC command 绝对路径仅允许 /usr/bin allowlist");
    }
    Ok(())
}

fn invalid_field<T>(
    field: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, ManifestError> {
    Err(ManifestError::InvalidField {
        field: field.into(),
        reason: reason.into(),
    })
}

fn is_valid_id_format(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}
