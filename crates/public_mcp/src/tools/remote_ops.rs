use crate::registry::PublicMcpRegistry;
use crate::remote_ops::{
    RemoteCommandCancelRequest, RemoteCommandMode, RemoteCommandOutputRequest,
    RemoteCommandPollRequest, RemoteCommandSignal, RemoteExecRequest, RemoteFileWriteRequest,
    SessionDiagnosticsRequest,
};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations as RuntimeToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolHandler, ToolMode, ToolRegistry, ToolResult,
};

#[derive(Clone)]
struct RemoteOpsRuntime {
    registry: PublicMcpRegistry,
}

impl RemoteOpsRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn list_sessions(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "sessions": self.registry.list_sessions()
        })))
    }

    fn session_diagnostics(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let request = parse_diagnostics_args(arguments)?;
        let result = self
            .registry
            .session_diagnostics(request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }

    fn remote_command_poll(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let request = parse_poll_args(arguments)?;
        let result = self
            .registry
            .remote_command_poll(request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }

    fn remote_command_output(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let request = parse_output_args(arguments)?;
        let result = self
            .registry
            .remote_command_output(request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }

    fn run_cancel(&self, arguments: Option<&JsonObject>) -> Result<CallToolResult, McpError> {
        let request = parse_cancel_args(arguments)?;
        let result = self
            .registry
            .remote_command_cancel(request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }

    fn run_remote_exec(&self, arguments: Option<&JsonObject>) -> Result<CallToolResult, McpError> {
        let (session_id, request) = parse_exec_args(arguments)?;
        let result = self
            .registry
            .remote_exec(&session_id, request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }

    fn run_remote_file_write(
        &self,
        arguments: Option<&JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let (session_id, request) = parse_file_write_args(arguments)?;
        let result = self
            .registry
            .remote_file_write(&session_id, request)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(result).map_err(internal_error)?,
        ))
    }
}

pub fn remote_ops_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    let provider = RemoteOpsRuntime::new(registry);
    ToolRegistry::new(
        remote_ops_specs()
            .into_iter()
            .map(|spec| {
                Arc::new(RemoteOpsRuntimeTool {
                    provider: provider.clone(),
                    spec,
                }) as Arc<dyn ToolHandler>
            })
            .collect(),
    )
}

#[derive(Clone)]
struct RemoteOpsRuntimeTool {
    provider: RemoteOpsRuntime,
    spec: RemoteOpsToolSpec,
}

impl ToolHandler for RemoteOpsRuntimeTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.spec.id.to_string(),
            title: self.spec.title.to_string(),
            description: self.spec.description.to_string(),
            input_schema: self.spec.input_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: self.spec.annotations(),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let tool = self.clone();
        Box::pin(async move {
            tool.call_sync(input)
                .map_err(mcp_error_to_tool_error)
                .and_then(call_tool_result_to_runtime_result)
        })
    }
}

impl RemoteOpsRuntimeTool {
    fn call_sync(&self, input: Value) -> Result<CallToolResult, McpError> {
        let arguments = value_to_arguments(input)?;
        match self.spec.id {
            "public_mcp.list_sessions" => self.provider.list_sessions(),
            "public_mcp.session_diagnostics" => self.provider.session_diagnostics(Some(arguments)),
            "public_mcp.remote_command_poll" => self.provider.remote_command_poll(Some(arguments)),
            "public_mcp.remote_command_output" => {
                self.provider.remote_command_output(Some(arguments))
            }
            "public_mcp.remote_command_cancel" => self.provider.run_cancel(Some(&arguments)),
            "public_mcp.remote_exec" => self.provider.run_remote_exec(Some(&arguments)),
            "public_mcp.remote_file_write" => self.provider.run_remote_file_write(Some(&arguments)),
            _ => Err(McpError::invalid_params(
                format!("unknown remote ops tool: {}", self.spec.id),
                None,
            )),
        }
    }
}

#[derive(Clone, Copy)]
struct RemoteOpsToolSpec {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    read_only: bool,
    open_world: bool,
}

impl RemoteOpsToolSpec {
    fn input_schema(self) -> Value {
        (self.schema)()
    }

    fn annotations(self) -> RuntimeToolAnnotations {
        if self.read_only {
            RuntimeToolAnnotations::read_only(self.title)
        } else {
            RuntimeToolAnnotations {
                title: self.title.to_string(),
                read_only: false,
                destructive: true,
                idempotent: false,
                open_world: self.open_world,
            }
        }
    }
}

fn remote_ops_specs() -> Vec<RemoteOpsToolSpec> {
    vec![
        list_sessions_spec(),
        session_diagnostics_spec(),
        command_poll_spec(),
        command_output_spec(),
        command_cancel_spec(),
        remote_exec_spec(),
        remote_file_write_spec(),
    ]
}

