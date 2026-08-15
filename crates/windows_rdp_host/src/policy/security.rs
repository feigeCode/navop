use crate::error::WindowsRdpHostError;
use crate::ffi::{
    SECURITY_FLAG_ENABLE_CREDSSP, SECURITY_FLAG_ENCRYPTION_ENABLED, SECURITY_FLAG_PUBLIC_MODE,
};

use super::collect_flags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpSecurityPolicy {
    pub enable_credssp: bool,
    pub authentication_level: u32,
    pub public_mode: bool,
    pub encryption_enabled: bool,
}

impl Default for WindowsRdpSecurityPolicy {
    fn default() -> Self {
        Self {
            enable_credssp: true,
            authentication_level: 0,
            public_mode: false,
            encryption_enabled: true,
        }
    }
}

impl WindowsRdpSecurityPolicy {
    pub(crate) fn flags(&self) -> u32 {
        collect_flags([
            (self.enable_credssp, SECURITY_FLAG_ENABLE_CREDSSP),
            (self.public_mode, SECURITY_FLAG_PUBLIC_MODE),
            (self.encryption_enabled, SECURITY_FLAG_ENCRYPTION_ENABLED),
        ])
    }

    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        if self.authentication_level <= 2 {
            Ok(())
        } else {
            Err(WindowsRdpHostError::InvalidArgument)
        }
    }
}
