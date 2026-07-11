use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlAction {
    Interrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlReadiness {
    SubmissionPending,
    CommandRunning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlRequest {
    pub target: String,
    pub action: TerminalControlAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlResult {
    pub target: String,
    pub action: TerminalControlAction,
    pub sent: bool,
    pub readiness_before: TerminalControlReadiness,
}
