use crate::error::WindowsRdpHostError;

const MILLISECONDS_PER_SECOND: u32 = 1_000;

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
            max_reconnect_attempts: 20,
        }
    }
}

impl WindowsRdpReconnectPolicy {
    pub(crate) fn validate(&self) -> Result<(), WindowsRdpHostError> {
        let keep_alive_valid = self
            .keep_alive_seconds
            .checked_mul(MILLISECONDS_PER_SECOND)
            .is_some();
        let timeout_valid = i32::try_from(self.timeout_seconds).is_ok();
        let reconnect_valid = i32::try_from(self.max_reconnect_attempts).is_ok();
        if keep_alive_valid && timeout_valid && reconnect_valid {
            Ok(())
        } else {
            Err(WindowsRdpHostError::InvalidArgument)
        }
    }
}
