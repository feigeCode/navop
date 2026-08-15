mod display;
mod performance;
mod resources;
mod security;

pub use display::*;
pub use performance::*;
pub use resources::*;
pub use security::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpSettings {
    pub admin_session: bool,
    pub display: RdpDisplaySettings,
    pub resources: RdpResourceSettings,
    pub performance: RdpPerformanceSettings,
    pub audio: RdpAudioSettings,
    pub input: RdpInputSettings,
    pub security: RdpSecuritySettings,
    pub gateway: RdpGatewaySettings,
    pub connection: RdpConnectionSettings,
}

impl RdpSettings {
    pub fn from_legacy_audio_playback(audio_playback: bool) -> Self {
        let mut settings = Self::default();
        if !audio_playback {
            settings.audio.mode = RdpAudioMode::Disabled;
        }
        settings
    }
}

impl Default for RdpSettings {
    fn default() -> Self {
        Self {
            admin_session: false,
            display: RdpDisplaySettings::default(),
            resources: RdpResourceSettings::default(),
            performance: RdpPerformanceSettings::default(),
            audio: RdpAudioSettings::default(),
            input: RdpInputSettings::default(),
            security: RdpSecuritySettings::default(),
            gateway: RdpGatewaySettings::default(),
            connection: RdpConnectionSettings::default(),
        }
    }
}
