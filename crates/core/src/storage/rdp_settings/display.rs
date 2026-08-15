use serde::{Deserialize, Serialize};

pub const DEFAULT_RDP_DESKTOP_WIDTH: u32 = 1920;
pub const DEFAULT_RDP_DESKTOP_HEIGHT: u32 = 1080;
pub const DEFAULT_RDP_SCALE_FACTOR: u32 = 100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpDisplayMode {
    #[default]
    Dynamic,
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpDisplaySettings {
    pub mode: RdpDisplayMode,
    pub width: u32,
    pub height: u32,
    pub smart_sizing: bool,
    pub use_multimon: bool,
    pub span_monitors: bool,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

impl Default for RdpDisplaySettings {
    fn default() -> Self {
        Self {
            mode: RdpDisplayMode::Dynamic,
            width: DEFAULT_RDP_DESKTOP_WIDTH,
            height: DEFAULT_RDP_DESKTOP_HEIGHT,
            smart_sizing: false,
            use_multimon: false,
            span_monitors: false,
            desktop_scale_factor: DEFAULT_RDP_SCALE_FACTOR,
            device_scale_factor: DEFAULT_RDP_SCALE_FACTOR,
        }
    }
}
