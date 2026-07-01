use super::config::resolve_ssh_config;
use super::output::{file_entry_json, stat_json, success_json};
use super::{AgentSshTool, AgentSshToolKind, tool_error};
use agent_runtime::{ToolError, ToolObservation, tools::ToolInvocation};
use base64::{Engine as _, engine::general_purpose};
use serde_json::{Value, json};
use sftp::{RusshSftpClient, SftpClient};

const DEFAULT_MAX_READ_BYTES: usize = 1024 * 1024;

impl AgentSshTool {
    pub(super) async fn execute_sftp(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let path = optional_str(&invocation.arguments, "path").unwrap_or(".");
        let config = resolve_ssh_config(&self.repo, &invocation)?;
        let mut client = RusshSftpClient::connect(config).await.map_err(tool_error)?;
        let value = match self.kind {
            AgentSshToolKind::ListDir => {
                let entries = client.list_dir(path).await.map_err(tool_error)?;
                json!({
                    "path": path,
                    "entries": entries.into_iter().map(file_entry_json).collect::<Vec<_>>()
                })
            }
            AgentSshToolKind::ReadFile => {
                let max_bytes = optional_usize(&invocation.arguments, "max_bytes")
                    .unwrap_or(DEFAULT_MAX_READ_BYTES);
                let content = client
                    .read_file(path, max_bytes.min(DEFAULT_MAX_READ_BYTES))
                    .await
                    .map_err(tool_error)?;
                json!({
                    "path": path,
                    "bytes_read": content.len(),
                    "content_base64": general_purpose::STANDARD.encode(content)
                })
            }
            AgentSshToolKind::FileStat => {
                let stat = client.stat(path).await.map_err(tool_error)?;
                stat_json(path.to_string(), stat)
            }
            AgentSshToolKind::WriteFile => write_file(&mut client, &invocation).await?,
        };
        let _ = client.disconnect().await;
        Ok(success_json(invocation, "SSH/SFTP tool executed", value))
    }
}

async fn write_file(
    client: &mut RusshSftpClient,
    invocation: &ToolInvocation,
) -> Result<Value, ToolError> {
    let path = required_str(&invocation.arguments, "path")?.to_string();
    let content = required_base64(&invocation.arguments, "content_base64")?;
    match write_policy(&invocation.arguments)? {
        WritePolicy::Fail if client.stat(&path).await.map_err(tool_error)?.is_some() => {
            return Err(ToolError::Execution(format!(
                "remote path already exists: {path}"
            )));
        }
        WritePolicy::Skip if client.stat(&path).await.map_err(tool_error)?.is_some() => {
            return Ok(json!({"path": path, "bytes_written": 0, "skipped": true}));
        }
        _ => {}
    }
    client
        .write_file(&path, &content)
        .await
        .map_err(tool_error)?;
    Ok(json!({
        "path": path,
        "bytes_written": content.len(),
        "skipped": false
    }))
}

enum WritePolicy {
    Fail,
    Overwrite,
    Skip,
}

fn write_policy(input: &Value) -> Result<WritePolicy, ToolError> {
    match optional_str(input, "on_exists").unwrap_or("fail") {
        "fail" => Ok(WritePolicy::Fail),
        "overwrite" => Ok(WritePolicy::Overwrite),
        "skip" => Ok(WritePolicy::Skip),
        value => Err(ToolError::InvalidArguments(format!(
            "invalid on_exists: {value}; expected fail, overwrite, or skip"
        ))),
    }
}

fn required_str<'a>(input: &'a Value, field: &'static str) -> Result<&'a str, ToolError> {
    optional_str(input, field).ok_or_else(|| {
        ToolError::InvalidArguments(format!("missing required string field `{field}`"))
    })
}

fn optional_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn optional_usize(input: &Value, field: &'static str) -> Option<usize> {
    input
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn required_base64(input: &Value, field: &'static str) -> Result<Vec<u8>, ToolError> {
    let encoded = required_str(input, field)?;
    general_purpose::STANDARD
        .decode(encoded)
        .map_err(tool_error)
}
