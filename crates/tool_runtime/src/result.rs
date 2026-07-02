use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolResult {
    pub structured_content: Value,
}

impl ToolResult {
    pub fn structured(structured_content: Value) -> Self {
        Self { structured_content }
    }
}
