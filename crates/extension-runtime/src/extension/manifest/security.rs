use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPermission {
    pub raw: String,
    pub kind: PermissionKind,
    pub risk: PermissionRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    FileSystem,
    Network,
    Spawn,
    Secrets,
    Host,
    Database,
    Ui,
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRisk {
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionReview {
    pub permissions: Vec<ValidatedPermission>,
    pub high_risk_count: usize,
    pub summary: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("invalid extension permission `{permission}`: {reason}")]
    Invalid { permission: String, reason: String },
}

pub fn validate_permissions(
    permissions: &[String],
) -> Result<Vec<ValidatedPermission>, PermissionError> {
    permissions
        .iter()
        .map(|permission| validate_permission(permission))
        .collect()
}

pub fn build_permission_review(
    permissions: &[String],
) -> Result<PermissionReview, PermissionError> {
    let permissions = validate_permissions(permissions)?;
    let high_risk_count = permissions
        .iter()
        .filter(|permission| permission.risk == PermissionRisk::High)
        .count();
    let summary = permissions
        .iter()
        .map(|permission| {
            let risk = if permission.risk == PermissionRisk::High {
                "HIGH"
            } else {
                "NORMAL"
            };
            format!("[{risk}] {}", permission.raw)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(PermissionReview {
        permissions,
        high_risk_count,
        summary,
    })
}

fn validate_permission(permission: &str) -> Result<ValidatedPermission, PermissionError> {
    super::security_rules::validate_permission(permission)
}

pub fn path_has_escape(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

pub(super) fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

pub(super) fn valid(
    permission: &str,
    kind: PermissionKind,
    risk: PermissionRisk,
) -> ValidatedPermission {
    ValidatedPermission {
        raw: permission.to_string(),
        kind,
        risk,
    }
}

pub(super) fn invalid<T>(
    permission: &str,
    reason: impl Into<String>,
) -> Result<T, PermissionError> {
    Err(permission_error(permission, reason))
}

pub(super) fn permission_error(permission: &str, reason: impl Into<String>) -> PermissionError {
    PermissionError::Invalid {
        permission: permission.to_string(),
        reason: reason.into(),
    }
}
