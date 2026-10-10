//! Presents the native Windows RDP child window inside the GPUI window's
//! DirectComposition tree.
//!
//! The "windows 原生" backend hosts its session in a real child window. Child
//! windows always draw above the DWM visuals that carry GPUI's own overlays, so
//! the session used to swallow everything GPUI painted over it — the tab menus
//! from issue #310. Composition can invert that relationship: the child window's
//! rasterization becomes a visual placed below GPUI's overlay layer, and the
//! window is cloaked so that this visual, not the window, is what the screen
//! shows.
//!
//! That cloak is not free. A cloaked window leaves the screen Z-order — which is
//! precisely why the overlay layer becomes visible — and the same mechanism
//! takes it out of the system's hit testing, so a cloaked session stops
//! receiving mouse and keyboard input. Composition therefore does **not** cloak
//! on its own: `WindowsNativeAdapter::set_overlay_cloak` applies the cloak only
//! while GPUI really has overlay content on screen and drops it again
//! afterwards, leaving the session with a plain child window's input the rest of
//! the time.
//!
//! The child window keeps its HWND, its size, its focus, and its input; only
//! its presentation moves into the visual tree. Without a composition surface
//! the adapter falls back to the previous behaviour of presenting the session as
//! a plain child window — where the overlay layer cannot win either way, and
//! nothing needs toggling.

use anyhow::Context as _;
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
    /// Whether the overlay still owes a cloak that would prove its mirrored
    /// rasterization is non-empty.
    ///
    /// Composing no longer cloaks on its own: a cloak costs the session its
    /// input, so the session keeps its input until GPUI really has overlay
    /// content to show (see `WindowsNativeAdapter::set_overlay_cloak`). This
    /// stays `false` for a session built here, and the primitive it guards
    /// remains because a cloak applied over a window that never painted freezes
    /// an empty composition surface.
    cloak_pending: bool,
}

impl WindowsNativeComposition {
    pub(super) fn new(surface: WindowCompositionSurface, window: usize) -> Self {
        Self {
            surface,
            window,
            attached: false,
            visible: false,
            cloak_pending: false,
        }
    }

    /// Whether the overlay still owes a cloak before it stops drawing itself on
    /// top of the composed visual.
    pub(super) fn cloak_pending(&self) -> bool {
        self.cloak_pending
    }

    /// Records that the deferred cloak has been applied.
    pub(super) fn mark_cloaked(&mut self) {
        self.cloak_pending = false;
    }

    /// Hands the overlay window's rasterization to the window composition tree.
    ///
    /// Must succeed before the window is cloaked: cloaking hides the original
    /// window, so an uncomposed cloak would take the session off screen
    /// entirely. It also does not cloak the window itself — that is deferred
    /// until the window has been positioned and painted, which is what
    /// `cloak_pending` keeps track of.
    pub(super) fn attach(&mut self) -> anyhow::Result<()> {
        if self.attached {
            return Ok(());
        }
        let attachment = self.surface.platform_surface()?;
        // The window handle travels as a raw `usize`: navop's Win32 layer already
        // deals in bare handles, and nothing forces it to agree with the
        // `windows` crate version GPUI was built against.
        //
        // Each step carries its own context: both calls go through the same
        // platform attachment, and a bare `E_NOINTERFACE` from three frames down
        // (DirectComposition visual casts included) is not attributable without
        // it.
        attachment
            .set_window_content(Box::new(self.window))
            .context("attaching the overlay window as the portal's composition content")?;
        // The visual is placed by the first bounds update; hiding it here only
        // avoids a zero-sized portal flashing a stale rectangle. A platform that
        // refuses the hide (GPUI's Windows portal asks for an
        // `IDCompositionVisual3` opacity setter, which a composition device from
        // `DCompositionCreateDevice` does not implement) still composes the
        // session correctly — the surface starts with a zero-area clip and the
        // bounds sync below runs in the same turn — so a failed hide must not
        // throw away the attachment.
        match attachment.set_visible(false) {
            Ok(()) => self.visible = false,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    overlay_hwnd = self.window,
                    "the portal visual refused to start hidden; composing it anyway"
                );
                self.visible = true;
            }
        }
        self.attached = true;
        // Deliberately left uncloaked. Cloaking is what takes the session out of
        // the screen's Z-order *and* out of hit testing, so the session keeps
        // its input until GPUI has overlay content that needs the layer above
        // it.
        self.cloak_pending = false;
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
