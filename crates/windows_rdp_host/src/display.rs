use crate::error::WindowsRdpHostError;

/// Post-login RDP session framebuffer dimensions and scale factors.
///
/// These settings are passed to
/// `IMsRdpClient9::UpdateSessionDisplaySettings` and are intentionally
/// separate from the native child-window bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpSessionDisplaySettings {
    desktop_width: u32,
    desktop_height: u32,
    physical_width: u32,
    physical_height: u32,
    orientation: u32,
    desktop_scale_factor: u32,
    device_scale_factor: u32,
}

impl WindowsRdpSessionDisplaySettings {
    /// Creates settings for one physical-pixel viewport.
    ///
    /// The RDP desktop and physical dimensions match the viewport,
    /// orientation is landscape (`0`), and the device scale is `100`.
    pub fn viewport(
        width: u32,
        height: u32,
        desktop_scale_factor: u32,
    ) -> Result<Self, WindowsRdpHostError> {
        if width == 0 || height == 0 || desktop_scale_factor == 0 {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        Ok(Self {
            desktop_width: width,
            desktop_height: height,
            physical_width: width,
            physical_height: height,
            orientation: 0,
            desktop_scale_factor,
            device_scale_factor: 100,
        })
    }

    pub const fn desktop_width(self) -> u32 {
        self.desktop_width
    }

    pub const fn desktop_height(self) -> u32 {
        self.desktop_height
    }

    pub const fn physical_width(self) -> u32 {
        self.physical_width
    }

    pub const fn physical_height(self) -> u32 {
        self.physical_height
    }

    pub const fn orientation(self) -> u32 {
        self.orientation
    }

    pub const fn desktop_scale_factor(self) -> u32 {
        self.desktop_scale_factor
    }

    pub const fn device_scale_factor(self) -> u32 {
        self.device_scale_factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_uses_matching_physical_dimensions_and_default_device_scale() {
        let settings =
            WindowsRdpSessionDisplaySettings::viewport(1920, 1080, 150).expect("valid viewport");

        assert_eq!(settings.desktop_width(), 1920);
        assert_eq!(settings.desktop_height(), 1080);
        assert_eq!(settings.physical_width(), 1920);
        assert_eq!(settings.physical_height(), 1080);
        assert_eq!(settings.orientation(), 0);
        assert_eq!(settings.desktop_scale_factor(), 150);
        assert_eq!(settings.device_scale_factor(), 100);
    }

    #[test]
    fn zero_viewport_dimensions_and_scale_are_rejected() {
        let valid = [1920, 1080, 100];
        for index in 0..valid.len() {
            let mut values = valid;
            values[index] = 0;
            assert_eq!(
                WindowsRdpSessionDisplaySettings::viewport(values[0], values[1], values[2]),
                Err(WindowsRdpHostError::InvalidArgument)
            );
        }
    }
}
