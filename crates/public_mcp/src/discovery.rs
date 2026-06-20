use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const DISCOVERY_VERSION: u32 = 1;
const APP_NAME: &str = "onetcli";
const DISCOVERY_FILE_NAME: &str = "public-mcp.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMcpMode {
    Temporary,
    Persistent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryDocument {
    pub version: u32,
    pub app: String,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub mode: PublicMcpMode,
    pub started_at: DateTime<Utc>,
    pub launcher_version: String,
}

impl DiscoveryDocument {
    pub fn new(pid: u32, bind_addr: SocketAddr, token: String, mode: PublicMcpMode) -> Self {
        Self {
            version: DISCOVERY_VERSION,
            app: APP_NAME.to_string(),
            pid,
            host: bind_addr.ip().to_string(),
            port: bind_addr.port(),
            token,
            mode,
            started_at: Utc::now(),
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn socket_addr(&self) -> Result<SocketAddr> {
        Ok(format!("{}:{}", self.host, self.port).parse()?)
    }
}

pub fn public_mcp_discovery_path() -> PathBuf {
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir);
    base.join(APP_NAME).join(DISCOVERY_FILE_NAME)
}

pub fn read_discovery(path: &Path) -> Result<DiscoveryDocument> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

pub fn write_discovery(path: &Path, document: &DiscoveryDocument) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("json.tmp");
    write_user_only_file(&tmp_path, serde_json::to_vec_pretty(document)?)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub fn remove_discovery(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_user_only_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(&bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}
