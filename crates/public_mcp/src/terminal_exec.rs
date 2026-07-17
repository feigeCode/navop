use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalExecRequest {
    pub target: String,
    pub command: String,
    #[serde(default = "default_submit")]
    pub submit: bool,
    #[serde(default = "default_wait_for_output")]
    pub wait_for_output: bool,
    #[serde(default)]
    pub ready_timeout_ms: u64,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalExecCompletion {
    ObservedOutput,
    ShellIntegrationExit,
    SubmittedOnly,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalExecResult {
    pub target: String,
    pub command: String,
    pub submitted: bool,
    pub completion: TerminalExecCompletion,
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
    pub command_id: Option<String>,
}

fn default_submit() -> bool {
    true
}

fn default_wait_for_output() -> bool {
    true
}
