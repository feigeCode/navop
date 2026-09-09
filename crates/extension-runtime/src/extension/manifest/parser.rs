use std::path::{Path, PathBuf};

use thiserror::Error;

use super::schema::Manifest;
use super::security::{path_has_escape, validate_permissions};
use super::shell_validation::validate_shell_views;
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
    validate_shell_views(manifest).map_err(|error| ManifestError::InvalidField {
        field: error.field,
        reason: error.reason,
    })?;
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
        validate_extension_path_containment(
            &manifest.manifest_dir,
            &manifest.manifest_dir.join(&runtime.module),
            format!("/runtime/wasm/{}/module", runtime.id),
            "WASM module",
        )?;
    }
    for runtime in &manifest.runtime.ipc {
        validate_ipc_command_path(&runtime.id, &runtime.entry.command)?;
        validate_ipc_working_dir(&runtime.id, runtime.entry.working_dir.as_deref())?;
        validate_ipc_transport(&runtime.id, &runtime.transport.kind)?;
        let working_dir = runtime
            .entry
            .working_dir
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| manifest.manifest_dir.join(path))
            .unwrap_or_else(|| manifest.manifest_dir.clone());
        validate_extension_path_containment(
            &manifest.manifest_dir,
            &working_dir,
            format!("/runtime/ipc/{}/entry/working_dir", runtime.id),
            "IPC working_dir",
        )?;
        if !Path::new(&runtime.entry.command).is_absolute() {
            validate_extension_path_containment(
                &manifest.manifest_dir,
                &working_dir.join(&runtime.entry.command),
                format!("/runtime/ipc/{}/entry/command", runtime.id),
                "IPC command",
            )?;
        }
    }
    for runtime in &manifest.runtime.ipc {
        validate_ipc_spawn_permission(
            &runtime.id,
            &runtime.entry.command,
            runtime.entry.working_dir.as_deref(),
            &manifest.permissions,
        )?;
        validate_ipc_restart_policy(
            &runtime.id,
            runtime.auto_restart,
            runtime.max_restart_attempts,
        )?;
    }
    Ok(())
}

fn validate_ipc_restart_policy(
    runtime_id: &str,
    auto_restart: bool,
    max_restart_attempts: u32,
) -> Result<(), ManifestError> {
    if !auto_restart || max_restart_attempts > 0 {
        return Ok(());
    }
    invalid_field(
        format!("/runtime/ipc/{runtime_id}/max_restart_attempts"),
        "auto-restart runtime 必须至少允许 1 次重启",
    )
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
    if !Path::new(command).is_absolute() && !command.contains('/') && !command.contains('\\') {
        return invalid_field(
            field,
            "IPC command 不能依赖 PATH 查找，必须使用扩展内相对路径或 /usr/bin allowlist",
        );
    }
    if path_has_escape(command) {
        return invalid_field(field, "IPC command 路径不能逃逸扩展目录");
    }
    if Path::new(command).is_absolute() && !command.starts_with("/usr/bin/") {
        return invalid_field(field, "IPC command 绝对路径仅允许 /usr/bin allowlist");
    }
    Ok(())
}

fn validate_ipc_working_dir(
    runtime_id: &str,
    working_dir: Option<&str>,
) -> Result<(), ManifestError> {
    let Some(working_dir) = working_dir else {
        return Ok(());
    };
    let field = format!("/runtime/ipc/{runtime_id}/entry/working_dir");
    if Path::new(working_dir).is_absolute() {
        return invalid_field(field, "IPC working_dir 不能使用绝对路径");
    }
    if path_has_escape(working_dir) {
        return invalid_field(field, "IPC working_dir 路径不能逃逸扩展目录");
    }
    Ok(())
}

fn validate_ipc_transport(runtime_id: &str, kind: &str) -> Result<(), ManifestError> {
    if kind == "local_socket" {
        return Ok(());
    }
    invalid_field(
        format!("/runtime/ipc/{runtime_id}/transport/kind"),
        "当前仅支持 local_socket",
    )
}

fn validate_ipc_spawn_permission(
    runtime_id: &str,
    command: &str,
    working_dir: Option<&str>,
    permissions: &[String],
) -> Result<(), ManifestError> {
    let required = required_spawn_permission(command, working_dir);
    if permissions.iter().any(|permission| permission == &required) {
        return Ok(());
    }
    invalid_field(
        format!("/runtime/ipc/{runtime_id}/entry/command"),
        format!("IPC command 必须声明精确权限 `{required}`"),
    )
}

pub(crate) fn required_spawn_permission(command: &str, working_dir: Option<&str>) -> String {
    let command = Path::new(command);
    if command.is_absolute() {
        return format!("spawn:{}", command.display());
    }

    let mut relative = PathBuf::new();
    if let Some(working_dir) = working_dir.filter(|path| !path.trim().is_empty()) {
        relative.push(working_dir);
    }
    relative.push(command);
    let normalized = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => Some(component.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("spawn:./{normalized}")
}

pub(super) fn validate_extension_path_containment(
    extension_root: &Path,
    candidate: &Path,
    field: String,
    kind: &str,
) -> Result<(), ManifestError> {
    let canonical_root = extension_root
        .canonicalize()
        .map_err(|error| ManifestError::Io {
            path: extension_root.to_path_buf(),
            source: error,
        })?;
    let mut existing = candidate;
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return invalid_field(field, format!("{kind} 无法解析到扩展目录"));
        };
        existing = parent;
    }
    let canonical_existing = existing.canonicalize().map_err(|error| ManifestError::Io {
        path: existing.to_path_buf(),
        source: error,
    })?;
    if canonical_existing.starts_with(&canonical_root) {
        return Ok(());
    }
    invalid_field(field, format!("{kind} 通过符号链接逃逸扩展目录"))
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
