use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const DISCOVERY_VERSION: u32 = 1;
const APP_NAME: &str = "navop";
const LEGACY_APP_NAME: &str = "onetcli";
const DISCOVERY_FILE_NAME: &str = "public-mcp.json";
const TOKEN_HEX_LEN: usize = 64;

/// 宿主产品注入的发现文件身份。三产品各自使用不同的 `app` 名称，
/// 但发现文件仍落在各自的数据目录里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoveryIdentity {
    pub app: &'static str,
    pub legacy: Option<&'static str>,
    pub file_name: &'static str,
}

impl DiscoveryIdentity {
    pub const NAVOP: Self = Self {
        app: APP_NAME,
        legacy: Some(LEGACY_APP_NAME),
        file_name: DISCOVERY_FILE_NAME,
    };

    pub fn accepts(&self, app: &str) -> bool {
        app == self.app || self.legacy == Some(app)
    }
}

static DISCOVERY_IDENTITY: OnceLock<DiscoveryIdentity> = OnceLock::new();

/// 覆盖发现文件身份。只在进程启动早期调用一次；重复调用返回 `false`。
pub fn configure_discovery_identity(identity: DiscoveryIdentity) -> bool {
    DISCOVERY_IDENTITY.set(identity).is_ok()
}

fn discovery_identity() -> DiscoveryIdentity {
    DISCOVERY_IDENTITY
        .get()
        .copied()
        .unwrap_or(DiscoveryIdentity::NAVOP)
}

/// 宿主注入的发现文件路径（正式、开发、便携必须各自落在自己的目录里）。
///
/// `dirs::config_dir()` 对已安装版和开发版返回同一个目录，而发现文件里有
/// pid、端口与令牌：开发版一旦发布到正式版的位置，外部 MCP 客户端就会被
/// 劫持到开发实例上（2026-10-28 事故）。宿主解析出运行模式后必须调用
/// [`set_public_mcp_discovery_paths`] 注入隔离后的路径。
static DISCOVERY_PATH_OVERRIDE: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

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
            app: discovery_identity().app.to_string(),
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

    pub fn legacy_compatible(&self) -> Self {
        let identity = discovery_identity();
        let mut document = self.clone();
        document.app = identity.legacy.unwrap_or(identity.app).to_string();
        document
    }

    pub fn validate_for_stdio_bridge(&self) -> Result<()> {
        if self.version != DISCOVERY_VERSION {
            bail!(
                "unsupported public MCP discovery version {} (expected {})",
                self.version,
                DISCOVERY_VERSION
            );
        }
        if !discovery_identity().accepts(&self.app) {
            bail!("unexpected app `{}` in public MCP discovery", self.app);
        }
        let addr = self.socket_addr()?;
        if !addr.ip().is_loopback() {
            bail!(
                "public MCP discovery must point to a loopback address, got {}",
                addr.ip()
            );
        }
        if !is_valid_token(&self.token) {
            bail!("invalid token in public MCP discovery");
        }
        Ok(())
    }
}

/// 覆盖发现文件位置。只在进程启动早期调用一次；重复调用返回 `false`，
/// 已生效的路径不会被改掉。
pub fn set_public_mcp_discovery_paths(
    canonical: impl Into<PathBuf>,
    legacy: impl Into<PathBuf>,
) -> bool {
    DISCOVERY_PATH_OVERRIDE
        .set((canonical.into(), legacy.into()))
        .is_ok()
}

pub fn public_mcp_discovery_path() -> PathBuf {
    match DISCOVERY_PATH_OVERRIDE.get() {
        Some((canonical, _)) => canonical.clone(),
        None => public_mcp_discovery_path_from(&default_config_dir()),
    }
}

pub fn legacy_public_mcp_discovery_path() -> PathBuf {
    match DISCOVERY_PATH_OVERRIDE.get() {
        Some((_, legacy)) => legacy.clone(),
        None => legacy_public_mcp_discovery_path_from(&default_config_dir()),
    }
}

/// 由运行模式的数据目录推导发现文件位置。
///
/// 正式版的数据目录是 `~/Library/Application Support/navop`，推导结果与历史
/// 安装位置完全一致（`.../navop/public-mcp.json` 与 `.../onetcli/public-mcp.json`）；
/// 开发/便携版的数据根不同，推导结果自然落到各自的目录，不会碰到正式版。
pub fn public_mcp_discovery_paths_in(data_dir: &Path) -> (PathBuf, PathBuf) {
    let identity = discovery_identity();
    let canonical = data_dir.join(identity.file_name);
    let legacy_root = match identity.legacy {
        Some(legacy) => match data_dir.parent() {
            Some(parent) => parent.join(legacy),
            None => data_dir.join(legacy),
        },
        None => data_dir.to_path_buf(),
    };
    (canonical, legacy_root.join(identity.file_name))
}

pub fn public_mcp_discovery_path_from(base: &Path) -> PathBuf {
    let identity = discovery_identity();
    base.join(identity.app).join(identity.file_name)
}

pub fn legacy_public_mcp_discovery_path_from(base: &Path) -> PathBuf {
    let identity = discovery_identity();
    base.join(identity.legacy.unwrap_or(identity.app))
        .join(identity.file_name)
}

fn default_config_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
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

fn is_valid_token(token: &str) -> bool {
    token.len() == TOKEN_HEX_LEN && token.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_identity_accepts_only_its_own_app() {
        let identity = DiscoveryIdentity {
            app: "navop-data",
            legacy: None,
            file_name: "public-mcp.json",
        };
        assert!(identity.accepts("navop-data"));
        assert!(!identity.accepts("navop"));
        assert!(!identity.accepts("navop-shell"));
    }

    #[test]
    fn full_identity_keeps_legacy_onetcli_compatibility() {
        assert!(DiscoveryIdentity::NAVOP.accepts("navop"));
        assert!(DiscoveryIdentity::NAVOP.accepts("onetcli"));
        assert!(!DiscoveryIdentity::NAVOP.accepts("navop-data"));
    }
}
