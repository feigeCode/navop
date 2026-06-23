use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "onetcli";
#[cfg(not(windows))]
const LAUNCHER_BIN_NAME: &str = "onetcli-public-mcp";
#[cfg(windows)]
const LAUNCHER_BIN_NAME: &str = "onetcli-public-mcp.exe";
const MCP_HELPER_EXTENSION_DIR: &str = "mcp_helpers";
const MCP_HELPER_EXTENSION_ID: &str = "onetcli-public-mcp";
const CODEX_BEGIN: &str = "# BEGIN ONETCLI PUBLIC MCP";
const CODEX_END: &str = "# END ONETCLI PUBLIC MCP";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfigInstall {
    pub launcher_path: PathBuf,
    pub discovery_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientConfigHealth {
    UpToDate,
    NotInstalled,
    NeedsRepair,
    MissingHelper,
    UnusableHelper,
}

impl ClientConfigInstall {
    pub fn from_current_app() -> Result<Self> {
        Ok(Self::from_helper_path(
            default_helper_install_path(),
            crate::discovery::public_mcp_discovery_path(),
        ))
    }

    pub fn from_helper_path(
        launcher_path: impl Into<PathBuf>,
        discovery_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            launcher_path: launcher_path.into(),
            discovery_path: discovery_path.into(),
        }
    }
}

pub fn default_helper_install_path() -> PathBuf {
    helper_install_path_from_data_dir(default_one_hub_config_dir())
}

pub fn helper_install_path_from_data_dir(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir
        .as_ref()
        .join("extensions")
        .join(MCP_HELPER_EXTENSION_DIR)
        .join(MCP_HELPER_EXTENSION_ID)
        .join(LAUNCHER_BIN_NAME)
}

fn default_one_hub_config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        return dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("one-hub");
    }
    dirs::home_dir()
        .map(|home| home.join(".config").join("one-hub"))
        .unwrap_or_else(|| std::env::temp_dir().join("one-hub"))
}

pub fn codex_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

pub fn claude_desktop_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Claude").join("claude_desktop_config.json"))
}

pub fn install_codex_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let existing = read_optional_text(path)?;
    let without_managed_block = remove_codex_managed_block(&existing);
    let mut next = without_managed_block.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(&codex_managed_block(install));
    write_user_only_file(path, next.into_bytes())
}

pub fn install_claude_desktop_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    let server = expected_claude_server_config(install);

    match root
        .entry("mcpServers".to_string())
        .or_insert_with(|| json!({}))
    {
        Value::Object(servers) => {
            servers.insert(SERVER_NAME.to_string(), server);
        }
        _ => bail!("Claude config field `mcpServers` must be a JSON object"),
    }

    write_user_only_file(path, serde_json::to_vec_pretty(&Value::Object(root))?)
}

pub fn inspect_codex_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = helper_unavailable_health(&install.launcher_path)? {
        return Ok(health);
    }

    let text = read_optional_text(path)?;
    if text.trim().is_empty() || !text.contains(CODEX_BEGIN) {
        return Ok(ClientConfigHealth::NotInstalled);
    }
    if text.contains(&codex_managed_block(install)) {
        return Ok(ClientConfigHealth::UpToDate);
    }
    Ok(ClientConfigHealth::NeedsRepair)
}

pub fn inspect_claude_desktop_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = helper_unavailable_health(&install.launcher_path)? {
        return Ok(health);
    }

    let root = read_optional_json_object(path)?;
    let Some(server) = root
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(SERVER_NAME))
    else {
        return Ok(ClientConfigHealth::NotInstalled);
    };

    if server == &expected_claude_server_config(install) {
        return Ok(ClientConfigHealth::UpToDate);
    }
    Ok(ClientConfigHealth::NeedsRepair)
}

fn codex_managed_block(install: &ClientConfigInstall) -> String {
    format!(
        "{CODEX_BEGIN}\n[mcp_servers.{SERVER_NAME}]\ncommand = {}\nargs = [\"--discovery\", {}]\n{CODEX_END}\n",
        toml_string(&install.launcher_path),
        toml_string(&install.discovery_path)
    )
}

fn expected_claude_server_config(install: &ClientConfigInstall) -> Value {
    json!({
        "command": path_string(&install.launcher_path),
        "args": ["--discovery", path_string(&install.discovery_path)]
    })
}

pub fn helper_unavailable_health(path: &Path) -> Result<Option<ClientConfigHealth>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(ClientConfigHealth::MissingHelper));
        }
        Err(error) => return Err(error).with_context(|| format!("metadata {}", path.display())),
    };
    if !metadata.is_file() {
        return Ok(Some(ClientConfigHealth::MissingHelper));
    }
    if !helper_is_executable(&metadata) {
        return Ok(Some(ClientConfigHealth::UnusableHelper));
    }
    Ok(None)
}

/// 移除 Codex 配置中的托管 block；若文件仅剩托管配置则删除文件。
pub fn uninstall_codex_config(path: &Path) -> Result<()> {
    let existing = read_optional_text(path)?;
    let cleaned = remove_codex_managed_block(&existing).trim().to_string();
    if cleaned.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_user_only_file(path, cleaned.into_bytes())
}

/// 从 Claude Desktop 配置中移除 onetcli MCP server 条目；若文件仅剩该条目且无其他内容则删除文件。
pub fn uninstall_claude_desktop_config(path: &Path) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        servers.remove(SERVER_NAME);
        if servers.is_empty() {
            root.remove("mcpServers");
        }
    }
    if root.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_user_only_file(path, serde_json::to_vec_pretty(&Value::Object(root))?)
}

#[cfg(unix)]
fn helper_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn helper_is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

fn remove_codex_managed_block(text: &str) -> String {
    let mut output = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == CODEX_BEGIN {
            skipping = true;
            continue;
        }
        if skipping {
            if trimmed == CODEX_END {
                skipping = false;
            }
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn read_optional_text(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn read_optional_json_object(path: &Path) -> Result<Map<String, Value>> {
    let text = read_optional_text(path)?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text)? {
        Value::Object(object) => Ok(object),
        _ => bail!("Claude config root must be a JSON object"),
    }
}

fn toml_string(path: &Path) -> String {
    serde_json::to_string(&path_string(path)).expect("path string should serialize")
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn write_user_only_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(&tmp_path)?;
    file.write_all(&bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }

    fs::rename(tmp_path, path)?;
    Ok(())
}
