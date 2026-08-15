use crate::error::WindowsRdpHostError;
use crate::ffi::{MAX_KEEP_ALIVE_SECONDS, MAX_RECONNECT_ATTEMPTS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpReconnectPolicy {
    pub keep_alive_seconds: u32,
    pub timeout_seconds: u32,
    pub auto_reconnect: bool,
    pub max_reconnect_attempts: u32,
}

impl Default for WindowsRdpReconnectPolicy {
    fn default() -> Self {
        Self {
            keep_alive_seconds: 60,
            timeout_seconds: 600,
            auto_reconnect: true,
            max_reconnect_attempts: MAX_RECONNECT_ATTEMPTS,
        }
    }
}

impl WindowsRdpReconnectPolicy {
    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        // keep_alive_seconds feeds KeepAliveInterval (milliseconds), so the
        // value must survive a LONG conversion after the ×1000 scale.
        let keep_alive_valid = self.keep_alive_seconds <= MAX_KEEP_ALIVE_SECONDS;
        let timeout_valid = i32::try_from(self.timeout_seconds).is_ok();
        let reconnect_valid = self.max_reconnect_attempts <= MAX_RECONNECT_ATTEMPTS;
        if keep_alive_valid && timeout_valid && reconnect_valid {
            Ok(())
        } else {
            Err(WindowsRdpHostError::InvalidArgument)
        }
    }
}
