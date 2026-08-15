use std::fmt;

use crate::error::WindowsRdpHostError;
use crate::ffi::{
    CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED, NavopRdpBorrowedUtf16, NavopRdpConnectionOptions,
};
use crate::policy::{WindowsRdpAudioMode, WindowsRdpConnectionPolicy};

/// Maximum accepted RDP server name length, measured in UTF-16 code units.
pub const WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS: usize = 255;

/// Options used to allocate the opaque native host lifecycle handle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsRdpHostOptions {
    generation: u64,
}

impl WindowsRdpHostOptions {
    pub const fn new(generation: u64) -> Self {
        Self { generation }
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Color depth requested from the Windows RDP ActiveX control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpColorDepth {
    Bpp8,
    Bpp15,
    Bpp16,
    Bpp24,
    Bpp32,
}

impl WindowsRdpColorDepth {
    pub const fn bits_per_pixel(self) -> i32 {
        match self {
            Self::Bpp8 => 8,
            Self::Bpp15 => 15,
            Self::Bpp16 => 16,
            Self::Bpp24 => 24,
            Self::Bpp32 => 32,
        }
    }
}

/// Connection endpoint and complete native MSTSC policy.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowsRdpConnectionOptions {
    host: String,
    port: u16,
    desktop_width: u32,
    desktop_height: u32,
    color_depth: WindowsRdpColorDepth,
    policy: WindowsRdpConnectionPolicy,
}

impl fmt::Debug for WindowsRdpConnectionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRdpConnectionOptions")
            .field("host", &redacted_host(&self.host))
            .field("port", &self.port)
            .field("desktop_width", &self.desktop_width)
            .field("desktop_height", &self.desktop_height)
            .field("color_depth", &self.color_depth)
            .field("audio_playback", &self.audio_playback())
            .field("policy", &self.policy)
            .finish()
    }
}

fn redacted_host(host: &str) -> String {
    format!(
        "<redacted, {} UTF-16 code units>",
        host.encode_utf16().count()
    )
}

impl WindowsRdpConnectionOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        desktop_width: u32,
        desktop_height: u32,
        color_depth: WindowsRdpColorDepth,
    ) -> Result<Self, WindowsRdpHostError> {
        let host = host.into();
        let host_utf16_len = host.encode_utf16().count();
        if host_utf16_len == 0
            || host_utf16_len > WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS
            || host.contains('\0')
            || port == 0
            || desktop_width == 0
            || desktop_height == 0
            || desktop_width > i32::MAX as u32
            || desktop_height > i32::MAX as u32
        {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        Ok(Self {
            host,
            port,
            desktop_width,
            desktop_height,
            color_depth,
            policy: WindowsRdpConnectionPolicy::default(),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    pub const fn desktop_width(&self) -> u32 {
        self.desktop_width
    }

    pub const fn desktop_height(&self) -> u32 {
        self.desktop_height
    }

    pub const fn color_depth(&self) -> WindowsRdpColorDepth {
        self.color_depth
    }

    pub fn audio_playback(&self) -> bool {
        self.policy.audio.mode == WindowsRdpAudioMode::Local
    }

    #[must_use]
    pub fn with_audio_playback(mut self, enabled: bool) -> Self {
        self.policy.audio.mode = if enabled {
            WindowsRdpAudioMode::Local
        } else {
            WindowsRdpAudioMode::Disabled
        };
        self
    }

    pub const fn policy(&self) -> &WindowsRdpConnectionPolicy {
        &self.policy
    }

    #[must_use]
    pub fn with_policy(mut self, policy: WindowsRdpConnectionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub(crate) fn as_native(&self) -> Result<NativeConnectionOptions, WindowsRdpHostError> {
        self.policy.validate()?;
        let host_utf16: Vec<u16> = self.host.encode_utf16().collect();
        let host = borrowed_utf16(&host_utf16)?;
        let gateway_hostname_utf16: Vec<u16> = self
            .policy
            .gateway
            .hostname
            .as_deref()
            .map(|hostname| hostname.encode_utf16().collect())
            .unwrap_or_default();
        let gateway_hostname = borrowed_utf16(&gateway_hostname_utf16)?;
        let mut native = NavopRdpConnectionOptions::current(
            host,
            u32::from(self.port),
            self.desktop_width as i32,
            self.desktop_height as i32,
            self.color_depth.bits_per_pixel(),
        );
        if self.policy.audio.mode == WindowsRdpAudioMode::Disabled {
            native.flags |= CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED;
        }
        apply_policy(&mut native, &self.policy, gateway_hostname);
        Ok(NativeConnectionOptions {
            native,
            _host_utf16: host_utf16,
            _gateway_hostname_utf16: gateway_hostname_utf16,
        })
    }
}

fn borrowed_utf16(text: &[u16]) -> Result<NavopRdpBorrowedUtf16, WindowsRdpHostError> {
    let len = u32::try_from(text.len()).map_err(|_| WindowsRdpHostError::InvalidArgument)?;
    Ok(NavopRdpBorrowedUtf16 {
        data: if text.is_empty() {
            std::ptr::null()
        } else {
            text.as_ptr()
        },
        len,
    })
}

fn apply_policy(
    native: &mut NavopRdpConnectionOptions,
    policy: &WindowsRdpConnectionPolicy,
    gateway_hostname: NavopRdpBorrowedUtf16,
) {
    native.display_mode = policy.display.mode as u32;
    native.display_flags = policy.display.flags();
    native.desktop_scale_factor = policy.display.desktop_scale_factor;
    native.device_scale_factor = policy.display.device_scale_factor;
    native.resource_flags = policy.resources.flags();
    native.audio_mode = policy.audio.mode as u32;
    native.audio_quality = policy.audio.quality as u32;
    native.audio_flags = policy.audio.flags();
    native.keyboard_hook_mode = policy.input.keyboard_hook as u32;
    native.input_flags = policy.input.flags();
    native.performance_preset = policy.performance.preset as u32;
    native.performance_flags = policy.performance.flags();
    native.network_connection_type = policy.performance.network_connection_type as u32;
    native.security_flags = policy.security.flags();
    native.authentication_level = policy.security.authentication_level;
    native.gateway_mode = policy.gateway.mode as u32;
    native.gateway_flags = policy.gateway.flags();
    native.gateway_credential_source = policy.gateway.credential_source as u32;
    native.gateway_hostname = gateway_hostname;
    native.keep_alive_seconds = policy.reconnect.keep_alive_seconds;
    native.timeout_seconds = policy.reconnect.timeout_seconds;
    native.connection_flags = policy.connection_flags();
    native.max_reconnect_attempts = policy.reconnect.max_reconnect_attempts;
}

pub(crate) struct NativeConnectionOptions {
    pub(crate) native: NavopRdpConnectionOptions,
    _host_utf16: Vec<u16>,
    _gateway_hostname_utf16: Vec<u16>,
}

/// A caller-owned native parent window handle for the Windows-only ActiveX
/// presentation path.
///
/// The integer is never dereferenced by Rust. The caller must keep the
/// underlying window valid on the host owner/UI thread until the host has been
/// successfully closed or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowsRdpParentWindow(usize);

impl WindowsRdpParentWindow {
    /// Wraps a valid, non-null native parent window handle.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid parent window handle owned by the caller, and the
    /// caller must keep that window alive until the created host is destroyed.
    pub const unsafe fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> usize {
        self.0
    }
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
