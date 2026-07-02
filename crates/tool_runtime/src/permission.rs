use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{ResourceId, RiskLevel, ToolAnnotations, ToolId};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    Safe,
    Confirm,
    Auto,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPolicy {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PermissionPolicy {
    pub mode: PermissionProfile,
    pub read_policy: OperationPolicy,
    pub write_policy: OperationPolicy,
    pub high_risk_policy: OperationPolicy,
    pub per_tool_overrides: HashMap<ToolId, OperationPolicy>,
    pub per_resource_overrides: HashMap<ResourceId, OperationPolicy>,
}

impl PermissionPolicy {
    pub fn for_profile(mode: PermissionProfile) -> Self {
        let (read_policy, write_policy, high_risk_policy) = match mode {
            PermissionProfile::Safe => (
                OperationPolicy::Allow,
                OperationPolicy::Deny,
                OperationPolicy::Deny,
            ),
            PermissionProfile::Confirm => (
                OperationPolicy::Allow,
                OperationPolicy::Ask,
                OperationPolicy::Ask,
            ),
            PermissionProfile::Auto => (
                OperationPolicy::Allow,
                OperationPolicy::Allow,
                OperationPolicy::Ask,
            ),
            PermissionProfile::Unrestricted => (
                OperationPolicy::Allow,
                OperationPolicy::Allow,
                OperationPolicy::Allow,
            ),
        };
        Self {
            mode,
            read_policy,
            write_policy,
            high_risk_policy,
            per_tool_overrides: HashMap::new(),
            per_resource_overrides: HashMap::new(),
        }
    }

    pub fn decide(
        &self,
        tool_id: &ToolId,
        resource_id: Option<&ResourceId>,
        annotations: &ToolAnnotations,
    ) -> PermissionDecision {
        if let Some(policy) = self.per_tool_overrides.get(tool_id) {
            return (*policy).into();
        }
        if let Some(policy) = resource_id.and_then(|id| self.per_resource_overrides.get(id)) {
            return (*policy).into();
        }
        if is_high_risk(annotations) {
            return self.high_risk_policy.into();
        }
        if annotations.read_only {
            return self.read_policy.into();
        }
        self.write_policy.into()
    }
}

impl From<OperationPolicy> for PermissionDecision {
    fn from(policy: OperationPolicy) -> Self {
        match policy {
            OperationPolicy::Allow => PermissionDecision::Allow,
            OperationPolicy::Ask => PermissionDecision::Ask,
            OperationPolicy::Deny => PermissionDecision::Deny,
        }
    }
}

fn is_high_risk(annotations: &ToolAnnotations) -> bool {
    annotations.risk >= RiskLevel::High || annotations.destructive || annotations.open_world
}
