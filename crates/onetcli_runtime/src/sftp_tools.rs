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
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};
use tool_runtime::{
    ResourceCapability, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

const DEFAULT_MAX_READ_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum SftpTool {
    List,
    Read,
    Write,
    Stat,
    Upload,
    Download,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OverwritePolicy {
    Fail,
    Overwrite,
    Skip,
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
        Arc::new(SftpToolHandler::new(repo.clone(), SftpTool::Write)),
        Arc::new(SftpToolHandler::new(repo.clone(), SftpTool::Stat)),
        Arc::new(SftpToolHandler::new(repo.clone(), SftpTool::Upload)),
        Arc::new(SftpToolHandler::new(repo, SftpTool::Download)),
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
                let remote_stat = client.stat(&path).await.map_err(tool_error)?;
                if prepare_remote_target(
                    &mut client,
                    &path,
                    parse_overwrite_policy(&input)?,
                    remote_stat,
                )
                .await?
                {
                    return Ok(ToolResult::structured(json!({
                        "path": path,
                        "bytes_written": 0,
                        "skipped": true
                    })));
                }
                client
                    .write_file(&path, &content)
                    .await
                    .map_err(tool_error)?;
                Ok(ToolResult::structured(json!({
                    "path": path,
                    "bytes_written": content.len(),
                    "skipped": false
                })))
            }
            SftpTool::Stat => {
                let stat = client.stat(&path).await.map_err(tool_error)?;
                Ok(ToolResult::structured(stat_json(path, stat)))
            }
            SftpTool::Upload => {
                let local_path = required_str(&input, "local_path")?;
                let remote_path = required_str(&input, "remote_path")?;
                upload_path(
                    &mut client,
                    local_path,
                    remote_path,
                    parse_overwrite_policy(&input)?,
                )
                .await
            }
            SftpTool::Download => {
                let remote_path = required_str(&input, "remote_path")?;
                let local_path = required_str(&input, "local_path")?;
                download_path(
                    &mut client,
                    remote_path,
                    local_path,
                    parse_overwrite_policy(&input)?,
                )
                .await
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
                "Read a remote file through a saved SSH/SFTP connection and return content_base64 plus bytes_read. Use this canonical file operation for remote file contents. The connection argument accepts a saved connection id or exact saved connection name.",
            ),
            SftpTool::Write => (
                "sftp.write",
                "Write SFTP file",
                "Write bytes to a remote file through a saved SSH/SFTP connection using content_base64. Use this canonical file operation for remote file creation or replacement. The connection argument accepts a saved connection id or exact saved connection name. on_exists defaults to fail; pass overwrite or skip explicitly.",
            ),
            SftpTool::Stat => (
                "sftp.stat",
                "Check SFTP path",
                "Check whether a remote SFTP path exists and return its type, size, modified time, and permissions when available.",
            ),
            SftpTool::Upload => (
                "sftp.upload",
                "Upload local path over SFTP",
                "Upload a local file or folder to a remote SFTP path. Checks the target first; on_exists defaults to fail so callers must explicitly choose overwrite or skip before replacing an existing target.",
            ),
            SftpTool::Download => (
                "sftp.download",
                "Download remote path over SFTP",
                "Download a remote SFTP file or folder to a local path. Checks the local target first; on_exists defaults to fail so callers must explicitly choose overwrite or skip before replacing an existing target.",
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
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: annotations(title, self.tool),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input).await })
    }

    fn target_spec(&self) -> ToolTargetSpec {
        ToolTargetSpec::required_with_capabilities(Vec::new(), vec![self.tool.capability()])
    }
}

