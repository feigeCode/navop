use serde::{Deserialize, Serialize};

use super::DiffStatus;

/// Stored routine kind exposed by database metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    #[default]
    Function,
    Procedure,
}

/// Read-only schema metadata used to compare functions and procedures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineSchema {
    pub kind: RoutineKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(default)]
    pub parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Difference for one function or procedure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineDiff {
    pub name: String,
    pub kind: RoutineKind,
    pub status: DiffStatus,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<RoutineSchema>,
    pub target: Option<RoutineSchema>,
}

/// Read-only schema metadata used to compare triggers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    pub event: String,
    pub timing: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

/// Difference for one trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDiff {
    pub name: String,
    pub status: DiffStatus,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<TriggerSchema>,
    pub target: Option<TriggerSchema>,
}
