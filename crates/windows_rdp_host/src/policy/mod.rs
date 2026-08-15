mod audio;
mod display;
mod gateway;
mod input;
mod performance;
mod reconnect;
mod resources;
mod security;

pub use audio::*;
pub use display::*;
pub use gateway::*;
pub use input::*;
pub use performance::*;
pub use reconnect::*;
pub use resources::*;
pub use security::*;

use crate::error::WindowsRdpHostError;

/// Complete connection policy transported to the native MSTSC ActiveX host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowsRdpConnectionPolicy {
    pub admin_session: bool,
    pub display: WindowsRdpDisplayPolicy,
    pub resources: WindowsRdpResourcePolicy,
    pub audio: WindowsRdpAudioPolicy,
    pub input: WindowsRdpInputPolicy,
    pub performance: WindowsRdpPerformancePolicy,
    pub security: WindowsRdpSecurityPolicy,
    pub gateway: WindowsRdpGatewayPolicy,
    pub reconnect: WindowsRdpReconnectPolicy,
}

impl WindowsRdpConnectionPolicy {
    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        self.display.validate()?;
        self.security.validate()?;
        self.gateway.validate()?;
        self.reconnect.validate()
    }

    pub(crate) fn connection_flags(&self) -> u32 {
        use crate::ffi::{CONNECTION_FLAG_ADMIN_SESSION, CONNECTION_FLAG_AUTO_RECONNECT};

        collect_flags([
            (self.admin_session, CONNECTION_FLAG_ADMIN_SESSION),
            (
                self.reconnect.auto_reconnect,
                CONNECTION_FLAG_AUTO_RECONNECT,
            ),
        ])
    }
}

pub(super) fn collect_flags<const N: usize>(values: [(bool, u32); N]) -> u32 {
    values
        .into_iter()
        .filter_map(|(enabled, flag)| enabled.then_some(flag))
        .fold(0, |flags, flag| flags | flag)
}