impl SftpTool {
    fn capability(self) -> ResourceCapability {
        match self {
            SftpTool::List => ResourceCapability::List,
            SftpTool::Read | SftpTool::Stat | SftpTool::Download => ResourceCapability::ReadFile,
            SftpTool::Write | SftpTool::Upload => ResourceCapability::WriteFile,
        }
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
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => SshAuth::PrivateKeyContent {
            private_key: private_key.clone(),
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

fn path_metadata_json(metadata: sftp::PathMetadata) -> Value {
    let modified_unix_secs = metadata
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    json!({
        "size": metadata.size,
        "modified_unix_secs": modified_unix_secs,
        "is_dir": metadata.is_dir,
        "permissions": metadata.permissions
    })
}

fn stat_json(path: String, metadata: Option<sftp::PathMetadata>) -> Value {
    match metadata {
        Some(metadata) => json!({
            "path": path,
            "exists": true,
            "metadata": path_metadata_json(metadata)
        }),
        None => json!({
            "path": path,
            "exists": false
        }),
    }
}

async fn upload_path(
    client: &mut RusshSftpClient,
    local_path: &str,
    remote_path: &str,
    policy: OverwritePolicy,
) -> Result<ToolResult, ToolError> {
    let metadata = std::fs::metadata(local_path).map_err(tool_error)?;
    let remote_stat = client.stat(remote_path).await.map_err(tool_error)?;
    if prepare_remote_target(client, remote_path, policy, remote_stat).await? {
        return Ok(skipped_result("upload", local_path, remote_path));
    }

    let progress = Box::new(|_| {});
    let cancelled = Arc::new(AtomicBool::new(false));
    if metadata.is_dir() {
        client
            .upload_dir_with_progress(local_path, remote_path, cancelled, progress)
            .await
            .map_err(tool_error)?;
    } else {
        client
            .upload_with_progress(local_path, remote_path, cancelled, progress)
            .await
            .map_err(tool_error)?;
    }

    Ok(ToolResult::structured(json!({
        "operation": "upload",
        "local_path": local_path,
        "remote_path": remote_path,
        "kind": path_kind(metadata.is_dir()),
        "bytes": metadata.len(),
        "skipped": false
    })))
}

async fn download_path(
    client: &mut RusshSftpClient,
    remote_path: &str,
    local_path: &str,
    policy: OverwritePolicy,
) -> Result<ToolResult, ToolError> {
    let remote_stat = client
        .stat(remote_path)
        .await
        .map_err(tool_error)?
        .ok_or_else(|| remote_not_found(remote_path))?;
    if prepare_local_target(local_path, policy)? {
        return Ok(skipped_result("download", local_path, remote_path));
    }

    ensure_local_parent(local_path)?;
    let progress = Box::new(|_| {});
    let cancelled = Arc::new(AtomicBool::new(false));
    if remote_stat.is_dir {
        client
            .download_dir_with_progress(remote_path, local_path, cancelled, progress)
            .await
            .map_err(tool_error)?;
    } else {
        client
            .download_with_progress(remote_path, local_path, cancelled, progress)
            .await
            .map_err(tool_error)?;
    }

    Ok(ToolResult::structured(json!({
        "operation": "download",
        "remote_path": remote_path,
        "local_path": local_path,
        "kind": path_kind(remote_stat.is_dir),
        "bytes": remote_stat.size,
        "skipped": false
    })))
}

async fn prepare_remote_target(
    client: &mut RusshSftpClient,
    path: &str,
    policy: OverwritePolicy,
    stat: Option<sftp::PathMetadata>,
) -> Result<bool, ToolError> {
    let Some(stat) = stat else {
        return Ok(false);
    };
    match policy {
        OverwritePolicy::Fail => Err(target_exists(path)),
        OverwritePolicy::Skip => Ok(true),
        OverwritePolicy::Overwrite => {
            let progress = Box::new(|_| {});
            let cancelled = Arc::new(AtomicBool::new(false));
            if stat.is_dir {
                client
                    .delete_recursive(path, cancelled, progress)
                    .await
                    .map_err(tool_error)?;
            } else {
                client.delete(path, false).await.map_err(tool_error)?;
            }
            Ok(false)
        }
    }
}

pub(crate) fn prepare_local_target(path: &str, policy: OverwritePolicy) -> Result<bool, ToolError> {
    let target = Path::new(path);
    if !target.exists() {
        return Ok(false);
    }
    match policy {
        OverwritePolicy::Fail => Err(target_exists(path)),
        OverwritePolicy::Skip => Ok(true),
        OverwritePolicy::Overwrite => {
            remove_local_target(target)?;
            Ok(false)
        }
    }
}

fn remove_local_target(path: &Path) -> Result<(), ToolError> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).map_err(tool_error)
    } else {
        std::fs::remove_file(path).map_err(tool_error)
    }
}

