mod file_io;

use anyhow::{Result, bail};
use file_io::{read_optional_json_object, read_optional_text, write_user_only_file};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "navop";
const LEGACY_SERVER_NAME: &str = "onetcli";
const PACKAGE_NAME: &str = "@navop/mcp";
pub const RECOMMENDED_PACKAGE_VERSION: &str = "0.1.2";
const CODEX_BEGIN: &str = "# BEGIN NAVOP MCP";
const CODEX_END: &str = "# END NAVOP MCP";
const LEGACY_CODEX_BEGIN: &str = "# BEGIN ONETCLI PUBLIC MCP";
const LEGACY_CODEX_END: &str = "# END ONETCLI PUBLIC MCP";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub package_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfigInstall {
    pub launcher_path: PathBuf,
    pub discovery_path: PathBuf,
    pub launch_spec: McpLaunchSpec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientConfigHealth {
    UpToDate,
    NotInstalled,
    NeedsMigration,
    NeedsRepair,
    PackageVersionOutdated,
    NodeUnavailable,
    NpxUnavailable,
    MissingHelper,
    UnusableHelper,
}

impl ClientConfigInstall {
    pub fn from_current_app() -> Result<Self> {
        let node =
            resolve_program("node").ok_or_else(|| anyhow::anyhow!("Node.js is unavailable"))?;
        let npx = resolve_program("npx").ok_or_else(|| anyhow::anyhow!("npx is unavailable"))?;
        let _ = node;
        Ok(Self::from_npx_path(
            npx,
            crate::discovery::public_mcp_discovery_path(),
            RECOMMENDED_PACKAGE_VERSION,
        ))
    }

    pub fn from_npx_path(
        npx_path: impl Into<PathBuf>,
        discovery_path: impl Into<PathBuf>,
        version: impl Into<String>,
    ) -> Self {
        let launcher_path = npx_path.into();
        let discovery_path = discovery_path.into();
        let version = version.into();
        let args = vec![
            "-y".to_string(),
            format!("{PACKAGE_NAME}@{version}"),
            "mcp".to_string(),
            "--discovery".to_string(),
            path_string(&discovery_path),
        ];
        Self {
            launch_spec: McpLaunchSpec {
                command: path_string(&launcher_path),
                args,
                env: BTreeMap::new(),
                package_version: Some(version),
            },
            launcher_path,
            discovery_path,
        }
    }

    pub fn from_helper_path(
        launcher_path: impl Into<PathBuf>,
        discovery_path: impl Into<PathBuf>,
    ) -> Self {
        let launcher_path = launcher_path.into();
        let discovery_path = discovery_path.into();
        Self {
            launch_spec: McpLaunchSpec {
                command: path_string(&launcher_path),
                args: vec!["--discovery".into(), path_string(&discovery_path)],
                env: BTreeMap::new(),
                package_version: None,
            },
            launcher_path,
            discovery_path,
        }
    }
}

pub fn default_helper_install_path() -> PathBuf {
    resolve_program("npx").unwrap_or_else(|| PathBuf::from("npx"))
}

pub fn helper_install_path_from_data_dir(_data_dir: impl AsRef<Path>) -> PathBuf {
    default_helper_install_path()
}

pub fn codex_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex/config.toml"))
}

pub fn claude_desktop_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Claude/claude_desktop_config.json"))
}

pub fn claude_code_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude.json"))
}

pub fn install_codex_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let existing = read_optional_text(path)?;
    let cleaned = remove_managed_blocks(&existing);
    let mut next = cleaned.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(&codex_managed_block(install));
    write_user_only_file(path, next.into_bytes())
}

pub fn install_claude_desktop_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    install_json_mcp_servers_config(path, install)
}

pub fn install_claude_code_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    install_json_mcp_servers_config(path, install)
}

fn install_json_mcp_servers_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Value::Object(servers) = servers else {
        bail!("Claude config field `mcpServers` must be a JSON object");
    };
    servers.remove(LEGACY_SERVER_NAME);
    servers.insert(SERVER_NAME.into(), expected_server_config(install));
    write_user_only_file(path, serde_json::to_vec_pretty(&Value::Object(root))?)
}

pub fn agent_mcp_config_json(install: &ClientConfigInstall) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "mcpServers": { SERVER_NAME: expected_server_config(install) }
    }))?)
}

