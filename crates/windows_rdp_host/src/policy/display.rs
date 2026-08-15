use crate::error::WindowsRdpHostError;
use crate::ffi::{
    DISPLAY_FLAG_SMART_SIZING, DISPLAY_FLAG_SPAN_MONITORS, DISPLAY_FLAG_USE_MULTIMON,
};

use super::collect_flags;

const MIN_SCALE_FACTOR: u32 = 100;
const MAX_SCALE_FACTOR: u32 = 500;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpDisplayMode {
    #[default]
    Dynamic = 0,
    Fixed = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpDisplayPolicy {
    pub mode: WindowsRdpDisplayMode,
    pub smart_sizing: bool,
    pub use_multimon: bool,
    pub span_monitors: bool,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

impl Default for WindowsRdpDisplayPolicy {
    fn default() -> Self {
        Self {
            mode: WindowsRdpDisplayMode::Dynamic,
            smart_sizing: false,
            use_multimon: false,
            span_monitors: false,
            desktop_scale_factor: MIN_SCALE_FACTOR,
            device_scale_factor: MIN_SCALE_FACTOR,
        }
    }
}

impl WindowsRdpDisplayPolicy {
    pub(crate) fn flags(&self) -> u32 {
        collect_flags([
            (self.smart_sizing, DISPLAY_FLAG_SMART_SIZING),
            (self.use_multimon, DISPLAY_FLAG_USE_MULTIMON),
            (self.span_monitors, DISPLAY_FLAG_SPAN_MONITORS),
        ])
    }

    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        let desktop_valid =
            (MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&self.desktop_scale_factor);
        let device_valid = matches!(self.device_scale_factor, 100 | 140 | 180);
        if desktop_valid && device_valid {
            Ok(())
        } else {
            Err(WindowsRdpHostError::InvalidArgument)
        }
    }
}
