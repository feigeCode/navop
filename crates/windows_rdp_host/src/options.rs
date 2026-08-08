use crate::error::WindowsRdpHostError;
use crate::ffi::{NavopRdpBorrowedUtf16, NavopRdpConnectionOptions};

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

/// Minimal connection configuration kept separate from credentials and future
/// security/redirection policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpConnectionOptions {
    host: String,
    port: u16,
    desktop_width: u32,
    desktop_height: u32,
    color_depth: WindowsRdpColorDepth,
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

    pub(crate) fn as_native(&self) -> Result<NativeConnectionOptions, WindowsRdpHostError> {
        let host_utf16: Vec<u16> = self.host.encode_utf16().collect();
        let host_len =
            u32::try_from(host_utf16.len()).map_err(|_| WindowsRdpHostError::InvalidArgument)?;
        let native = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_utf16.as_ptr(),
                len: host_len,
            },
            u32::from(self.port),
            self.desktop_width as i32,
            self.desktop_height as i32,
            self.color_depth.bits_per_pixel(),
        );
        Ok(NativeConnectionOptions {
            native,
            _host_utf16: host_utf16,
        })
    }
}

pub(crate) struct NativeConnectionOptions {
    pub(crate) native: NavopRdpConnectionOptions,
    _host_utf16: Vec<u16>,
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
mod tests {
    use super::*;

    fn valid_options(
        host: impl Into<String>,
    ) -> Result<WindowsRdpConnectionOptions, WindowsRdpHostError> {
        WindowsRdpConnectionOptions::new(host, 3389, 1920, 1080, WindowsRdpColorDepth::Bpp32)
    }

    #[test]
    fn connection_options_reject_invalid_host_and_port() {
        assert_eq!(valid_options(""), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!(
            valid_options("rdp\0host"),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            WindowsRdpConnectionOptions::new(
                "rdp.example",
                0,
                1920,
                1080,
                WindowsRdpColorDepth::Bpp32,
            ),
            Err(WindowsRdpHostError::InvalidArgument)
        );
    }

    #[test]
    fn connection_options_measure_host_length_in_utf16_code_units() {
        let exact = "a".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS);
        assert!(valid_options(exact).is_ok());

        let over = "a".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS + 1);
        assert_eq!(
            valid_options(over),
            Err(WindowsRdpHostError::InvalidArgument)
        );

        let supplementary = "😀".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS / 2);
        assert!(valid_options(supplementary).is_ok());
        let supplementary_over = "😀".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS / 2 + 1);
        assert_eq!(
            valid_options(supplementary_over),
            Err(WindowsRdpHostError::InvalidArgument)
        );
    }

    #[test]
    fn connection_options_reject_invalid_dimensions() {
        for (width, height) in [
            (0, 1080),
            (1920, 0),
            (i32::MAX as u32 + 1, 1080),
            (1920, i32::MAX as u32 + 1),
        ] {
            assert_eq!(
                WindowsRdpConnectionOptions::new(
                    "rdp.example",
                    3389,
                    width,
                    height,
                    WindowsRdpColorDepth::Bpp32,
                ),
                Err(WindowsRdpHostError::InvalidArgument)
            );
        }
    }

    #[test]
    fn connection_options_preserve_valid_values() {
        let options = WindowsRdpConnectionOptions::new(
            "rdp.example",
            3390,
            1600,
            900,
            WindowsRdpColorDepth::Bpp24,
        )
        .expect("valid connection options");

        assert_eq!(options.host(), "rdp.example");
        assert_eq!(options.port(), 3390);
        assert_eq!(options.desktop_width(), 1600);
        assert_eq!(options.desktop_height(), 900);
        assert_eq!(options.color_depth(), WindowsRdpColorDepth::Bpp24);
    }
}
