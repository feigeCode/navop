use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDesktopProtocol {
    Rdp,
    Vnc,
}

impl RemoteDesktopProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rdp => "RDP",
            Self::Vnc => "VNC",
        }
    }

    pub fn provider_id(self) -> &'static str {
        match self {
            Self::Rdp => "rdp",
            Self::Vnc => "vnc",
        }
    }
}

#[derive(Clone)]
pub struct RemoteDesktopConnectionOptions {
    pub protocol: RemoteDesktopProtocol,
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktopSize {
    pub width: u16,
    pub height: u16,
}

impl fmt::Debug for RemoteDesktopConnectionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteDesktopConnectionOptions")
            .field("protocol", &self.protocol)
            .field("destination", &self.destination)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("domain", &self.domain)
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_options_debug_redacts_password() {
        let options = RemoteDesktopConnectionOptions {
            protocol: RemoteDesktopProtocol::Rdp,
            destination: "10.2.178.12:3389".to_string(),
            username: Some("administrator".to_string()),
            password: Some("secret".to_string()),
            domain: None,
            read_only: false,
        };

        let debug = format!("{options:?}");

        assert!(debug.contains("administrator"));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));
    }
}
