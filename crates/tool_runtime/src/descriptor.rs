use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ResourceKind, ToolId};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAdapter {
    Cli,
    FunctionCalling,
    Mcp,
    Gui,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    Deterministic,
    Interactive,
    LongRunning,
    Streaming,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Read,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolAnnotations {
    pub title: String,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
    #[serde(default)]
    pub supports_parallel: bool,
    #[serde(default)]
    pub risk: RiskLevel,
}

impl ToolAnnotations {
    pub fn read_only(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
            supports_parallel: false,
            risk: RiskLevel::Read,
        }
    }

    pub fn mutating(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: false,
            supports_parallel: false,
            risk: RiskLevel::High,
        }
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk = risk;
        self
    }

    pub fn with_parallel_support(mut self, supports_parallel: bool) -> Self {
        self.supports_parallel = supports_parallel;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    #[default]
    Builtin,
    Database,
    Ssh,
    Sftp,
    Redis,
    Terminal,
    PublicMcp,
    ExternalMcp,
    Acp,
    Cli,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolAlias {
    pub id: String,
}

impl ToolAlias {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolTargetSpec {
    pub supported_kinds: Vec<ResourceKind>,
    pub required: bool,
}

impl ToolTargetSpec {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn required(supported_kinds: Vec<ResourceKind>) -> Self {
        Self {
            supported_kinds,
            required: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permissions: Vec<String>,
    pub mode: ToolMode,
    pub adapters: Vec<ToolAdapter>,
    pub annotations: ToolAnnotations,
}

impl ToolDescriptor {
    pub fn tool_id(&self) -> ToolId {
        ToolId::new(self.id.clone())
    }

    pub fn supports_adapter(&self, adapter: ToolAdapter) -> bool {
        self.adapters.contains(&adapter)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RuntimeToolDescriptor {
    pub id: ToolId,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permissions: Vec<String>,
    pub mode: ToolMode,
    pub adapters: Vec<ToolAdapter>,
    pub annotations: ToolAnnotations,
    pub target: ToolTargetSpec,
    pub origin: ToolOrigin,
    pub aliases: Vec<ToolAlias>,
}

impl RuntimeToolDescriptor {
    pub fn supports_adapter(&self, adapter: ToolAdapter) -> bool {
        self.adapters.contains(&adapter)
    }

    pub fn matches_id_or_alias(&self, value: &str) -> bool {
        self.id.as_str() == value || self.aliases.iter().any(|alias| alias.id == value)
    }

    pub fn legacy_descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.id.as_str().to_string(),
            title: self.title.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
            output_schema: self.output_schema.clone(),
            permissions: self.permissions.clone(),
            mode: self.mode,
            adapters: self.adapters.clone(),
            annotations: self.annotations.clone(),
        }
    }
}