fn list_sessions_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.list_sessions",
        title: "List terminal sessions",
        description: "List currently connected SSH terminal sessions exposed by OnetCli.",
        schema: empty_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn session_diagnostics_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.session_diagnostics",
        title: "Session diagnostics",
        description: "Report structured diagnostics for a Public MCP terminal session, including connection state and recovery hints.",
        schema: diagnostics_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_poll_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.remote_command_poll",
        title: "Poll remote command",
        description: "Poll the status of a background remote command, including running state, exit code and output byte counters.",
        schema: poll_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_output_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.remote_command_output",
        title: "Read remote command output",
        description: "Read stdout and stderr of a background remote command by byte offset.",
        schema: output_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_cancel_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.remote_command_cancel",
        title: "Cancel remote command",
        description: "Request cancellation of a background remote command.",
        schema: cancel_schema_value,
        read_only: false,
        open_world: false,
    }
}

fn remote_exec_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.remote_exec",
        title: "Execute remote command",
        description: "Run a non-interactive command on an exposed connected SSH session and return stdout, stderr, exit code, duration and timeout state.",
        schema: exec_schema_value,
        read_only: false,
        open_world: true,
    }
}

fn remote_file_write_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "public_mcp.remote_file_write",
        title: "Write remote file",
        description: "Write a file to a remote path through an exposed connected SSH session and return bytes written plus SHA-256.",
        schema: file_write_schema_value,
        read_only: false,
        open_world: true,
    }
}

fn value_to_arguments(value: Value) -> Result<JsonObject, McpError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(McpError::invalid_params(
            "remote ops tool input must be an object",
            None,
        )),
    }
}

fn call_tool_result_to_runtime_result(result: CallToolResult) -> Result<ToolResult, ToolError> {
    Ok(ToolResult::structured(
        result.structured_content.unwrap_or(Value::Null),
    ))
}

fn mcp_error_to_tool_error(error: McpError) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

fn schema_to_value(schema: Arc<JsonObject>) -> Value {
    Value::Object(schema.as_ref().clone())
}

fn empty_schema_value() -> Value {
    schema_to_value(object_schema([]))
}

fn diagnostics_schema_value() -> Value {
    schema_to_value(object_schema([("session_id", string_schema())]))
}

fn poll_schema_value() -> Value {
    schema_to_value(object_schema([("command_id", string_schema())]))
}

fn output_schema_value() -> Value {
    schema_to_value(output_schema())
}

fn cancel_schema_value() -> Value {
    schema_to_value(cancel_schema())
}

fn exec_schema_value() -> Value {
    schema_to_value(exec_schema())
}

fn file_write_schema_value() -> Value {
    schema_to_value(file_write_schema())
}

fn exec_schema() -> Arc<JsonObject> {
    let mut props = JsonObject::new();
    props.insert("session_id".to_string(), string_schema());
    props.insert("command".to_string(), string_schema());
    props.insert("cwd".to_string(), json!({ "type": ["string", "null"] }));
    props.insert(
        "env".to_string(),
        json!({ "type": "object", "additionalProperties": { "type": "string" } }),
    );
    props.insert(
        "timeout_ms".to_string(),
        json!({ "type": ["integer", "null"] }),
    );
    props.insert(
        "mode".to_string(),
        json!({ "type": "string", "enum": ["foreground", "background"] }),
    );
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(props));
    schema.insert(
        "required".to_string(),
        Value::Array(vec![
            Value::String("session_id".to_string()),
            Value::String("command".to_string()),
        ]),
    );
    Arc::new(schema)
}

fn file_write_schema() -> Arc<JsonObject> {
    let mut props = JsonObject::new();
    props.insert("session_id".to_string(), string_schema());
    props.insert("path".to_string(), string_schema());
    props.insert("content".to_string(), string_schema());
    props.insert("mode".to_string(), json!({ "type": ["integer", "null"] }));
    props.insert("overwrite".to_string(), json!({ "type": "boolean" }));
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(props));
    schema.insert(
        "required".to_string(),
        Value::Array(vec![
            Value::String("session_id".to_string()),
            Value::String("path".to_string()),
            Value::String("content".to_string()),
        ]),
    );
    Arc::new(schema)
}

fn output_schema() -> Arc<JsonObject> {
    let mut props = JsonObject::new();
    props.insert("command_id".to_string(), string_schema());
    props.insert("stdout_offset".to_string(), json!({ "type": "integer" }));
    props.insert("stderr_offset".to_string(), json!({ "type": "integer" }));
    props.insert(
        "limit_bytes".to_string(),
        json!({ "type": ["integer", "null"] }),
    );
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(props));
    schema.insert(
        "required".to_string(),
        Value::Array(vec![Value::String("command_id".to_string())]),
    );
    Arc::new(schema)
}

