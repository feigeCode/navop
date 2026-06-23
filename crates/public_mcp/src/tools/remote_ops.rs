use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use crate::approval::PublicMcpApprovalOutcome;
use crate::permissions::{ApprovalDecision, PublicMcpOperationKind, decide_permission};
use crate::registry::PublicMcpRegistry;
use crate::remote_ops::{
    DEFAULT_FOREGROUND_TIMEOUT_MS, DEFAULT_OUTPUT_LIMIT_BYTES, RemoteCommandCancelRequest,
    RemoteCommandMode, RemoteCommandOutputRequest, RemoteCommandPollRequest, RemoteCommandSignal,
    RemoteExecRequest, RemoteFileWriteRequest, SessionDiagnosticsRequest,
};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 结构化远程操作工具提供者。基于 `PublicMcpRegistry` 暴露的非交互 SSH 执行能力。
#[derive(Clone)]
pub struct RemoteOpsToolProvider {
    registry: PublicMcpRegistry,
}

impl RemoteOpsToolProvider {
    pub fn new(registry: PublicMcpRegistry) -> Self {
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

    async fn remote_command_cancel(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        match decide_permission(
            context.permission_mode,
            PublicMcpOperationKind::CancelRemoteCommand,
        ) {
            ApprovalDecision::Allow => self.run_cancel(arguments.as_ref()),
            ApprovalDecision::Ask => self.ask_then_cancel(arguments, context).await,
            ApprovalDecision::Deny => Ok(permission_denied_result(
                "remote command cancel denied by permission mode",
            )),
        }
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

    async fn ask_then_cancel(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let request = parse_cancel_args(arguments.as_ref())?;
        let command_preview = self
            .registry
            .command_store()
            .command_text(&request.command_id)
            .map(|text| text.chars().take(160).collect::<String>())
            .unwrap_or_default();
        let outcome = context
            .request_approval(
                PublicMcpOperationKind::CancelRemoteCommand,
                "public_mcp.remote_command_cancel",
                format!("Cancel remote command {}", request.command_id),
                json!({
                    "command_id": request.command_id,
                    "command_preview": command_preview,
                    "signal": request.signal,
                }),
            )
            .await;

        match outcome {
            PublicMcpApprovalOutcome::Approved => self.run_cancel(arguments.as_ref()),
            PublicMcpApprovalOutcome::Denied { reason } => {
                Ok(permission_denied_result(reason.unwrap_or_else(|| {
                    "remote command cancel denied by approval".to_string()
                })))
            }
        }
    }

    async fn remote_exec(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        match decide_permission(
            context.permission_mode,
            PublicMcpOperationKind::ExecuteRemoteCommand,
        ) {
            ApprovalDecision::Allow => self.run_remote_exec(arguments.as_ref()),
            ApprovalDecision::Ask => self.ask_then_exec(arguments, context).await,
            ApprovalDecision::Deny => Ok(permission_denied_result(
                "remote exec denied by permission mode",
            )),
        }
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

    async fn ask_then_exec(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let (session_id, request) = parse_exec_args(arguments.as_ref())?;
        let command_preview = request.command.chars().take(160).collect::<String>();
        let risk = classify_command_risk(&request.command);
        let outcome = context
            .request_approval(
                PublicMcpOperationKind::ExecuteRemoteCommand,
                "public_mcp.remote_exec",
                format!("Execute remote command on {session_id}"),
                json!({
                    "session_id": session_id,
                    "command_preview": command_preview,
                    "cwd": request.cwd,
                    "env_keys": request.env.keys().collect::<Vec<_>>(),
                    "mode": request.mode,
                    "timeout_ms": request.timeout_ms.unwrap_or(DEFAULT_FOREGROUND_TIMEOUT_MS),
                    "risk_classification": risk,
                }),
            )
            .await;

        match outcome {
            PublicMcpApprovalOutcome::Approved => self.run_remote_exec(arguments.as_ref()),
            PublicMcpApprovalOutcome::Denied { reason } => {
                Ok(permission_denied_result(reason.unwrap_or_else(|| {
                    "remote exec denied by approval".to_string()
                })))
            }
        }
    }

    async fn remote_file_write(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        match decide_permission(
            context.permission_mode,
            PublicMcpOperationKind::WriteRemoteFile,
        ) {
            ApprovalDecision::Allow => self.run_remote_file_write(arguments.as_ref()),
            ApprovalDecision::Ask => self.ask_then_write_file(arguments, context).await,
            ApprovalDecision::Deny => Ok(permission_denied_result(
                "remote file write denied by permission mode",
            )),
        }
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

    async fn ask_then_write_file(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let (session_id, request) = parse_file_write_args(arguments.as_ref())?;
        let risk = classify_path_risk(&request.path);
        let outcome = context
            .request_approval(
                PublicMcpOperationKind::WriteRemoteFile,
                "public_mcp.remote_file_write",
                format!("Write remote file on {session_id}"),
                json!({
                    "session_id": session_id,
                    "path": request.path,
                    "bytes": request.content.len(),
                    "overwrite": request.overwrite,
                    "mode": request.mode,
                    "risk_classification": risk,
                }),
            )
            .await;

        match outcome {
            PublicMcpApprovalOutcome::Approved => self.run_remote_file_write(arguments.as_ref()),
            PublicMcpApprovalOutcome::Denied { reason } => {
                Ok(permission_denied_result(reason.unwrap_or_else(|| {
                    "remote file write denied by approval".to_string()
                })))
            }
        }
    }
}

impl PublicMcpToolProvider for RemoteOpsToolProvider {
    fn tools(&self) -> Vec<Tool> {
        remote_ops_tools()
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        match name {
            "public_mcp.list_sessions" => {
                let result = self.list_sessions();
                Some(Box::pin(async move { result }))
            }
            "public_mcp.session_diagnostics" => {
                let result = self.session_diagnostics(arguments);
                Some(Box::pin(async move { result }))
            }
            "public_mcp.remote_command_poll" => {
                let result = self.remote_command_poll(arguments);
                Some(Box::pin(async move { result }))
            }
            "public_mcp.remote_command_output" => {
                let result = self.remote_command_output(arguments);
                Some(Box::pin(async move { result }))
            }
            "public_mcp.remote_command_cancel" => {
                let provider = self.clone();
                Some(Box::pin(async move {
                    provider.remote_command_cancel(arguments, context).await
                }))
            }
            "public_mcp.remote_exec" => {
                let provider = self.clone();
                Some(Box::pin(async move {
                    provider.remote_exec(arguments, context).await
                }))
            }
            "public_mcp.remote_file_write" => {
                let provider = self.clone();
                Some(Box::pin(async move {
                    provider.remote_file_write(arguments, context).await
                }))
            }
            _ => None,
        }
    }
}

fn remote_ops_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "public_mcp.list_sessions",
            "List currently connected SSH terminal sessions exposed by OnetCli.",
            object_schema([]),
        )
        .with_annotations(read_only_annotations("List terminal sessions")),
        Tool::new(
            "public_mcp.session_diagnostics",
            "Report structured diagnostics for a Public MCP terminal session, including connection state and recovery hints.",
            object_schema([("session_id", string_schema())]),
        )
        .with_annotations(read_only_annotations("Session diagnostics")),
        Tool::new(
            "public_mcp.remote_command_poll",
            "Poll the status of a background remote command, including running state, exit code and output byte counters.",
            object_schema([("command_id", string_schema())]),
        )
        .with_annotations(read_only_annotations("Poll remote command")),
        Tool::new(
            "public_mcp.remote_command_output",
            "Read stdout and stderr of a background remote command by byte offset.",
            output_schema(),
        )
        .with_annotations(read_only_annotations("Read remote command output")),
        Tool::new(
            "public_mcp.remote_command_cancel",
            "Request cancellation of a background remote command.",
            cancel_schema(),
        )
        .with_annotations(destructive_annotations("Cancel remote command")),
        Tool::new(
            "public_mcp.remote_exec",
            "Run a non-interactive command on an exposed connected SSH session and return stdout, stderr, exit code, duration and timeout state.",
            exec_schema(),
        )
        .with_annotations(destructive_annotations("Execute remote command")),
        Tool::new(
            "public_mcp.remote_file_write",
            "Write a file to a remote path through an exposed connected SSH session and return bytes written plus SHA-256.",
            file_write_schema(),
        )
        .with_annotations(destructive_annotations("Write remote file")),
    ]
}

fn read_only_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn destructive_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(false)
        .destructive(true)
        .idempotent(false)
        .open_world(true)
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

fn permission_denied_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": "permission_denied",
        "message": message.into()
    }))
}

