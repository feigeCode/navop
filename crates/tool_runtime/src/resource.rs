use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use crate::ResourceId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Ssh,
    Sftp,
    Mysql,
    Postgres,
    Sqlite,
    Redis,
    Mongo,
    Terminal,
    Other(String),
}

impl ResourceKind {
    pub fn as_str(&self) -> &str {
        match self {
            ResourceKind::Ssh => "ssh",
            ResourceKind::Sftp => "sftp",
            ResourceKind::Mysql => "mysql",
            ResourceKind::Postgres => "postgres",
            ResourceKind::Sqlite => "sqlite",
            ResourceKind::Redis => "redis",
            ResourceKind::Mongo => "mongo",
            ResourceKind::Terminal => "terminal",
            ResourceKind::Other(value) => value,
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCapability {
    Query,
    Execute,
    ReadFile,
    WriteFile,
    ExecCommand,
    List,
    OpenSession,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOrigin {
    SavedConnection,
    ActiveSession,
    Workspace,
    PublicMcp,
    ExternalMcp,
    Acp,
    Cli,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceScope {
    pub key: String,
    pub label: String,
    pub value: String,
}

impl ResourceScope {
    pub fn new(key: impl Into<String>, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub label: String,
    pub aliases: Vec<String>,
    pub scopes: Vec<ResourceScope>,
    pub capabilities: Vec<ResourceCapability>,
    pub origin: ResourceOrigin,
}

impl ResourceRef {
    pub fn new(id: impl Into<ResourceId>, kind: ResourceKind, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            label: label.into(),
            aliases: Vec::new(),
            scopes: Vec::new(),
            capabilities: Vec::new(),
            origin: ResourceOrigin::SavedConnection,
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn with_scope(mut self, scope: ResourceScope) -> Self {
        self.scopes.push(scope);
        self
    }

    pub fn with_capability(mut self, capability: ResourceCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    fn matches_target(&self, target: &str) -> bool {
        let target_values = target_variants(target);
        self.target_values()
            .iter()
            .flat_map(|value| target_variants(value))
            .any(|value| {
                target_values
                    .iter()
                    .any(|target| same_target(&value, target))
            })
    }

    fn target_values(&self) -> Vec<&str> {
        let mut values = Vec::with_capacity(2 + self.aliases.len());
        values.push(self.id.as_str());
        values.push(self.label.as_str());
        values.extend(self.aliases.iter().map(String::as_str));
        values
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePool {
    pub default_target: Option<ResourceId>,
    pub resources: Vec<ResourceRef>,
}

impl ResourcePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_resource(mut self, resource: ResourceRef) -> Self {
        if self.default_target.is_none() {
            self.default_target = Some(resource.id.clone());
        }
        self.resources.push(resource);
        self
    }

    pub fn with_default_target(mut self, id: impl Into<ResourceId>) -> Self {
        self.default_target = Some(id.into());
        self
    }

    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRef> {
        self.resources.iter().find(|resource| &resource.id == id)
    }

    pub fn default_resource(&self) -> Option<&ResourceRef> {
        self.default_target.as_ref().and_then(|id| self.get(id))
    }

    pub fn resolve_target(&self, value: &str) -> Result<&ResourceRef, TargetResolutionError> {
        let matches = self
            .resources
            .iter()
            .filter(|resource| resource.matches_target(value))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [resource] => Ok(resource),
            [] => Err(TargetResolutionError::TargetNotInPool {
                target: value.to_string(),
            }),
            _ => Err(TargetResolutionError::AmbiguousTarget {
                target: value.to_string(),
                matches: matches.iter().map(|resource| resource.id.clone()).collect(),
            }),
        }
    }

    pub fn resolve_target_for_kinds(
        &self,
        value: &str,
        supported_kinds: &[ResourceKind],
    ) -> Result<&ResourceRef, TargetResolutionError> {
        if supported_kinds.is_empty() {
            return self.resolve_target(value);
        }
        let matches = self.matches_for_kinds(value, supported_kinds);
        if !matches.is_empty() {
            return resolved_target(value, matches);
        }
        let matches = self.linked_matches_for_kinds(value, supported_kinds);
        resolved_target(value, matches)
    }

    fn matches_for_kinds(
        &self,
        value: &str,
        supported_kinds: &[ResourceKind],
    ) -> Vec<&ResourceRef> {
        self.resources
            .iter()
            .filter(|resource| supported_kinds.contains(&resource.kind))
            .filter(|resource| resource.matches_target(value))
            .collect()
    }

    fn linked_matches_for_kinds(
        &self,
        value: &str,
        supported_kinds: &[ResourceKind],
    ) -> Vec<&ResourceRef> {
        let link_ids = self
            .resources
            .iter()
            .filter(|resource| !supported_kinds.contains(&resource.kind))
            .filter(|resource| resource.matches_target(value))
            .map(|resource| resource.id.as_str().to_string())
            .collect::<Vec<_>>();
        self.resources
            .iter()
            .filter(|resource| supported_kinds.contains(&resource.kind))
            .filter(|resource| {
                link_ids
                    .iter()
                    .any(|link_id| resource.matches_target(link_id))
            })
            .collect()
    }

    pub fn resolve_resource_target(
        &self,
        target: Option<&ResourceTarget>,
    ) -> Result<&ResourceRef, TargetResolutionError> {
        match target {
            Some(ResourceTarget::Id(id)) => {
                self.get(id)
                    .ok_or_else(|| TargetResolutionError::TargetNotInPool {
                        target: id.to_string(),
                    })
            }
            Some(ResourceTarget::Label(label)) => self.resolve_target(label),
            None => self
                .default_resource()
                .ok_or(TargetResolutionError::MissingTarget),
        }
    }

    pub fn matching_kind(&self, kind: &ResourceKind) -> Vec<&ResourceRef> {
        self.resources
            .iter()
            .filter(|resource| &resource.kind == kind)
            .collect()
    }
}

fn resolved_target<'a>(
    value: &str,
    matches: Vec<&'a ResourceRef>,
) -> Result<&'a ResourceRef, TargetResolutionError> {
    match matches.as_slice() {
        [resource] => Ok(resource),
        [] => Err(TargetResolutionError::TargetNotInPool {
            target: value.to_string(),
        }),
        _ => Err(TargetResolutionError::AmbiguousTarget {
            target: value.to_string(),
            matches: matches.iter().map(|resource| resource.id.clone()).collect(),
        }),
    }
}

fn target_variants(value: &str) -> Vec<String> {
    let mut variants = Vec::new();
    push_variant(&mut variants, value.trim());
    if let Some(host) = host_from_user_target(value) {
        push_variant(&mut variants, host);
    }
    if let Some(host) = host_from_prompt_target(value) {
        push_variant(&mut variants, host);
    }
    variants
}

fn host_from_user_target(value: &str) -> Option<&str> {
    let value = value.trim();
    let value = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(value);
    let (_, host) = value.rsplit_once('@')?;
    let end = host
        .find(|ch: char| matches!(ch, ':' | '/' | ' ' | '\t' | '$' | '#'))
        .unwrap_or(host.len());
    non_empty(&host[..end])
}

fn host_from_prompt_target(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.contains('@') || value.matches(':').count() != 1 {
        return None;
    }
    let (host, suffix) = value.split_once(':')?;
    let prompt_suffix = suffix.starts_with('~')
        || suffix.starts_with('/')
        || suffix.chars().all(|ch| ch.is_ascii_digit());
    prompt_suffix.then_some(host).and_then(non_empty)
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn push_variant(variants: &mut Vec<String>, value: &str) {
    if value.is_empty() {
        return;
    }
    let value = value.trim_matches(|ch| matches!(ch, '`' | '"' | '\''));
    if !value.is_empty() && !variants.iter().any(|item| same_target(item, value)) {
        variants.push(value.to_string());
    }
}

fn same_target(left: &str, right: &str) -> bool {
    left == right || left.eq_ignore_ascii_case(right)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTarget {
    Id(ResourceId),
    Label(String),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TargetResolutionError {
    #[error("missing target and no default target is available")]
    MissingTarget,
    #[error("target is not in resource pool: {target}")]
    TargetNotInPool { target: String },
    #[error("target `{target}` is ambiguous")]
    AmbiguousTarget {
        target: String,
        matches: Vec<ResourceId>,
    },
}
