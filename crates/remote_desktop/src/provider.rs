use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{RemoteDesktopCapabilities, RemoteDesktopProtocol};

pub const PROVIDER_MANIFEST_FILE: &str = "remote_desktop_provider.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteDesktopProviderManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    pub protocol: RemoteDesktopProtocol,
    pub entry: RemoteDesktopProviderEntry,
    pub capabilities: RemoteDesktopCapabilities,
    #[serde(default)]
    pub ui: RemoteDesktopProviderUi,
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteDesktopProviderEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteDesktopProviderUi {
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub default_port: Option<u16>,
}

impl RemoteDesktopProviderManifest {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_id(&self.id)?;
        require_non_empty(&self.name, "remote desktop provider name")?;
        require_non_empty(&self.entry.command, "remote desktop provider command")?;
        Ok(())
    }

    pub fn command_working_dir(&self) -> PathBuf {
        self.entry
            .working_dir
            .as_deref()
            .map(|dir| self.manifest_dir.join(dir))
            .unwrap_or_else(|| self.manifest_dir.clone())
    }
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    require_non_empty(id, "remote desktop provider id")?;
    if id == "." || id == ".." || id.contains('/') || id.contains('\\') {
        anyhow::bail!("remote desktop provider id contains path separators: {id}");
    }
    Ok(())
}

fn require_non_empty(value: &str, field: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{field} is required");
    }
    Ok(())
}
