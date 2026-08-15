use crate::ffi::{INPUT_FLAG_ENABLE_WINDOWS_KEY, INPUT_FLAG_GRAB_FOCUS_ON_CONNECT};

use super::collect_flags;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpKeyboardHookMode {
    Local = 0,
    #[default]
    Focused = 1,
    Fullscreen = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpInputPolicy {
    pub keyboard_hook: WindowsRdpKeyboardHookMode,
    pub enable_windows_key: bool,
    pub grab_focus_on_connect: bool,
}

impl Default for WindowsRdpInputPolicy {
    fn default() -> Self {
        Self {
            keyboard_hook: WindowsRdpKeyboardHookMode::Focused,
            enable_windows_key: true,
            grab_focus_on_connect: true,
        }
    }
}

impl WindowsRdpInputPolicy {
    pub(crate) fn flags(&self) -> u32 {
        collect_flags([
            (self.enable_windows_key, INPUT_FLAG_ENABLE_WINDOWS_KEY),
            (self.grab_focus_on_connect, INPUT_FLAG_GRAB_FOCUS_ON_CONNECT),
        ])
    }
}