fn cancel_schema() -> Arc<JsonObject> {
    let mut props = JsonObject::new();
    props.insert("command_id".to_string(), string_schema());
    props.insert(
        "signal".to_string(),
        json!({ "type": "string", "enum": ["sigint", "sigterm"] }),
    );
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(props));
    schema.insert(
        "required".to_string(),
        Value::Array(vec![Value::String("command_id".to_string())]),
    );
    Arc::new(schema)
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

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn parse_exec_args(
    arguments: Option<&JsonObject>,
) -> Result<(String, RemoteExecRequest), McpError> {
    let session_id = required_string(arguments, "session_id")?.to_string();
    let command = required_string(arguments, "command")?.to_string();
    let cwd = optional_string(arguments, "cwd").map(str::to_string);
    let env = optional_string_map(arguments, "env").unwrap_or_default();
    let timeout_ms = optional_u64(arguments, "timeout_ms");
    let mode = optional_string(arguments, "mode")
        .map(parse_mode)
        .transpose()?
        .unwrap_or_default();

    Ok((
        session_id,
        RemoteExecRequest {
            session_id: String::new(),
            command,
            cwd,
            env,
            timeout_ms,
            mode,
        },
    ))
}

fn parse_mode(value: &str) -> Result<RemoteCommandMode, McpError> {
    match value {
        "foreground" => Ok(RemoteCommandMode::Foreground),
        "background" => Ok(RemoteCommandMode::Background),
        other => Err(McpError::invalid_params(
            format!("unknown remote_exec mode: {other}"),
            None,
        )),
    }
}

fn parse_file_write_args(
    arguments: Option<&JsonObject>,
) -> Result<(String, RemoteFileWriteRequest), McpError> {
    let session_id = required_string(arguments, "session_id")?.to_string();
    let path = required_string(arguments, "path")?.to_string();
    let content = required_string(arguments, "content")?.to_string();
    let mode = optional_u64(arguments, "mode").map(|value| value as u32);
    let overwrite = optional_bool(arguments, "overwrite").unwrap_or(false);

    Ok((
        session_id,
        RemoteFileWriteRequest {
            session_id: String::new(),
            path,
            content,
            mode,
            overwrite,
        },
    ))
}

fn parse_diagnostics_args(
    arguments: Option<JsonObject>,
) -> Result<SessionDiagnosticsRequest, McpError> {
    let session_id = required_string(arguments.as_ref(), "session_id")?.to_string();
    Ok(SessionDiagnosticsRequest { session_id })
}

fn parse_poll_args(arguments: Option<JsonObject>) -> Result<RemoteCommandPollRequest, McpError> {
    let command_id = required_string(arguments.as_ref(), "command_id")?.to_string();
    Ok(RemoteCommandPollRequest { command_id })
}

fn parse_output_args(
    arguments: Option<JsonObject>,
) -> Result<RemoteCommandOutputRequest, McpError> {
    let command_id = required_string(arguments.as_ref(), "command_id")?.to_string();
    let stdout_offset = optional_u64(arguments.as_ref(), "stdout_offset")
        .map(|value| value as usize)
        .unwrap_or(0);
    let stderr_offset = optional_u64(arguments.as_ref(), "stderr_offset")
        .map(|value| value as usize)
        .unwrap_or(0);
    let limit_bytes = optional_u64(arguments.as_ref(), "limit_bytes").map(|value| value as usize);
    Ok(RemoteCommandOutputRequest {
        command_id,
        stdout_offset,
        stderr_offset,
        limit_bytes,
    })
}

fn parse_cancel_args(
    arguments: Option<&JsonObject>,
) -> Result<RemoteCommandCancelRequest, McpError> {
    let command_id = required_string(arguments, "command_id")?.to_string();
    let signal = optional_string(arguments, "signal")
        .map(parse_signal)
        .transpose()?
        .unwrap_or(RemoteCommandSignal::Sigint);
    Ok(RemoteCommandCancelRequest { command_id, signal })
}

fn parse_signal(value: &str) -> Result<RemoteCommandSignal, McpError> {
    match value {
        "sigint" => Ok(RemoteCommandSignal::Sigint),
        "sigterm" => Ok(RemoteCommandSignal::Sigterm),
        other => Err(McpError::invalid_params(
            format!("unknown cancel signal: {other}"),
            None,
        )),
    }
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

fn optional_string<'a>(arguments: Option<&'a JsonObject>, field: &str) -> Option<&'a str> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_str)
}

fn optional_u64(arguments: Option<&JsonObject>, field: &str) -> Option<u64> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_u64)
}

fn optional_bool(arguments: Option<&JsonObject>, field: &str) -> Option<bool> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_bool)
}

fn optional_string_map(
    arguments: Option<&JsonObject>,
    field: &str,
) -> Option<BTreeMap<String, String>> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
}

fn internal_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(std::borrow::Cow::Owned(error.to_string()), None)
}
