use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use crate::approval::PublicMcpApprovalOutcome;
use crate::permissions::{ApprovalDecision, PublicMcpOperationKind, decide_permission};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc};

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
pub struct InternalFunctionToolProvider {
    functions: Arc<Vec<InternalFunctionDefinition>>,
}

impl InternalFunctionToolProvider {
    pub fn new(functions: Vec<InternalFunctionDefinition>) -> Self {
        Self {
            functions: Arc::new(functions),
        }
    }

    fn list_functions(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "functions": self
                .functions
                .iter()
                .map(InternalFunctionDefinition::catalog_entry)
                .collect::<Vec<_>>()
        })))
    }

    async fn call_function(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let (function, function_args) = self.resolve_call(arguments.as_ref())?;
        if function.read_only {
            return call_resolved_function(function, function_args).await;
        }

        match decide_permission(
            context.permission_mode,
            PublicMcpOperationKind::CallInternalFunction,
        ) {
            ApprovalDecision::Allow => call_resolved_function(function, function_args).await,
            ApprovalDecision::Ask => self.ask_then_call(function, function_args, context).await,
            ApprovalDecision::Deny => Ok(permission_denied_result(
                "internal function call denied by permission mode",
            )),
        }
    }

    async fn ask_then_call(
        &self,
        function: InternalFunctionDefinition,
        arguments: JsonObject,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let outcome = context
            .request_approval(
                PublicMcpOperationKind::CallInternalFunction,
                "public_mcp.internal_functions.call",
                format!("Call internal function {}", function.name),
                json!({
                    "function": function.name,
                    "arguments": arguments,
                }),
            )
            .await;

        match outcome {
            PublicMcpApprovalOutcome::Approved => call_resolved_function(function, arguments).await,
            PublicMcpApprovalOutcome::Denied { reason } => {
                Ok(permission_denied_result(reason.unwrap_or_else(|| {
                    "internal function call denied by approval".to_string()
                })))
            }
        }
    }

    fn resolve_call(
        &self,
        arguments: Option<&JsonObject>,
    ) -> Result<(InternalFunctionDefinition, JsonObject), McpError> {
        let name = required_string(arguments, "name")?;
        let function = self
            .functions
            .iter()
            .find(|function| function.name == name)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(format!("unknown internal function: {name}"), None)
            })?;
        Ok((function, optional_object(arguments, "arguments")?))
    }
}

impl PublicMcpToolProvider for InternalFunctionToolProvider {
    fn tools(&self) -> Vec<Tool> {
        internal_function_tools()
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        match name {
            "public_mcp.internal_functions.list" => {
                let result = self.list_functions();
                Some(Box::pin(async move { result }))
            }
            "public_mcp.internal_functions.call" => {
                let provider = self.clone();
                Some(Box::pin(async move {
                    provider.call_function(arguments, context).await
                }))
            }
            _ => None,
        }
    }
}

async fn call_resolved_function(
    function: InternalFunctionDefinition,
    arguments: JsonObject,
) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::structured(json!({
        "result": function.call(arguments).await?
    })))
}

fn internal_function_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "public_mcp.internal_functions.list",
            "List internal OnetCli functions exposed through public MCP.",
            object_schema([]),
        )
        .with_annotations(read_only_annotations("List internal functions")),
        Tool::new(
            "public_mcp.internal_functions.call",
            "Call a registered internal OnetCli function.",
            object_schema([
                ("name", string_schema()),
                ("arguments", json!({ "type": "object" })),
            ]),
        )
        .with_annotations(
            ToolAnnotations::with_title("Call internal function")
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(false),
        ),
    ]
}

fn read_only_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn object_schema(properties: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<JsonObject> {
    let required = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<JsonObject>();
    let required_names = required
        .keys()
        .cloned()
        .map(Value::String)
        .collect::<Vec<_>>();
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(required));
    if !required_names.is_empty() {
        schema.insert("required".to_string(), Value::Array(required_names));
    }
    Arc::new(schema)
}

fn empty_object_schema() -> Arc<JsonObject> {
    object_schema([])
}

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn required_string<'a>(
    arguments: Option<&'a JsonObject>,
    field: &'static str,
) -> Result<&'a str, McpError> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params(format!("missing string argument: {field}"), None))
}

fn optional_object(
    arguments: Option<&JsonObject>,
    field: &'static str,
) -> Result<JsonObject, McpError> {
    match arguments.and_then(|args| args.get(field)) {
        Some(Value::Object(object)) => Ok(object.clone()),
        Some(_) => Err(McpError::invalid_params(
            format!("argument must be an object: {field}"),
            None,
        )),
        None => Ok(JsonObject::new()),
    }
}

fn permission_denied_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": "permission_denied",
        "message": message.into()
    }))
}
