use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpPerformancePreset {
    #[default]
    Auto,
    Low,
    Medium,
    High,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpNetworkConnectionType {
    Modem,
    BroadbandLow,
    Satellite,
    BroadbandHigh,
    Wan,
    Lan,
    #[default]
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpPerformanceSettings {
    pub preset: RdpPerformancePreset,
    pub wallpaper: bool,
    pub full_window_drag: bool,
    pub menu_animations: bool,
    pub themes: bool,
    pub cursor_shadow: bool,
    pub cursor_settings: bool,
    pub font_smoothing: bool,
    pub desktop_composition: bool,
    pub bitmap_cache: bool,
    pub network_connection_type: RdpNetworkConnectionType,
}

impl Default for RdpPerformanceSettings {
    fn default() -> Self {
        Self {
            preset: RdpPerformancePreset::Auto,
            wallpaper: true,
            full_window_drag: true,
            menu_animations: true,
            themes: true,
            cursor_shadow: true,
            cursor_settings: true,
            font_smoothing: true,
            desktop_composition: true,
            bitmap_cache: true,
            network_connection_type: RdpNetworkConnectionType::Auto,
        }
    }
}
