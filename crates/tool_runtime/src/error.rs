use thiserror::Error;

use crate::ToolAdapter;

#[derive(Debug, Error, PartialEq)]
pub enum ToolError {
    #[error("unknown tool: {id}")]
    UnknownTool { id: String },
    #[error("tool `{id}` is not exposed for adapter {adapter:?}")]
    UnsupportedAdapter { id: String, adapter: ToolAdapter },
    #[error("{message}")]
    Failed { message: String },
}
