use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalReadRequest {
    pub target: String,
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalReadResult {
    pub target: String,
    pub text: String,
    pub requested_lines: usize,
    pub returned_lines: usize,
    pub available_lines: usize,
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
    pub truncated: bool,
}
