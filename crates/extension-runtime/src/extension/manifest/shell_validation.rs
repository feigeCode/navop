use std::collections::HashSet;
use std::path::{Component, Path};

use semver::Version;

use super::parser::validate_extension_path_containment;
use super::{Manifest, ShellHostModule, ShellViewContrib};

const SHELL_EXEC_PERMISSION: &str = "shell:exec";
const RESERVED_BACKEND_ALIASES: &[&str] = &["host", "navop", "default"];
const GPUI_SHELL_VERSION: &str = "0.2.0";

pub(crate) fn validate_shell_views(manifest: &Manifest) -> Result<(), ShellViewValidationError> {
    if manifest.contributes.shell_views.is_empty() {
        return Ok(());
    }
    if !manifest
        .permissions
        .iter()
        .any(|permission| permission == SHELL_EXEC_PERMISSION)
    {
        return Err(error("/permissions", "shellViews requires `shell:exec`"));
    }
    validate_gpui_shell_version(manifest)?;

    let ipc_ids: HashSet<&str> = manifest
        .runtime
        .ipc
        .iter()
        .map(|runtime| runtime.id.as_str())
        .collect();
    let mut view_ids = HashSet::new();
    for view in &manifest.contributes.shell_views {
        validate_shell_view(manifest, view, &ipc_ids)?;
        if !view_ids.insert(view.id.as_str()) {
            return Err(error(
                shell_field(view, "id"),
                format!("duplicate shell view id `{}`", view.id),
            ));
        }
    }
    Ok(())
}

fn validate_shell_view(
    manifest: &Manifest,
    view: &ShellViewContrib,
    ipc_ids: &HashSet<&str>,
) -> Result<(), ShellViewValidationError> {
    validate_identifier(&view.id, shell_field(view, "id"), "shell view id")?;
    if view.title.trim().is_empty() {
        return Err(error(shell_field(view, "title"), "must not be empty"));
    }
    validate_relative_path(&view.entry, shell_field(view, "entry"), "entry")?;
    if manifest.manifest_dir.exists() {
        let entry_path = manifest.manifest_dir.join(&view.entry);
        if !entry_path.is_file() {
            return Err(error(
                shell_field(view, "entry"),
                format!("shell entry does not exist: {}", entry_path.display()),
            ));
        }
        validate_extension_path_containment(
            &manifest.manifest_dir,
            &entry_path,
            shell_field(view, "entry"),
            "shell entry",
        )
        .map_err(|error| error_from_manifest(error, shell_field(view, "entry")))?;
    }
    if let Some(icon) = view.icon.as_deref() {
        validate_relative_path(icon, shell_field(view, "icon"), "icon")?;
        if manifest.manifest_dir.exists() {
            let icon_path = manifest.manifest_dir.join(icon);
            if !icon_path.is_file() {
                return Err(error(
                    shell_field(view, "icon"),
                    format!("shell icon does not exist: {}", icon_path.display()),
                ));
            }
            validate_extension_path_containment(
                &manifest.manifest_dir,
                &icon_path,
                shell_field(view, "icon"),
                "shell icon",
            )
            .map_err(|error| error_from_manifest(error, shell_field(view, "icon")))?;
        }
    }
    validate_backends(view, ipc_ids)?;
    validate_modules(view)
}

fn validate_gpui_shell_version(manifest: &Manifest) -> Result<(), ShellViewValidationError> {
    let required = Version::parse(&manifest.engines.gpui_shell).map_err(|_| {
        error(
            "/engines/gpui_shell",
            "shellViews requires a semantic gpui_shell version",
        )
    })?;
    let current = Version::parse(GPUI_SHELL_VERSION).expect("host shell version must be semantic");
    let compatible = if required.major == 0 {
        current.major == 0 && current.minor == required.minor
    } else {
        current.major == required.major
    };
    if compatible && current >= required {
        return Ok(());
    }
    Err(error(
        "/engines/gpui_shell",
        format!("requires gpui-shell {required}, host provides {current}"),
    ))
}

fn error_from_manifest(error: super::ManifestError, field: String) -> ShellViewValidationError {
    match error {
        super::ManifestError::InvalidField { reason, .. } => {
            ShellViewValidationError { field, reason }
        }
        other => ShellViewValidationError {
            field,
            reason: other.to_string(),
        },
    }
}

fn validate_backends(
    view: &ShellViewContrib,
    ipc_ids: &HashSet<&str>,
) -> Result<(), ShellViewValidationError> {
    for (alias, runtime_id) in &view.backends {
        let field = format!("{}/backends/{alias}", shell_root(view));
        validate_identifier(alias, field.clone(), "backend alias")?;
        if RESERVED_BACKEND_ALIASES.contains(&alias.as_str()) {
            return Err(error(field, format!("reserved backend alias `{alias}`")));
        }
        if !ipc_ids.contains(runtime_id.as_str()) {
            return Err(error(
                field,
                format!("unknown IPC runtime `{runtime_id}` in this manifest"),
            ));
        }
    }
    Ok(())
}

fn validate_modules(view: &ShellViewContrib) -> Result<(), ShellViewValidationError> {
    let mut modules = HashSet::new();
    for module in &view.modules {
        if !matches!(
            module,
            ShellHostModule::Context
                | ShellHostModule::Resource
                | ShellHostModule::Job
                | ShellHostModule::Event
                | ShellHostModule::Blob
                | ShellHostModule::Log
                | ShellHostModule::Runtime
        ) {
            return Err(error(
                shell_field(view, "modules"),
                format!("shell host module `{module:?}` is not supported by this host"),
            ));
        }
        if !modules.insert(*module) {
            return Err(error(
                shell_field(view, "modules"),
                format!("duplicate shell host module `{module:?}`"),
            ));
        }
    }
    if view.backends.is_empty()
        && view
            .modules
            .iter()
            .copied()
            .any(ShellHostModule::requires_backend)
    {
        return Err(error(
            shell_field(view, "backends"),
            "resource/job/event/blob modules require at least one backend",
        ));
    }
    Ok(())
}

fn validate_identifier(
    value: &str,
    field: String,
    kind: &str,
) -> Result<(), ShellViewValidationError> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(&byte)
        })
        || value.starts_with(['.', '-', '_'])
        || value.ends_with(['.', '-', '_'])
        || value.contains("..")
    {
        return Err(error(
            field,
            format!("{kind} may contain only lowercase ASCII, digits, '.', '-' and '_'"),
        ));
    }
    Ok(())
}

fn validate_relative_path(
    value: &str,
    field: String,
    kind: &str,
) -> Result<(), ShellViewValidationError> {
    if value.trim().is_empty() {
        return Err(error(field, format!("{kind} must not be empty")));
    }
    let path = Path::new(value);
    let has_windows_prefix = value.starts_with("\\\\")
        || value.starts_with("//")
        || value.as_bytes().get(1) == Some(&b':');
    if path.is_absolute() || has_windows_prefix {
        return Err(error(field, format!("{kind} must be a relative path")));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(error(
            field,
            format!("{kind} path must not escape the extension root"),
        ));
    }
    Ok(())
}

fn shell_root(view: &ShellViewContrib) -> String {
    format!("/contributes/shellViews/{}", view.id)
}

fn shell_field(view: &ShellViewContrib, field: &str) -> String {
    format!("{}/{}", shell_root(view), field)
}

fn error(field: impl Into<String>, reason: impl Into<String>) -> ShellViewValidationError {
    ShellViewValidationError {
        field: field.into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellViewValidationError {
    pub field: String,
    pub reason: String,
}