fn internal_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(std::borrow::Cow::Owned(error.to_string()), None)
}

/// 保守的高风险命令判定，仅用于审批/审计提示，不是安全沙箱。
fn classify_command_risk(command: &str) -> &'static str {
    let lowered = command.to_ascii_lowercase();
    const HIGH_RISK_TOKENS: &[&str] = &[
        "rm ",
        "mkfs",
        " dd ",
        "systemctl stop",
        "systemctl restart",
        "docker system prune",
        "docker rm",
        "mv /var/lib",
        "chmod -r",
        "chown -r",
    ];
    if HIGH_RISK_TOKENS.iter().any(|token| lowered.contains(token)) {
        "high"
    } else {
        "normal"
    }
}

/// 远程文件路径风险分级，用于审批提示。
fn classify_path_risk(path: &str) -> &'static str {
    const HIGH_RISK_PREFIXES: &[&str] =
        &["/etc/", "/var/lib/", "/usr/", "/bin/", "/sbin/", "/root/"];
    const MEDIUM_RISK_PREFIXES: &[&str] = &["/data/", "/var/tmp/"];
    if HIGH_RISK_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        "high"
    } else if MEDIUM_RISK_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        "medium"
    } else {
        "low"
    }
}

#[allow(dead_code)]
const UNUSED_OUTPUT_LIMIT: usize = DEFAULT_OUTPUT_LIMIT_BYTES;