fn skipped_result(operation: &str, local_path: &str, remote_path: &str) -> ToolResult {
    ToolResult::structured(json!({
        "operation": operation,
        "local_path": local_path,
        "remote_path": remote_path,
        "skipped": true
    }))
}

fn ensure_local_parent(path: &str) -> Result<(), ToolError> {
    if let Some(parent) = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(tool_error)?;
    }
    Ok(())
}

fn path_kind(is_dir: bool) -> &'static str {
    if is_dir { "directory" } else { "file" }
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

pub(crate) fn parse_overwrite_policy(input: &Value) -> Result<OverwritePolicy, ToolError> {
    match optional_str(input, "on_exists").unwrap_or("fail") {
        "fail" => Ok(OverwritePolicy::Fail),
        "overwrite" => Ok(OverwritePolicy::Overwrite),
        "skip" => Ok(OverwritePolicy::Skip),
        value => Err(ToolError::Failed {
            message: format!("invalid on_exists: {value}; expected fail, overwrite, or skip"),
        }),
    }
}

fn required_base64(input: &Value, field: &'static str) -> Result<Vec<u8>, ToolError> {
    let encoded = required_str(input, field)?;
    general_purpose::STANDARD
        .decode(encoded)
        .map_err(tool_error)
}

fn annotations(title: &str, tool: SftpTool) -> ToolAnnotations {
    match tool {
        SftpTool::List | SftpTool::Read | SftpTool::Stat => ToolAnnotations::read_only(title),
        SftpTool::Write | SftpTool::Upload | SftpTool::Download => ToolAnnotations::mutating(title),
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
                },
                "on_exists": overwrite_policy_schema("What to do when the remote path already exists. Defaults to fail.")
            },
            "required": ["connection", "content_base64"]
        }),
        SftpTool::Stat => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "path": path_schema("Remote file or directory path to check.")
            },
            "required": ["connection", "path"]
        }),
        SftpTool::Upload => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "local_path": path_schema("Local file or directory path to upload."),
                "remote_path": path_schema("Remote destination file or directory path."),
                "on_exists": overwrite_policy_schema("What to do when the remote destination path already exists. Defaults to fail.")
            },
            "required": ["connection", "local_path", "remote_path"]
        }),
        SftpTool::Download => json!({
            "type": "object",
            "properties": {
                "connection": connection_schema(),
                "remote_path": path_schema("Remote file or directory path to download."),
                "local_path": path_schema("Local destination file or directory path."),
                "on_exists": overwrite_policy_schema("What to do when the local destination path already exists. Defaults to fail.")
            },
            "required": ["connection", "remote_path", "local_path"]
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

fn overwrite_policy_schema(description: &'static str) -> Value {
    json!({
        "type": "string",
        "enum": ["fail", "overwrite", "skip"],
        "description": description
    })
}

fn unknown_connection(connection: &str) -> ToolError {
    ToolError::Failed {
        message: format!("unknown connection: {connection}"),
    }
}

fn remote_not_found(path: &str) -> ToolError {
    ToolError::Failed {
        message: format!("remote path does not exist: {path}"),
    }
}

fn target_exists(path: &str) -> ToolError {
    ToolError::Failed {
        message: format!("target already exists: {path}; set on_exists to overwrite or skip"),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
