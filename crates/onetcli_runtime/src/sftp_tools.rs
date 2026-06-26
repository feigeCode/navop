#[cfg(test)]
mod tests;

use base64::{Engine as _, engine::general_purpose};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, JumpServerConfig, ProxyConfig,
    ProxyType as StorageProxyType, SshAuthMethod, SshParams, StoredConnection,
};
use serde_json::{Value, json};
use sftp::{RusshSftpClient, SftpClient};
use ssh::{JumpServerConnectConfig, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

const DEFAULT_MAX_READ_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum SftpTool {
    List,
    Read,
    Write,
}

#[derive(Clone)]
struct SftpToolHandler {
    repo: Arc<ConnectionRepository>,
    tool: SftpTool,
}

pub fn sftp_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(SftpToolHandler::new(repo.clone(), SftpTool::List)),
        Arc::new(SftpToolHandler::new(repo.clone(), SftpTool::Read)),
        Arc::new(SftpToolHandler::new(repo, SftpTool::Write)),
    ])
}

impl SftpToolHandler {
    fn new(repo: Arc<ConnectionRepository>, tool: SftpTool) -> Self {
        Self { repo, tool }
    }

    async fn call_tool(&self, input: Value) -> Result<ToolResult, ToolError> {
        let path = optional_str(&input, "path").unwrap_or(".").to_string();
        let config = self.ssh_config(&input)?;
        let mut client = RusshSftpClient::connect(config).await.map_err(tool_error)?;

        match self.tool {
            SftpTool::List => {
                let entries = client.list_dir(&path).await.map_err(tool_error)?;
                Ok(ToolResult::structured(json!({
                    "path": path,
                    "entries": entries.into_iter().map(file_entry_json).collect::<Vec<_>>()
                })))
            }
            SftpTool::Read => {
                let max_bytes =
                    optional_usize(&input, "max_bytes").unwrap_or(DEFAULT_MAX_READ_BYTES);
                let content = client
                    .read_file(&path, max_bytes)
                    .await
                    .map_err(tool_error)?;
                Ok(ToolResult::structured(json!({
                    "path": path,
                    "bytes_read": content.len(),
                    "content_base64": general_purpose::STANDARD.encode(content)
                })))
            }
            SftpTool::Write => {
                let content = required_base64(&input, "content_base64")?;
                client
                    .write_file(&path, &content)
                    .await
                    .map_err(tool_error)?;
                Ok(ToolResult::structured(json!({
                    "path": path,
                    "bytes_written": content.len()
                })))
            }
        }
    }

    fn ssh_config(&self, input: &Value) -> Result<SshConnectConfig, ToolError> {
        let connection = required_str(input, "connection")?;
        let stored = self.find_connection(connection)?;
        if stored.connection_type != ConnectionType::SshSftp {
            return Err(ToolError::Failed {
                message: format!("connection is not ssh_sftp: {connection}"),
            });
        }
        let params = stored.to_ssh_params().map_err(tool_error)?;
        Ok(ssh_config_from_params(&params))
    }

    fn find_connection(&self, connection: &str) -> Result<StoredConnection, ToolError> {
        if let Ok(id) = connection.parse::<i64>() {
            return self
                .repo
                .get(id)
                .map_err(tool_error)?
                .ok_or_else(|| unknown_connection(connection));
        }
        self.repo
            .list()
            .map_err(tool_error)?
            .into_iter()
            .find(|stored| stored.name == connection)
            .ok_or_else(|| unknown_connection(connection))
    }
}

