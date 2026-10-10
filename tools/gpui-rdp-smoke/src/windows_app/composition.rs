//! Presents the native Windows RDP child window inside the GPUI window's
//! DirectComposition tree.
//!
//! The smoke client exists to isolate exactly this question: a real child window
//! always draws above the DWM visuals that carry GPUI's own drawing, so the
//! session swallows everything GPUI paints over it. Composition is supposed to
//! invert that relationship by turning the child window's rasterization into a
//! visual below GPUI's overlay layer — which is why the window is cloaked once
//! it is composed.
//!
//! Everything here mirrors what `remote_desktop_view` does in Navop, so the
//! experiment and the product take the same path through GPUI.

use gpui::{Bounds, DevicePixels, WindowCompositionSurface, point, size};

/// Drives the composition portal that shows the RDP overlay window's content.
///
/// The portal visual is owned by GPUI; this type only mirrors the child window's
/// placement and visibility into it, and keeps the window content attached for
/// as long as it lives.
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

    /// Hands the overlay window's rasterization to the composition tree.
    ///
    /// Must succeed before the window is cloaked: cloaking hides the original
    /// window, so an uncomposed cloak would take the session off screen
    /// entirely.
    pub(super) fn attach(&mut self) -> Result<(), String> {
        if self.attached {
            return Ok(());
        }
        let attachment = self
            .surface
            .platform_surface()
            .map_err(|error| format!("this composition surface is not platform backed: {error:#}"))?;
        attachment
            .set_window_content(Box::new(self.window))
            .map_err(|error| {
                format!("attaching the overlay window as the portal's composition content: {error:#}")
            })?;
        match attachment.set_visible(false) {
            Ok(()) => self.visible = false,
            Err(error) => {
                // A platform that refuses the hide still composes correctly: the
                // surface starts with a zero-area clip and the bounds sync runs
                // in the same turn. Retiring the attachment over it would throw
                // away a working composition surface.
                println!(
                    "composition: portal refused to start hidden error={error:#}; composing anyway"
                );
                self.visible = true;
            }
        }
        self.attached = true;
        println!(
            "composition: stage=attached overlay_hwnd=0x{:016X}",
            self.window
        );
        Ok(())
    }

    /// Mirrors the overlay placement into the composition tree.
    ///
    /// `None` means the overlay was clipped away, so its visual must not be
    /// presented either.
    pub(super) fn sync_bounds(&mut self, bounds: Option<(i32, i32, i32, i32)>) -> Result<(), String> {
        if !self.attached {
            return Ok(());
        }
        let attachment = self
            .surface
            .platform_surface()
            .map_err(|error| format!("this composition surface is not platform backed: {error:#}"))?;
        match bounds {
            Some(bounds) => {
                attachment
                    .set_bounds(physical_bounds_to_device_pixels(bounds))
                    .map_err(|error| format!("mirroring the overlay bounds into the portal: {error:#}"))?;
                if !self.visible {
                    attachment
                        .set_visible(true)
                        .map_err(|error| format!("showing the composed overlay visual: {error:#}"))?;
                    self.visible = true;
                }
            }
            None => {
                if self.visible {
                    attachment
                        .set_visible(false)
                        .map_err(|error| format!("hiding the composed overlay visual: {error:#}"))?;
                    self.visible = false;
                }
            }
        }
        Ok(())
    }

    /// Mirrors requested visibility into the composition tree.
    pub(super) fn sync_visible(&mut self, visible: bool) -> Result<(), String> {
        if !self.attached || self.visible == visible {
            return Ok(());
        }
        let attachment = self
            .surface
            .platform_surface()
            .map_err(|error| format!("this composition surface is not platform backed: {error:#}"))?;
        attachment
            .set_visible(visible)
            .map_err(|error| format!("mirroring the overlay visibility into the portal: {error:#}"))?;
        self.visible = visible;
        Ok(())
    }
}

/// Conversion into the coordinate space GPUI composes in: window content device
/// pixels, which is also the space the overlay window is positioned in.
fn physical_bounds_to_device_pixels(bounds: (i32, i32, i32, i32)) -> Bounds<DevicePixels> {
    Bounds {
        origin: point(DevicePixels(bounds.0), DevicePixels(bounds.1)),
        size: size(
            DevicePixels(bounds.2.max(0)),
            DevicePixels(bounds.3.max(0)),
        ),
    }
}
