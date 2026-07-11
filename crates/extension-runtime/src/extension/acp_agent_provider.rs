use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

mod config;

pub use config::{AcpAgentExtensionAuth, AcpAgentExtensionAuthMethod, AcpAgentExtensionTimeouts};

const MANIFEST_FILE: &str = "acp_agent.json";

pub struct AcpAgentExtensionProvider;

impl AcpAgentExtensionProvider {
    pub fn load_agents_from_root(root: &Path) -> Result<Vec<AcpAgentExtensionAgent>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut agents = Vec::new();
        for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let manifest = load_manifest(&entry.path())?;
                agents.extend(manifest.agents);
            }
        }
        Ok(agents)
    }
}

impl ExtensionProvider for AcpAgentExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::AcpAgent
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                summaries.push(to_summary(&load_manifest(&entry.path())?));
            }
        }
        Ok(summaries)
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let manifest = load_manifest(dir)?;
        Ok(to_summary(&manifest))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("无效的 ACP agent 目录名: {}", dir.display()))?;
        std::fs::remove_dir_all(dir)
            .with_context(|| format!("删除 ACP agent 目录 {}", dir.display()))?;
        Ok(name)
    }
}

#[derive(Debug, Deserialize)]
struct AcpAgentManifest {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    agents: Vec<AcpAgentExtensionAgent>,
    #[serde(skip)]
    manifest_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionAgent {
    #[serde(skip)]
    pub extension_id: String,
    pub id: String,
    pub name: String,
    pub transport: AcpAgentExtensionTransport,
    #[serde(default)]
    pub auth: AcpAgentExtensionAuth,
    #[serde(default)]
    pub timeouts: AcpAgentExtensionTimeouts,
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

impl AcpAgentExtensionAgent {
    pub fn stdio(
        extension_id: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        manifest_dir: PathBuf,
        command: impl Into<String>,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            id: id.into(),
            name: name.into(),
            transport: AcpAgentExtensionTransport::Stdio {
                command: command.into(),
                args,
                env,
            },
            auth: AcpAgentExtensionAuth::default(),
            timeouts: AcpAgentExtensionTimeouts::default(),
            manifest_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpAgentExtensionTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
    },
}

fn load_manifest(dir: &Path) -> Result<AcpAgentManifest> {
    let manifest_path = dir.join(MANIFEST_FILE);
    if !manifest_path.exists() {
        anyhow::bail!("未在 {} 找到 {MANIFEST_FILE}", dir.display());
    }
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let mut manifest: AcpAgentManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("解析 {MANIFEST_FILE} 失败: {}", manifest_path.display()))?;
    validate_manifest(&manifest, dir)?;
    manifest.manifest_dir = dir.to_path_buf();
    for agent in &mut manifest.agents {
        agent.extension_id = manifest.id.clone();
        agent.manifest_dir = dir.to_path_buf();
    }
    Ok(manifest)
}

fn validate_manifest(manifest: &AcpAgentManifest, dir: &Path) -> Result<()> {
    if manifest.id.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 id");
    }
    if manifest.name.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 name");
    }
    if manifest.agents.is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 agents");
    }
    for agent in &manifest.agents {
        validate_agent(agent, dir)?;
    }
    Ok(())
}

fn validate_agent(agent: &AcpAgentExtensionAgent, dir: &Path) -> Result<()> {
    if agent.id.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} agents[].id 不能为空");
    }
    if agent.name.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} agents[].name 不能为空");
    }
    match &agent.transport {
        AcpAgentExtensionTransport::Stdio { command, .. } => {
            validate_entry_command(dir, command.trim())?;
        }
        AcpAgentExtensionTransport::Http { url } => {
            if url.trim().is_empty() {
                anyhow::bail!("{MANIFEST_FILE} http transport 缺少 url");
            }
        }
    }
    agent.auth.validate()?;
    agent.timeouts.validate()?;
    Ok(())
}

fn validate_entry_command(dir: &Path, command: &str) -> Result<()> {
    let command_path = Path::new(command);
    if command_path.is_absolute() || command_path.components().any(reject_component) {
        anyhow::bail!("{MANIFEST_FILE} stdio command must stay inside package: {command}");
    }
    let resolved = dir.join(command_path);
    if !resolved.is_file() {
        anyhow::bail!(
            "{MANIFEST_FILE} stdio command does not exist or is not a file: {}",
            resolved.display()
        );
    }
    validate_command_is_executable(&resolved)?;
    Ok(())
}

fn reject_component(component: Component<'_>) -> bool {
    matches!(component, Component::ParentDir | Component::Prefix(_))
}

#[cfg(unix)]
fn validate_command_is_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .with_context(|| format!("metadata {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        anyhow::bail!(
            "{MANIFEST_FILE} stdio command is not executable: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_command_is_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn to_summary(manifest: &AcpAgentManifest) -> ExtensionSummary {
    let description = if manifest.description.trim().is_empty() {
        format!("{} ACP agent", manifest.name)
    } else {
        manifest.description.clone()
    };
    let version = if manifest.version.trim().is_empty() {
        "0.0.0".to_string()
    } else {
        manifest.version.clone()
    };
    ExtensionSummary::new(
        ExtensionKind::AcpAgent,
        manifest.id.clone(),
        version,
        manifest.manifest_dir.clone(),
    )
    .with_description(description)
}
