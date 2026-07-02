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
        self.id.as_str() == target
            || self.label == target
            || self.aliases.iter().any(|alias| alias == target)
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
