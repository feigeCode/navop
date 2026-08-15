use crate::ffi::AUDIO_FLAG_CAPTURE;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpAudioMode {
    #[default]
    Local = 0,
    Remote = 1,
    Disabled = 2,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowsRdpAudioQuality {
    #[default]
    Dynamic = 0,
    Medium = 1,
    High = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowsRdpAudioPolicy {
    pub mode: WindowsRdpAudioMode,
    pub quality: WindowsRdpAudioQuality,
    pub capture: bool,
}

impl WindowsRdpAudioPolicy {
    pub(crate) const fn flags(&self) -> u32 {
        if self.capture { AUDIO_FLAG_CAPTURE } else { 0 }
    }
}
