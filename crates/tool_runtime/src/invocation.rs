use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PermissionPolicy, ResourcePool, ResourceTarget, ToolId};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCaller {
    Agent,
    Acp,
    Mcp,
    Cli,
    Ui,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AuditContext {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolInvocation {
    pub tool_id: ToolId,
    pub arguments: Value,
    pub target: Option<ResourceTarget>,
    pub resources: ResourcePool,
    pub permission: PermissionPolicy,
    pub caller: ToolCaller,
    pub audit: AuditContext,
}

impl ToolInvocation {
    pub fn new(
        tool_id: ToolId,
        arguments: Value,
        resources: ResourcePool,
        permission: PermissionPolicy,
        caller: ToolCaller,
    ) -> Self {
        Self {
            tool_id,
            arguments,
            target: None,
            resources,
            permission,
            caller,
            audit: AuditContext::default(),
        }
    }

    pub fn with_target(mut self, target: ResourceTarget) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_audit(mut self, audit: AuditContext) -> Self {
        self.audit = audit;
        self
    }
}
