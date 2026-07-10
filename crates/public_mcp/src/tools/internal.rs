use rmcp::{ErrorData as McpError, model::JsonObject};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc};
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

pub type InternalFunctionFuture =
    Pin<Box<dyn Future<Output = Result<Value, McpError>> + Send + 'static>>;

type InternalFunctionHandler = Arc<dyn Fn(JsonObject) -> InternalFunctionFuture + Send + Sync>;

#[derive(Clone)]
pub struct InternalFunctionDefinition {
    name: String,
    description: String,
    input_schema: Arc<JsonObject>,
    read_only: bool,
    handler: InternalFunctionHandler,
}

impl InternalFunctionDefinition {
    pub fn read_only<F, Fut>(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(JsonObject) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, McpError>> + Send + 'static,
    {
        Self::new(name, description, empty_object_schema(), true, handler)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn new<F, Fut>(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Arc<JsonObject>,
        read_only: bool,
        handler: F,
    ) -> Self
    where
        F: Fn(JsonObject) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, McpError>> + Send + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            read_only,
            handler: Arc::new(move |args| Box::pin(handler(args))),
        }
    }

    fn catalog_entry(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "read_only": self.read_only,
            "input_schema": self.input_schema.as_ref(),
        })
    }

    async fn call(&self, arguments: JsonObject) -> Result<Value, McpError> {
        (self.handler)(arguments).await
    }
}

#[derive(Clone)]
struct InternalFunctionStore {
    functions: Arc<Vec<InternalFunctionDefinition>>,
}

pub fn internal_function_tool_registry(functions: Vec<InternalFunctionDefinition>) -> ToolRegistry {
    let store = InternalFunctionStore {
        functions: Arc::new(functions),
    };
    ToolRegistry::new(vec![
        Arc::new(InternalFunctionListTool {
            store: store.clone(),
        }),
        Arc::new(InternalFunctionCallTool { store }),
    ])
}

#[derive(Clone)]
struct InternalFunctionListTool {
    store: InternalFunctionStore,
}

impl ToolHandler for InternalFunctionListTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "internal_functions.list".to_string(),
            title: "List internal functions".to_string(),
            description: "List named internal Navop functions available through MCP, including each function description, read-only flag, and input schema. Use this before internal_functions.call when you need app-level capabilities such as onetcli.app_info.".to_string(),
            input_schema: empty_object_value_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations::read_only("List internal functions"),
        }
    }

    fn call(&self, _input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let store = self.store.clone();
        Box::pin(async move {
            Ok(ToolResult::structured(json!({
                "functions": store
                    .functions
                    .iter()
                    .map(InternalFunctionDefinition::catalog_entry)
                    .collect::<Vec<_>>()
            })))
        })
    }
}

#[derive(Clone)]
struct InternalFunctionCallTool {
    store: InternalFunctionStore,
}

impl ToolHandler for InternalFunctionCallTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "internal_functions.call".to_string(),
            title: "Call internal function".to_string(),
            description: "Call one named internal Navop function with JSON arguments. Use internal_functions.list first to discover valid function names and schemas. Do not use this for SSH, SFTP, Redis, or saved connection operations; those have dedicated tool namespaces.".to_string(),
            input_schema: object_value_schema([
                ("name", string_schema()),
                ("arguments", json!({ "type": "object" })),
            ]),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations::mutating("Call internal function"),
        }
    }

    fn call_annotations(&self, input: &Value) -> ToolAnnotations {
        match self.resolve_function(input) {
            Ok(function) if function.read_only => {
                ToolAnnotations::read_only("Call internal function")
            }
            _ => ToolAnnotations::mutating("Call internal function"),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let tool = self.clone();
        Box::pin(async move {
            let function = tool.resolve_function(&input)?;
            let arguments = optional_object_value(&input, "arguments")?;
            let result = function.call(arguments).await.map_err(mcp_error_to_tool)?;
            Ok(ToolResult::structured(json!({ "result": result })))
        })
    }
}

impl InternalFunctionCallTool {
    fn resolve_function(&self, input: &Value) -> Result<InternalFunctionDefinition, ToolError> {
        let name = required_string_value(input, "name")?;
        self.store
            .functions
            .iter()
            .find(|function| function.name == name)
            .cloned()
            .ok_or_else(|| ToolError::Failed {
                message: format!("unknown internal function: {name}"),
            })
    }
}

fn object_value_schema(properties: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let properties = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<JsonObject>();
    let required = properties.keys().cloned().map(Value::String).collect();
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(properties));
    schema.insert("required".to_string(), Value::Array(required));
    Value::Object(schema)
}

fn empty_object_schema() -> Arc<JsonObject> {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    Arc::new(schema)
}

fn empty_object_value_schema() -> Value {
    json!({
        "type": "object",
        "properties": {}
    })
}

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn required_string_value(input: &Value, field: &'static str) -> Result<String, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing string argument: {field}"),
        })
}

fn optional_object_value(input: &Value, field: &'static str) -> Result<JsonObject, ToolError> {
    match input.get(field) {
        Some(Value::Object(object)) => Ok(object.clone()),
        Some(_) => Err(ToolError::Failed {
            message: format!("argument must be an object: {field}"),
        }),
        None => Ok(JsonObject::new()),
    }
}

fn mcp_error_to_tool(error: McpError) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