pub fn inspect_codex_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = launcher_unavailable_health(install)? {
        return Ok(health);
    }
    let text = read_optional_text(path)?;
    if text.contains(LEGACY_CODEX_BEGIN) || text.contains("[mcp_servers.onetcli]") {
        return Ok(ClientConfigHealth::NeedsMigration);
    }
    if !text.contains(CODEX_BEGIN) {
        return Ok(ClientConfigHealth::NotInstalled);
    }
    classify_config_match(text.contains(&codex_managed_block(install)), &text)
}

pub fn inspect_claude_desktop_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    inspect_json_mcp_servers_config(path, install)
}

pub fn inspect_claude_code_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    inspect_json_mcp_servers_config(path, install)
}

fn inspect_json_mcp_servers_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = launcher_unavailable_health(install)? {
        return Ok(health);
    }
    let root = read_optional_json_object(path)?;
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return Ok(ClientConfigHealth::NotInstalled);
    };
    if servers.contains_key(LEGACY_SERVER_NAME) {
        return Ok(ClientConfigHealth::NeedsMigration);
    }
    let Some(server) = servers.get(SERVER_NAME) else {
        return Ok(ClientConfigHealth::NotInstalled);
    };
    classify_config_match(
        server == &expected_server_config(install),
        &server.to_string(),
    )
}

fn classify_config_match(matches: bool, text: &str) -> Result<ClientConfigHealth> {
    if matches {
        return Ok(ClientConfigHealth::UpToDate);
    }
    if text.contains(PACKAGE_NAME)
        && !text.contains(&format!("{PACKAGE_NAME}@{RECOMMENDED_PACKAGE_VERSION}"))
    {
        return Ok(ClientConfigHealth::PackageVersionOutdated);
    }
    Ok(ClientConfigHealth::NeedsRepair)
}

fn codex_managed_block(install: &ClientConfigInstall) -> String {
    let args = install
        .launch_spec
        .args
        .iter()
        .map(|arg| serde_json::to_string(arg).unwrap())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{CODEX_BEGIN}\n[mcp_servers.{SERVER_NAME}]\ncommand = {}\nargs = [{args}]\n{CODEX_END}\n",
        serde_json::to_string(&install.launch_spec.command).unwrap()
    )
}

fn expected_server_config(install: &ClientConfigInstall) -> Value {
    json!({ "command": install.launch_spec.command, "args": install.launch_spec.args })
}

fn launcher_unavailable_health(
    install: &ClientConfigInstall,
) -> Result<Option<ClientConfigHealth>> {
    if install.launch_spec.package_version.is_some() {
        return helper_unavailable_health(&install.launcher_path)
            .map(|health| health.map(|_| ClientConfigHealth::NpxUnavailable));
    }
    helper_unavailable_health(&install.launcher_path)
}

pub fn helper_unavailable_health(path: &Path) -> Result<Option<ClientConfigHealth>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(ClientConfigHealth::MissingHelper));
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(Some(ClientConfigHealth::MissingHelper));
    }
    if !helper_is_executable(&metadata) {
        return Ok(Some(ClientConfigHealth::UnusableHelper));
    }
    Ok(None)
}

pub fn uninstall_codex_config(path: &Path) -> Result<()> {
    let cleaned = remove_managed_blocks(&read_optional_text(path)?)
        .trim()
        .to_string();
    if cleaned.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_user_only_file(path, cleaned.into_bytes())
}

pub fn uninstall_claude_desktop_config(path: &Path) -> Result<()> {
    uninstall_json_mcp_servers_config(path)
}

pub fn uninstall_claude_code_config(path: &Path) -> Result<()> {
    uninstall_json_mcp_servers_config(path)
}

fn uninstall_json_mcp_servers_config(path: &Path) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        servers.remove(SERVER_NAME);
        servers.remove(LEGACY_SERVER_NAME);
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

fn resolve_program(name: &str) -> Option<PathBuf> {
    let mut candidates = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|dir| dir.join(executable_name(name)))
        .collect::<Vec<_>>();
    for root in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(Path::new(root).join(executable_name(name)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
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

fn remove_managed_blocks(text: &str) -> String {
    remove_block(
        &remove_block(text, CODEX_BEGIN, CODEX_END),
        LEGACY_CODEX_BEGIN,
        LEGACY_CODEX_END,
    )
}

fn remove_block(text: &str, begin: &str, end: &str) -> String {
    let mut output = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim() == begin {
            skipping = true;
            continue;
        }
        if skipping && line.trim() == end {
            skipping = false;
            continue;
        }
        if !skipping {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}
