//! Screen-space port layout for canvas rendering ([`PortScreenFrame`]).

use gpui::{Div, Pixels, Point, Size, Styled as _, div};

/// Screen-space layout for one port after the viewport transform.
///
/// Prefer resolving via [`crate::plugin::RenderContext::port_screen_frame`] (or
/// [`crate::plugin::PluginContext::port_screen_frame`] during interaction).
///
/// Typical patterns:
/// - Default disc: [`Self::anchor_div`] then chain `.rounded_full()`, colors, borders.
/// - Custom chrome: build children inside [`Self::anchor_div`], or use [`Self::center`]
///   / [`Self::scaled_size`] for labels, sockets, multi-layer ports.
/// - Larger hit target: [`Self::anchor_div`] then override `.w`/`.h` while keeping [`Self::center`].
#[derive(Clone, Copy, Debug)]
pub struct PortScreenFrame {
    /// Port center in screen pixels (aligned with edge curve endpoints).
    pub center: Point<Pixels>,
    /// Logical port size from graph data (same units as on the node card).
    pub size: Size<Pixels>,
    pub zoom: f32,
}

impl PortScreenFrame {
    /// `size` scaled by [`Self::zoom`], i.e. the on-screen port box size.
    pub fn scaled_size(&self) -> Size<Pixels> {
        let z = self.zoom;
        Size {
            width: self.size.width * z,
            height: self.size.height * z,
        }
    }

    /// Top-left of the axis-aligned rectangle centered on [`Self::center`].
    pub fn origin(&self) -> Point<Pixels> {
        let s = self.scaled_size();
        Point::new(
            self.center.x - s.width / 2.0,
            self.center.y - s.height / 2.0,
        )
    }

    /// `absolute` container covering the default port hit box; chain GPUI styles and children.
    pub fn anchor_div(self) -> Div {
        let s = self.scaled_size();
        let o = self.origin();
        div().absolute().left(o.x).top(o.y).w(s.width).h(s.height)
    }
}
