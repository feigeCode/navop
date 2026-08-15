use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpSharedFolder {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpResourceSettings {
    pub clipboard: bool,
    pub drives: bool,
    pub dynamic_drives: bool,
    pub dynamic_devices: bool,
    pub printers: bool,
    pub serial_ports: bool,
    pub smart_cards: bool,
    pub cameras: bool,
    pub microphones: bool,
    pub pos_devices: bool,
    pub shared_folders: Vec<RdpSharedFolder>,
}

impl Default for RdpResourceSettings {
    fn default() -> Self {
        Self {
            clipboard: true,
            drives: false,
            dynamic_drives: false,
            dynamic_devices: false,
            printers: false,
            serial_ports: false,
            smart_cards: false,
            cameras: false,
            microphones: false,
            pos_devices: false,
            shared_folders: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpAudioMode {
    #[default]
    Local,
    Remote,
    Disabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpAudioQuality {
    #[default]
    Dynamic,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpAudioSettings {
    pub mode: RdpAudioMode,
    pub quality: RdpAudioQuality,
    pub capture: bool,
}

impl Default for RdpAudioSettings {
    fn default() -> Self {
        Self {
            mode: RdpAudioMode::Local,
            quality: RdpAudioQuality::Dynamic,
            capture: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpKeyboardHookMode {
    Local,
    #[default]
    Focused,
    Fullscreen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpInputSettings {
    pub keyboard_hook: RdpKeyboardHookMode,
    pub enable_windows_key: bool,
    pub grab_focus_on_connect: bool,
}

impl Default for RdpInputSettings {
    fn default() -> Self {
        Self {
            keyboard_hook: RdpKeyboardHookMode::Focused,
            enable_windows_key: true,
            grab_focus_on_connect: true,
        }
    }
}