impl ToolHandler for SftpToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description) = match self.tool {
            SftpTool::List => (
                "sftp.list",
                "List SFTP directory",
                "List files and directories at a remote path through a saved SSH/SFTP connection. Use this for remote filesystem browsing. The connection argument accepts a saved connection id or exact saved connection name; path defaults to \".\".",
            ),
            SftpTool::Read => (
                "sftp.read",
                "Read SFTP file",
                "Read a remote file through a saved SSH/SFTP connection and return content_base64 plus bytes_read. Use this for remote file contents instead of ssh.remote_exec with cat/base64. The connection argument accepts a saved connection id or exact saved connection name.",
            ),
            SftpTool::Write => (
                "sftp.write",
                "Write SFTP file",
                "Write bytes to a remote file through a saved SSH/SFTP connection using content_base64. Use this for remote file creation or replacement instead of ssh.remote_exec shell redirection. The connection argument accepts a saved connection id or exact saved connection name.",
            ),
        };
        ToolDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            input_schema: input_schema(self.tool),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
                ToolAdapter::Cli,
            ],
            annotations: annotations(title, self.tool),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input).await })
    }
}

fn ssh_config_from_params(params: &SshParams) -> SshConnectConfig {
    SshConnectConfig {
        host: params.host.clone(),
        port: params.port,
        username: params.username.clone(),
        auth: auth_from_method(&params.auth_method),
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params.jump_server.as_ref().map(jump_config),
        proxy: params.proxy.as_ref().map(proxy_config),
        keyboard_interactive_responder: None,
    }
}

fn auth_from_method(auth: &SshAuthMethod) -> SshAuth {
    match auth {
        SshAuthMethod::Password { password } => SshAuth::Password(password.clone()),
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path: key_path.clone(),
            passphrase: passphrase.clone(),
            certificate_path: None,
        },
        SshAuthMethod::Agent => SshAuth::Agent,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

fn jump_config(jump: &JumpServerConfig) -> JumpServerConnectConfig {
    JumpServerConnectConfig {
        host: jump.host.clone(),
        port: jump.port,
        username: jump.username.clone(),
        auth: auth_from_method(&jump.auth_method),
    }
}

fn proxy_config(proxy: &ProxyConfig) -> ProxyConnectConfig {
    let proxy_type = match proxy.proxy_type {
        StorageProxyType::Socks5 => ProxyType::Socks5,
        StorageProxyType::Http => ProxyType::Http,
    };
    ProxyConnectConfig {
        proxy_type,
        host: proxy.host.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    }
}

fn file_entry_json(entry: sftp::FileEntry) -> Value {
    let modified_unix_secs = entry
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    json!({
        "name": entry.name,
        "path": entry.path,
        "size": entry.size,
        "modified_unix_secs": modified_unix_secs,
        "is_dir": entry.is_dir,
        "permissions": entry.permissions
    })
}

fn required_str<'a>(input: &'a Value, field: &'static str) -> Result<&'a str, ToolError> {
    optional_str(input, field).ok_or_else(|| ToolError::Failed {
        message: format!("missing string argument: {field}"),
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

fn annotations(title: &str, tool: SftpTool) -> ToolAnnotations {
    match tool {
        SftpTool::List | SftpTool::Read => ToolAnnotations::read_only(title),
        SftpTool::Write => ToolAnnotations::mutating(title),
    }
}

fn input_schema(tool: SftpTool) -> Value {
    match tool {
        SftpTool::List => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "path": path_schema("Remote directory path to list. Defaults to \".\".")
            },
            "required": ["connection"]
        }),
        SftpTool::Read => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "path": path_schema("Remote file path to read. Defaults to \".\"."),
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum number of bytes to read before truncating the response. Defaults to 1048576."
                }
            },
            "required": ["connection"]
        }),
        SftpTool::Write => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "path": path_schema("Remote file path to create or replace. Defaults to \".\"."),
                "content_base64": {
                    "type": "string",
                    "description": "Base64-encoded file bytes to write to the remote path."
                }
            },
            "required": ["connection", "content_base64"]
        }),
    }
}

fn connection_schema() -> Value {
    json!({
        "type": "string",
        "description": "Saved SSH/SFTP connection id or exact saved connection name."
    })
}

fn path_schema(description: &'static str) -> Value {
    json!({
        "type": "string",
        "description": description
    })
}

fn unknown_connection(connection: &str) -> ToolError {
    ToolError::Failed {
        message: format!("unknown connection: {connection}"),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
