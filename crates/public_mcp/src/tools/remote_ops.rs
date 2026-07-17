use crate::registry::PublicMcpRegistry;
use crate::remote_ops::{
    RemoteCommandCancelRequest, RemoteCommandMode, RemoteCommandOutputRequest,
    RemoteCommandPollRequest, RemoteCommandSignal, RemoteExecRequest, SessionDiagnosticsRequest,
};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter,
    ToolAnnotations as RuntimeToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler,
    ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[derive(Clone)]
struct RemoteOpsRuntime {
    registry: PublicMcpRegistry,
}

impl RemoteOpsRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
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

    fn target_spec(&self) -> ToolTargetSpec {
        match self.spec.id {
            "ssh.exec" => ToolTargetSpec::required_with_capabilities(
                vec![ResourceKind::Terminal],
                vec![ResourceCapability::RemoteExec],
            ),
            "ssh.session_diagnostics" => ToolTargetSpec::required(vec![ResourceKind::Terminal]),
            _ => ToolTargetSpec::none(),
        }
    }
}

impl RemoteOpsRuntimeTool {
    fn call_sync(&self, input: Value) -> Result<CallToolResult, McpError> {
        let arguments = value_to_arguments(input)?;
        match self.spec.id {
            "ssh.session_diagnostics" => self.provider.session_diagnostics(Some(arguments)),
            "ssh.command.poll" => self.provider.remote_command_poll(Some(arguments)),
            "ssh.command.output" => self.provider.remote_command_output(Some(arguments)),
            "ssh.command.cancel" => self.provider.run_cancel(Some(&arguments)),
            "ssh.exec" => self.provider.run_remote_exec(Some(&arguments)),
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
                supports_parallel: false,
                risk: RiskLevel::High,
            }
        }
    }
}

fn remote_ops_specs() -> Vec<RemoteOpsToolSpec> {
    vec![
        session_diagnostics_spec(),
        command_poll_spec(),
        command_output_spec(),
        command_cancel_spec(),
        remote_exec_spec(),
    ]
}

fn session_diagnostics_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "ssh.session_diagnostics",
        title: "Session diagnostics",
        description: "Inspect one active SSH terminal session by session_id and return connection state, host label, current working directory, terminal size, and recovery hints. Use when a session cannot run commands or needs troubleshooting.",
        schema: diagnostics_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_poll_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "ssh.command.poll",
        title: "Poll remote command",
        description: "Check the status of a background SSH command or a foreground ssh.exec call that detached after its wait timeout. Returns running/exited/failed state, exit code when available, elapsed duration, and stdout/stderr byte counts.",
        schema: poll_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_output_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "ssh.command.output",
        title: "Read remote command output",
        description: "Read buffered stdout and stderr incrementally from a background SSH command or a foreground command that detached after timeout. Use stdout_offset and stderr_offset from the previous response to page through output without rereading old bytes.",
        schema: output_schema_value,
        read_only: true,
        open_world: false,
    }
}

fn command_cancel_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "ssh.command.cancel",
        title: "Cancel remote command",
        description: "Request cancellation of a tracked SSH command by command_id. Use signal=\"sigint\" for graceful interrupt or signal=\"sigterm\" for termination.",
        schema: cancel_schema_value,
        read_only: false,
        open_world: false,
    }
}

fn remote_exec_spec() -> RemoteOpsToolSpec {
    RemoteOpsToolSpec {
        id: "ssh.exec",
        title: "Execute remote command",
        description: "Run a non-interactive shell command on an active SSH target. Output is available incrementally while it runs. If foreground wait times out, the command continues on its own SSH channel and the result includes command_id for ssh.command.poll/output/cancel. For remote file read/write, use sftp.read or sftp.write instead of shell commands.",
        schema: exec_schema_value,
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

fn exec_schema() -> Arc<JsonObject> {
    let mut props = JsonObject::new();
    props.insert("target".to_string(), string_schema());
    props.insert("command".to_string(), string_schema());
    props.insert("connection".to_string(), string_schema());
    props.insert("connection_id".to_string(), string_schema());
    props.insert("session_id".to_string(), string_schema());
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
            Value::String("target".to_string()),
            Value::String("command".to_string()),
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
    let session_id = required_target(arguments)?.to_string();
    let command = required_string(arguments, "command")?.to_string();
    let cwd = optional_string(arguments, "cwd").map(str::to_string);
    let env = optional_string_map(arguments, "env").unwrap_or_default();
    let timeout_ms = optional_u64(arguments, "timeout_ms");
    let mode = optional_string(arguments, "mode")
        .map(parse_mode)
        .transpose()?
        .unwrap_or_default();

    Ok((
        session_id.clone(),
        RemoteExecRequest {
            session_id,
            command,
            cwd,
            env,
            timeout_ms,
            mode,
        },
    ))
}

fn required_target(arguments: Option<&JsonObject>) -> Result<&str, McpError> {
    for field in ["target", "connection", "connection_id", "session_id"] {
        if let Some(value) = optional_string(arguments, field) {
            return Ok(value);
        }
    }
    Err(McpError::invalid_params(
        "missing string argument: target",
        None,
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
