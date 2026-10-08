//! Presents the native Windows RDP child window inside the GPUI window's
//! DirectComposition tree.
//!
//! The "windows 原生" backend hosts its session in a real child window. Child
//! windows always draw above the DWM visuals that carry GPUI's own overlays, so
//! the session used to swallow everything GPUI painted over it — the tab menus
//! from issue #310. Composition inverts that relationship: the child window's
//! rasterization becomes a visual placed below GPUI's overlay layer, which is
//! why the window is cloaked once it is composed.
//!
//! The child window keeps its HWND, its size, its focus, and its input; only
//! its presentation moves into the visual tree. Without a composition surface
//! the adapter falls back to the previous behaviour of presenting the session as
//! a plain child window.

use gpui::{Bounds, DevicePixels, WindowCompositionSurface, point, size};

use super::windows_native::Win32ClientPhysicalBounds;

/// Drives the composition portal that shows the RDP overlay window's content.
///
/// The portal visual is owned by GPUI; this type only mirrors the child
/// window's placement and visibility into it, and keeps the window content
/// attached for as long as the adapter lives.
pub(super) struct WindowsNativeComposition {
    surface: WindowCompositionSurface,
    /// The overlay window whose rasterization feeds the portal visual.
    window: usize,
    attached: bool,
    visible: bool,
}

impl WindowsNativeComposition {
    pub(super) fn new(surface: WindowCompositionSurface, window: usize) -> Self {
        Self {
            surface,
            window,
            attached: false,
            visible: false,
        }
    }

    /// Hands the overlay window's rasterization to the window composition tree.
    ///
    /// Must succeed before the window is cloaked: cloaking hides the original
    /// window, so an uncomposed cloak would take the session off screen
    /// entirely.
    pub(super) fn attach(&mut self) -> anyhow::Result<()> {
        if self.attached {
            return Ok(());
        }
        let attachment = self.surface.platform_surface()?;
        // The window handle travels as a raw `usize`: navop's Win32 layer already
        // deals in bare handles, and nothing forces it to agree with the
        // `windows` crate version GPUI was built against.
        attachment.set_window_content(Box::new(self.window))?;
        // The visual is placed by the first bounds update; start hidden so a
        // zero-sized portal cannot flash the wrong rectangle first.
        attachment.set_visible(false)?;
        self.attached = true;
        self.visible = false;
        tracing::info!(
            stage = "composition_attached",
            overlay_hwnd = self.window,
            "composed the Windows native RDP overlay into the GPUI visual tree"
        );
        Ok(())
    }

    /// Mirrors the overlay placement into the composition tree.
    ///
    /// `None` means the overlay was clipped away, so its visual must not be
    /// presented either.
    pub(super) fn sync_bounds(
        &mut self,
        bounds: Option<Win32ClientPhysicalBounds>,
    ) -> anyhow::Result<()> {
        if !self.attached {
            return Ok(());
        }
        let attachment = self.surface.platform_surface()?;
        match bounds {
            Some(bounds) => {
                attachment.set_bounds(physical_bounds_to_device_pixels(bounds))?;
                if !self.visible {
                    attachment.set_visible(true)?;
                    self.visible = true;
                }
            }
            None => {
                if self.visible {
                    attachment.set_visible(false)?;
                    self.visible = false;
                }
            }
        }
        Ok(())
    }

    /// Mirrors requested visibility into the composition tree.
    pub(super) fn sync_visible(&mut self, visible: bool) -> anyhow::Result<()> {
        if !self.attached || self.visible == visible {
            return Ok(());
        }
        let attachment = self.surface.platform_surface()?;
        attachment.set_visible(visible)?;
        self.visible = visible;
        Ok(())
    }
}

/// Conversion into the coordinate space GPUI composes in: window content device
/// pixels, which is also the space the overlay window is positioned in.
fn physical_bounds_to_device_pixels(bounds: Win32ClientPhysicalBounds) -> Bounds<DevicePixels> {
    Bounds {
        origin: point(DevicePixels(bounds.x), DevicePixels(bounds.y)),
        size: size(
            DevicePixels(bounds.width.max(0)),
            DevicePixels(bounds.height.max(0)),
        ),
    }
}
