use std::{collections::BTreeSet, error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<ToolResult, ToolError>> + Send + 'static>>;

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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolAnnotations {
    pub title: String,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

impl ToolAnnotations {
    pub fn read_only(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        }
    }

    pub fn mutating(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: false,
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
    pub fn supports_adapter(&self, adapter: ToolAdapter) -> bool {
        self.adapters.contains(&adapter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolContext {
    pub adapter: ToolAdapter,
}

impl ToolContext {
    pub fn for_adapter(adapter: ToolAdapter) -> Self {
        Self { adapter }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolResult {
    pub structured_content: Value,
}

impl ToolResult {
    pub fn structured(structured_content: Value) -> Self {
        Self { structured_content }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ToolError {
    #[error("unknown tool: {id}")]
    UnknownTool { id: String },
    #[error("tool `{id}` is not exposed for adapter {adapter:?}")]
    UnsupportedAdapter { id: String, adapter: ToolAdapter },
    #[error("{message}")]
    Failed { message: String },
}

pub trait ToolHandler: Send + Sync + 'static {
    fn descriptor(&self) -> ToolDescriptor;

    fn call_annotations(&self, _input: &Value) -> ToolAnnotations {
        self.descriptor().annotations
    }

    fn call(&self, input: Value, context: ToolContext) -> ToolFuture;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRegistryError {
    duplicate_tool_ids: Vec<String>,
}

impl ToolRegistryError {
    pub fn duplicate_tool_ids(&self) -> Vec<String> {
        self.duplicate_tool_ids.clone()
    }
}

impl fmt::Display for ToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate tool ids: {}",
            self.duplicate_tool_ids.join(", ")
        )
    }
}

impl Error for ToolRegistryError {}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    handlers: Arc<Vec<Arc<dyn ToolHandler>>>,
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("handler_count", &self.handlers.len())
            .finish()
    }
}

impl ToolRegistry {
    pub fn new(handlers: Vec<Arc<dyn ToolHandler>>) -> Self {
        Self::try_new(handlers).expect("tool ids must be unique")
    }

    pub fn try_new(handlers: Vec<Arc<dyn ToolHandler>>) -> Result<Self, ToolRegistryError> {
        let duplicate_tool_ids = duplicate_tool_ids(&handlers);
        if !duplicate_tool_ids.is_empty() {
            return Err(ToolRegistryError { duplicate_tool_ids });
        }
        Ok(Self {
            handlers: Arc::new(handlers),
        })
    }

    pub fn list(&self, adapter: ToolAdapter) -> Vec<ToolDescriptor> {
        self.handlers
            .iter()
            .map(|handler| handler.descriptor())
            .filter(|descriptor| descriptor.supports_adapter(adapter))
            .collect()
    }

    pub fn get(&self, id: &str, adapter: ToolAdapter) -> Option<ToolDescriptor> {
        self.list(adapter).into_iter().find(|tool| tool.id == id)
    }

    pub fn call_annotations(
        &self,
        id: &str,
        adapter: ToolAdapter,
        input: &Value,
    ) -> Option<ToolAnnotations> {
        for handler in self.handlers.iter() {
            let descriptor = handler.descriptor();
            if descriptor.id != id || !descriptor.supports_adapter(adapter) {
                continue;
            }
            return Some(handler.call_annotations(input));
        }
        None
    }

    pub async fn call(
        &self,
        id: &str,
        input: Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        for handler in self.handlers.iter() {
            let descriptor = handler.descriptor();
            if descriptor.id != id {
                continue;
            }
            if !descriptor.supports_adapter(context.adapter) {
                return Err(ToolError::UnsupportedAdapter {
                    id: id.to_string(),
                    adapter: context.adapter,
                });
            }
            return handler.call(input, context).await;
        }
        Err(ToolError::UnknownTool { id: id.to_string() })
    }
}

fn duplicate_tool_ids(handlers: &[Arc<dyn ToolHandler>]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for handler in handlers {
        let id = handler.descriptor().id;
        if !seen.insert(id.clone()) {
            duplicates.insert(id);
        }
    }
    duplicates.into_iter().collect()
}
