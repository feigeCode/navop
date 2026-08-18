use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWriteKeysRequest {
    pub target: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWriteKeysResult {
    pub target: String,
    pub sent: bool,
    pub bytes_written: usize,
}
