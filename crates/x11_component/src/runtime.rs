use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x11_forwarding::{MagicCookie, ServerEndpoint};

use crate::X11ComponentError;
use crate::paths::atomic_write_private;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistentEndpoint {
    Unix { path: PathBuf },
    Inet { host: String, port: u16 },
}

impl PersistentEndpoint {
    pub fn to_endpoint(&self) -> ServerEndpoint {
        match self {
            Self::Unix { path } => ServerEndpoint::Unix(path.clone()),
            Self::Inet { host, port } => ServerEndpoint::Inet {
                host: host.clone(),
                port: *port,
            },
        }
    }

    pub fn from_endpoint(endpoint: &ServerEndpoint) -> Self {
        match endpoint {
            ServerEndpoint::Unix(path) => Self::Unix { path: path.clone() },
            ServerEndpoint::Inet { host, port } => Self::Inet {
                host: host.clone(),
                port: *port,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub endpoint: PersistentEndpoint,
    cookie: String,
}

impl RuntimeIdentity {
    #[cfg(test)]
    pub fn loopback(port: u16, cookie: MagicCookie) -> Self {
        Self {
            endpoint: PersistentEndpoint::Inet {
                host: "127.0.0.1".into(),
                port,
            },
            cookie: cookie.hex(),
        }
    }

    pub fn new(endpoint: &ServerEndpoint, cookie: &MagicCookie) -> Self {
        Self {
            endpoint: PersistentEndpoint::from_endpoint(endpoint),
            cookie: cookie.hex(),
        }
    }

    pub fn read(path: &Path) -> Result<Self, X11ComponentError> {
        let bytes = std::fs::read(path).map_err(config_error)?;
        let identity: Self = serde_json::from_slice(&bytes)
            .map_err(|error| X11ComponentError::Config(error.to_string()))?;
        identity.validate()?;
        // 旧版本可能留下 0644 文件，读取时立即收紧权限。
        identity.write(path)?;
        Ok(identity)
    }

    pub fn write(&self, path: &Path) -> Result<(), X11ComponentError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| X11ComponentError::Config(error.to_string()))?;
        atomic_write_private(path, &bytes)
    }

    #[cfg(test)]
    pub fn cookie_hex(&self) -> &str {
        &self.cookie
    }

    pub fn cookie(&self) -> Result<MagicCookie, X11ComponentError> {
        MagicCookie::from_hex(&self.cookie)
            .map_err(|error| X11ComponentError::Config(error.to_string()))
    }

    fn validate(&self) -> Result<(), X11ComponentError> {
        if let PersistentEndpoint::Inet { host, .. } = &self.endpoint {
            if host != "127.0.0.1" {
                return Err(X11ComponentError::Config("持久端点不是回环地址".into()));
            }
        }
        self.cookie().map(|_| ())
    }
}

fn config_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Config(error.to_string())
}

#[cfg(test)]
mod tests;
