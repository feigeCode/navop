use std::{collections::BTreeSet, error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use serde_json::Value;

use crate::{
    RuntimeToolDescriptor, ToolAdapter, ToolAlias, ToolAnnotations, ToolDescriptor, ToolError,
    ToolOrigin, ToolResult, ToolTargetSpec,
};

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<ToolResult, ToolError>> + Send + 'static>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolContext {
    pub adapter: ToolAdapter,
}

impl ToolContext {
    pub fn for_adapter(adapter: ToolAdapter) -> Self {
        Self { adapter }
    }
}

pub trait ToolHandler: Send + Sync + 'static {
    fn descriptor(&self) -> ToolDescriptor;

    fn aliases(&self) -> Vec<ToolAlias> {
        Vec::new()
    }

    fn target_spec(&self) -> ToolTargetSpec {
        ToolTargetSpec::default()
    }

    fn origin(&self) -> ToolOrigin {
        ToolOrigin::Builtin
    }

    fn runtime_descriptor(&self) -> RuntimeToolDescriptor {
        let descriptor = self.descriptor();
        RuntimeToolDescriptor {
            id: descriptor.tool_id(),
            title: descriptor.title,
            description: descriptor.description,
            input_schema: descriptor.input_schema,
            output_schema: descriptor.output_schema,
            permissions: descriptor.permissions,
            mode: descriptor.mode,
            adapters: descriptor.adapters,
            annotations: descriptor.annotations,
            target: self.target_spec(),
            origin: self.origin(),
            aliases: self.aliases(),
        }
    }

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

    pub fn merge(registries: Vec<Self>) -> Result<Self, ToolRegistryError> {
        let handlers = registries
            .into_iter()
            .flat_map(|registry| registry.handlers.iter().cloned().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        Self::try_new(handlers)
    }

    pub fn list(&self, adapter: ToolAdapter) -> Vec<ToolDescriptor> {
        self.handlers
            .iter()
            .map(|handler| handler.runtime_descriptor())
            .filter(|descriptor| descriptor.supports_adapter(adapter))
            .map(|descriptor| descriptor.legacy_descriptor())
            .collect()
    }

    pub fn list_runtime(&self, adapter: ToolAdapter) -> Vec<RuntimeToolDescriptor> {
        self.handlers
            .iter()
            .map(|handler| handler.runtime_descriptor())
            .filter(|descriptor| descriptor.supports_adapter(adapter))
            .collect()
    }

    pub fn get(&self, id: &str, adapter: ToolAdapter) -> Option<ToolDescriptor> {
        self.runtime_match(id, adapter)
            .map(|(_, descriptor)| descriptor.legacy_descriptor())
    }

    pub fn get_runtime(&self, id: &str, adapter: ToolAdapter) -> Option<RuntimeToolDescriptor> {
        self.runtime_match(id, adapter)
            .map(|(_, descriptor)| descriptor)
    }

    pub fn call_annotations(
        &self,
        id: &str,
        adapter: ToolAdapter,
        input: &Value,
    ) -> Option<ToolAnnotations> {
        self.runtime_match(id, adapter)
            .map(|(handler, _)| handler.call_annotations(input))
    }

    pub async fn call(
        &self,
        id: &str,
        input: Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let Some((handler, descriptor)) = self.runtime_match(id, context.adapter) else {
            return self.unsupported_or_unknown(id, context.adapter);
        };
        if !descriptor.supports_adapter(context.adapter) {
            return Err(ToolError::UnsupportedAdapter {
                id: id.to_string(),
                adapter: context.adapter,
            });
        }
        handler.call(input, context).await
    }

    fn runtime_match(
        &self,
        id: &str,
        adapter: ToolAdapter,
    ) -> Option<(Arc<dyn ToolHandler>, RuntimeToolDescriptor)> {
        self.handlers.iter().find_map(|handler| {
            let descriptor = handler.runtime_descriptor();
            (descriptor.matches_id_or_alias(id) && descriptor.supports_adapter(adapter))
                .then(|| (handler.clone(), descriptor))
        })
    }

    fn unsupported_or_unknown(
        &self,
        id: &str,
        adapter: ToolAdapter,
    ) -> Result<ToolResult, ToolError> {
        let matches_id = self
            .handlers
            .iter()
            .map(|handler| handler.runtime_descriptor())
            .any(|descriptor| descriptor.matches_id_or_alias(id));
        if matches_id {
            return Err(ToolError::UnsupportedAdapter {
                id: id.to_string(),
                adapter,
            });
        }
        Err(ToolError::UnknownTool { id: id.to_string() })
    }
}

fn duplicate_tool_ids(handlers: &[Arc<dyn ToolHandler>]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for handler in handlers {
        for id in descriptor_keys(&handler.runtime_descriptor()) {
            if !seen.insert(id.clone()) {
                duplicates.insert(id);
            }
        }
    }
    duplicates.into_iter().collect()
}

fn descriptor_keys(descriptor: &RuntimeToolDescriptor) -> Vec<String> {
    let mut keys = vec![descriptor.id.as_str().to_string()];
    keys.extend(descriptor.aliases.iter().map(|alias| alias.id.clone()));
    keys
}
