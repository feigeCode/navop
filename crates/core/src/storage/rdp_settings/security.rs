use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEFAULT_RDP_KEEP_ALIVE_SECONDS: u32 = 60;
pub const DEFAULT_RDP_CONNECTION_TIMEOUT_SECONDS: u32 = 600;
// Matches NAVOP_RDP_MAX_RECONNECT_ATTEMPTS in the Windows RDP host ABI; the
// native MSTSC MaxReconnectAttempts property and the retry dialog share the
// same 200-attempt ceiling.
pub const DEFAULT_RDP_MAX_RECONNECT_ATTEMPTS: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpSecuritySettings {
    pub enable_credssp: bool,
    pub authentication_level: u32,
    pub public_mode: bool,
    pub encryption_enabled: bool,
}

impl Default for RdpSecuritySettings {
    fn default() -> Self {
        Self {
            enable_credssp: true,
            authentication_level: 0,
            public_mode: false,
            encryption_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpGatewayMode {
    #[default]
    Disabled,
    Explicit,
    AutoDetect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpGatewayCredentialSource {
    #[default]
    Password,
    SmartCard,
    Any,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpGatewaySettings {
    pub mode: RdpGatewayMode,
    pub bypass_local: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub credential_source: RdpGatewayCredentialSource,
}

impl Default for RdpGatewaySettings {
    fn default() -> Self {
        Self {
            mode: RdpGatewayMode::Disabled,
            bypass_local: true,
            hostname: None,
            username: None,
            password: None,
            domain: None,
            credential_source: RdpGatewayCredentialSource::Password,
        }
    }
}

impl fmt::Debug for RdpGatewaySettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RdpGatewaySettings")
            .field("mode", &self.mode)
            .field("bypass_local", &self.bypass_local)
            .field("hostname_present", &self.hostname.is_some())
            .field("username_present", &self.username.is_some())
            .field("password_present", &self.password.is_some())
            .field("domain_present", &self.domain.is_some())
            .field("credential_source", &self.credential_source)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpConnectionSettings {
    pub keep_alive_seconds: u32,
    pub timeout_seconds: u32,
    pub auto_reconnect: bool,
    pub max_reconnect_attempts: u32,
}

impl Default for RdpConnectionSettings {
    fn default() -> Self {
        Self {
            keep_alive_seconds: DEFAULT_RDP_KEEP_ALIVE_SECONDS,
            timeout_seconds: DEFAULT_RDP_CONNECTION_TIMEOUT_SECONDS,
            auto_reconnect: true,
            max_reconnect_attempts: DEFAULT_RDP_MAX_RECONNECT_ATTEMPTS,
        }
    }
}
