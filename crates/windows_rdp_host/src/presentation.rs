//! Synchronous presentation-readiness snapshot for the native ActiveX child.
//!
//! The native host reports whether the control drawing window exists, is
//! visible, lies inside the host subtree, and has non-zero screen rects. The
//! view combines this snapshot with its own connection phase (LoginComplete /
//! Reconnected), non-zero logical bounds, and owner-window visibility before
//! declaring the native presentation ready.

use crate::ffi::NavopRdpPresentationState;

/// Presentation-readiness flags for the Windows-native ActiveX child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpPresentationState {
    /// The ActiveX drawing window handle is valid and live.
    pub control_hwnd_valid: bool,
    /// The native host window has a non-zero screen rect.
    pub host_rect_nonzero: bool,
    /// The control drawing window has a non-zero screen rect.
    pub control_rect_nonzero: bool,
    /// The control drawing window is currently visible.
    pub control_visible: bool,
    /// The control drawing window is a descendant of the native host window.
    pub control_is_host_descendant: bool,
    /// The native host window is currently visible.
    pub host_visible: bool,
}

impl WindowsRdpPresentationState {
    pub(crate) const fn from_native(native: NavopRdpPresentationState) -> Self {
        Self {
            control_hwnd_valid: native.control_hwnd_valid == 1,
            host_rect_nonzero: native.host_rect_nonzero == 1,
            control_rect_nonzero: native.control_rect_nonzero == 1,
            control_visible: native.control_visible == 1,
            control_is_host_descendant: native.control_is_host_descendant == 1,
            host_visible: native.host_visible == 1,
        }
    }

    /// Whether the ActiveX child itself is structurally ready to present:
    /// the drawing window exists inside the host subtree and both windows have
    /// non-zero rects. Visibility is deliberately excluded so callers can
    /// distinguish "ready but hidden" from "not yet created".
    pub const fn child_ready(&self) -> bool {
        self.control_hwnd_valid
            && self.control_is_host_descendant
            && self.host_rect_nonzero
            && self.control_rect_nonzero
    }
}

#[cfg(test)]
mod tests {
    use super::WindowsRdpPresentationState;
    use crate::ffi::NavopRdpPresentationState;

    fn native(flags: [bool; 6]) -> NavopRdpPresentationState {
        NavopRdpPresentationState {
            struct_size: std::mem::size_of::<NavopRdpPresentationState>() as u32,
            abi_version: crate::ffi::PRESENTATION_STATE_ABI_VERSION,
            control_hwnd_valid: u32::from(flags[0]),
            host_rect_nonzero: u32::from(flags[1]),
            control_rect_nonzero: u32::from(flags[2]),
            control_visible: u32::from(flags[3]),
            control_is_host_descendant: u32::from(flags[4]),
            host_visible: u32::from(flags[5]),
        }
    }

    #[test]
    fn from_native_preserves_flags() {
        let state = WindowsRdpPresentationState::from_native(native([
            true, false, true, false, true, true,
        ]));
        assert!(state.control_hwnd_valid);
        assert!(!state.host_rect_nonzero);
        assert!(state.control_rect_nonzero);
        assert!(!state.control_visible);
        assert!(state.control_is_host_descendant);
        assert!(state.host_visible);
    }

    #[test]
    fn child_ready_requires_structure_but_not_visibility() {
        let ready = WindowsRdpPresentationState::from_native(native([
            true, true, true, false, true, false,
        ]));
        assert!(ready.child_ready());

        let missing_control =
            WindowsRdpPresentationState::from_native(native([false, true, true, true, true, true]));
        assert!(!missing_control.child_ready());

        let outside_subtree =
            WindowsRdpPresentationState::from_native(native([true, true, true, true, false, true]));
        assert!(!outside_subtree.child_ready());

        let zero_rect =
            WindowsRdpPresentationState::from_native(native([true, false, true, true, true, true]));
        assert!(!zero_rect.child_ready());
    }
}
