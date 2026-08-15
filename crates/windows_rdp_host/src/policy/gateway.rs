use std::fmt;

use crate::error::WindowsRdpHostError;
use crate::ffi::GATEWAY_FLAG_BYPASS_LOCAL;

pub const WINDOWS_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS: usize = 255;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpGatewayMode {
    #[default]
    Disabled = 0,
    Explicit = 1,
    AutoDetect = 2,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpGatewayCredentialSource {
    #[default]
    Password = 0,
    SmartCard = 1,
    Any = 4,
}

#[derive(Clone, PartialEq, Eq)]
pub struct WindowsRdpGatewayPolicy {
    pub mode: WindowsRdpGatewayMode,
    pub bypass_local: bool,
    pub credential_source: WindowsRdpGatewayCredentialSource,
    pub hostname: Option<String>,
}

impl Default for WindowsRdpGatewayPolicy {
    fn default() -> Self {
        Self {
            mode: WindowsRdpGatewayMode::Disabled,
            bypass_local: true,
            credential_source: WindowsRdpGatewayCredentialSource::Password,
            hostname: None,
        }
    }
}

impl fmt::Debug for WindowsRdpGatewayPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRdpGatewayPolicy")
            .field("mode", &self.mode)
            .field("bypass_local", &self.bypass_local)
            .field("credential_source", &self.credential_source)
            .field("hostname_present", &self.hostname.is_some())
            .finish()
    }
}

impl WindowsRdpGatewayPolicy {
    pub(crate) const fn flags(&self) -> u32 {
        if self.bypass_local {
            GATEWAY_FLAG_BYPASS_LOCAL
        } else {
            0
        }
    }

    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        let hostname = self.hostname.as_deref();
        if self.mode == WindowsRdpGatewayMode::Explicit && hostname.is_none_or(str::is_empty) {
            return Err(WindowsRdpHostError::InvalidArgument);
        }
        if hostname.is_some_and(invalid_hostname) {
            return Err(WindowsRdpHostError::InvalidArgument);
        }
        Ok(())
    }
}

fn invalid_hostname(hostname: &str) -> bool {
    hostname.contains('\0')
        || hostname.encode_utf16().count() > WINDOWS_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS
}
