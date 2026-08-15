use crate::ffi::{
    PERFORMANCE_FLAG_BITMAP_CACHE, PERFORMANCE_FLAG_CURSOR_SETTINGS,
    PERFORMANCE_FLAG_CURSOR_SHADOW, PERFORMANCE_FLAG_DESKTOP_COMPOSITION,
    PERFORMANCE_FLAG_FONT_SMOOTHING, PERFORMANCE_FLAG_FULL_WINDOW_DRAG,
    PERFORMANCE_FLAG_MENU_ANIMATIONS, PERFORMANCE_FLAG_THEMES, PERFORMANCE_FLAG_WALLPAPER,
};

use super::collect_flags;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpPerformancePreset {
    #[default]
    Auto = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Custom = 4,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpNetworkConnectionType {
    Modem = 0,
    BroadbandLow = 1,
    Satellite = 2,
    BroadbandHigh = 3,
    Wan = 4,
    Lan = 5,
    #[default]
    Auto = 6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpPerformancePolicy {
    pub preset: WindowsRdpPerformancePreset,
    pub wallpaper: bool,
    pub full_window_drag: bool,
    pub menu_animations: bool,
    pub themes: bool,
    pub cursor_shadow: bool,
    pub cursor_settings: bool,
    pub font_smoothing: bool,
    pub desktop_composition: bool,
    pub bitmap_cache: bool,
    pub network_connection_type: WindowsRdpNetworkConnectionType,
}

impl Default for WindowsRdpPerformancePolicy {
    fn default() -> Self {
        Self {
            preset: WindowsRdpPerformancePreset::Auto,
            wallpaper: true,
            full_window_drag: true,
            menu_animations: true,
            themes: true,
            cursor_shadow: true,
            cursor_settings: true,
            font_smoothing: true,
            desktop_composition: true,
            bitmap_cache: true,
            network_connection_type: WindowsRdpNetworkConnectionType::Auto,
        }
    }
}

impl WindowsRdpPerformancePolicy {
    pub(crate) fn flags(&self) -> u32 {
        collect_flags([
            (self.wallpaper, PERFORMANCE_FLAG_WALLPAPER),
            (self.full_window_drag, PERFORMANCE_FLAG_FULL_WINDOW_DRAG),
            (self.menu_animations, PERFORMANCE_FLAG_MENU_ANIMATIONS),
            (self.themes, PERFORMANCE_FLAG_THEMES),
            (self.cursor_shadow, PERFORMANCE_FLAG_CURSOR_SHADOW),
            (self.cursor_settings, PERFORMANCE_FLAG_CURSOR_SETTINGS),
            (self.font_smoothing, PERFORMANCE_FLAG_FONT_SMOOTHING),
            (
                self.desktop_composition,
                PERFORMANCE_FLAG_DESKTOP_COMPOSITION,
            ),
            (self.bitmap_cache, PERFORMANCE_FLAG_BITMAP_CACHE),
        ])
    }
}
