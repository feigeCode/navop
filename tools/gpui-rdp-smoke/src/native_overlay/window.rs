use std::ffi::c_void;
use std::ptr;

use crate::native_overlay_ffi::*;

const WS_CHILD: u32 = 0x4000_0000;
pub(super) const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
const SS_BLACKRECT: u32 = 0x0000_0004;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const SWP_SHOWWINDOW: u32 = 0x0040;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const ERROR_SUCCESS: u32 = 0;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const LWA_ALPHA: u32 = 0x0000_0002;
const OVERLAY_LAYERED_ALPHA: u8 = 255;
const DWMWA_CLOAK: u32 = 13;
const DWMWA_CLOAKED: u32 = 14;

const STATIC_CLASS: [u16; 7] = [
    b'S' as u16,
    b'T' as u16,
    b'A' as u16,
    b'T' as u16,
    b'I' as u16,
    b'C' as u16,
    0,
];
const OVERLAY_TITLE: [u16; 18] = [
    b'N' as u16,
    b'a' as u16,
    b'v' as u16,
    b'o' as u16,
    b'p' as u16,
    b' ' as u16,
    b'R' as u16,
    b'D' as u16,
    b'P' as u16,
    b' ' as u16,
    b'O' as u16,
    b'v' as u16,
    b'e' as u16,
    b'r' as u16,
    b'l' as u16,
    b'a' as u16,
    b'y' as u16,
    0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChildBounds {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

impl ChildBounds {
    pub(super) fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub(super) fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }
}

impl From<(i32, i32, i32, i32)> for ChildBounds {
    fn from(value: (i32, i32, i32, i32)) -> Self {
        Self {
            x: value.0,
            y: value.1,
            width: value.2,
            height: value.3,
        }
    }
}

pub(super) fn ensure_owner_clips_children(owner: *mut c_void) -> Result<(), String> {
    let style_before = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if style_before & WS_CLIPCHILDREN as usize != 0 {
        log_owner_style(owner, style_before, style_before, false);
        return Ok(());
    }
    set_owner_clip_style(owner, style_before)?;
    let observed = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if observed & WS_CLIPCHILDREN as usize == 0 {
        return Err(format!(
            "GPUI owner style did not retain WS_CLIPCHILDREN: observed=0x{observed:016X}"
        ));
    }
    log_owner_style(owner, style_before, observed, true);
    Ok(())
}

fn set_owner_clip_style(owner: *mut c_void, style_before: usize) -> Result<(), String> {
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    let style_after = style_before | WS_CLIPCHILDREN as usize;
    let previous = unsafe { SetWindowLongPtrW(owner, GWL_STYLE, style_after as isize) };
    let style_error = unsafe { GetLastError() };
    if previous == 0 && style_error != ERROR_SUCCESS {
        return Err(last_error_code(
            "SetWindowLongPtrW(GPUI owner, WS_CLIPCHILDREN)",
            style_error,
        ));
    }
    let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
    let positioned = unsafe { SetWindowPos(owner, ptr::null_mut(), 0, 0, 0, 0, flags) };
    if positioned == 0 {
        return Err(last_error("SetWindowPos(GPUI owner after WS_CLIPCHILDREN)"));
    }
    Ok(())
}

fn log_owner_style(owner: *mut c_void, before: usize, after: usize, changed: bool) {
    println!(
        "presentation: owner_style hwnd=0x{:016X} before=0x{before:016X} after=0x{after:016X} clip_children=true changed={changed}",
        owner as usize
    );
}

pub(super) fn create_overlay_window(
    parent: *mut c_void,
    instance: *mut c_void,
) -> Result<*mut c_void, String> {
    let overlay = unsafe {
        CreateWindowExW(
            WS_EX_NOPARENTNOTIFY,
            STATIC_CLASS.as_ptr(),
            OVERLAY_TITLE.as_ptr(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT,
            0,
            0,
            1,
            1,
            parent,
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    if overlay.is_null() {
        return Err(last_error("CreateWindowExW(child RDP overlay)"));
    }
    Ok(overlay)
}

pub(super) fn position_overlay_window(
    window: *mut c_void,
    bounds: ChildBounds,
) -> Result<(), String> {
    let positioned = unsafe {
        SetWindowPos(
            window,
            ptr::null_mut(),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
    if positioned == 0 {
        return Err(last_error("SetWindowPos(child RDP overlay)"));
    }
    Ok(())
}

pub(super) fn last_error(operation: &str) -> String {
    last_error_code(operation, unsafe { GetLastError() })
}

fn last_error_code(operation: &str, code: u32) -> String {
    format!("{operation} failed with Win32 code 0x{code:08X} ({code})")
}

/// Applies or removes `WS_EX_LAYERED`, and — when applying it — gives the window
/// the alpha its layered composition needs.
///
/// `SetWindowLongPtrW` alone only marks the window layered: until
/// `SetLayeredWindowAttributes` supplies an alpha the window composites as fully
/// transparent, so `CreateSurfaceFromHwnd` wraps a surface that holds no visible
/// pixels and the composed session presents an empty rectangle while every
/// DirectComposition call still reports success.
pub(super) fn set_overlay_layered(window: *mut c_void, layered: bool) -> Result<(), String> {
    let stage = if layered {
        "add_overlay_layered"
    } else {
        "remove_overlay_layered"
    };
    let style_before = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize;
    let style_after = if layered {
        style_before | WS_EX_LAYERED as usize
    } else {
        style_before & !(WS_EX_LAYERED as usize)
    };
    if style_after != style_before {
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let previous = unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, style_after as isize) };
        let error = unsafe { GetLastError() };
        if previous == 0 && error != ERROR_SUCCESS {
            return Err(last_error_code(stage, error));
        }
        let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
        if unsafe { SetWindowPos(window, ptr::null_mut(), 0, 0, 0, 0, flags) } == 0 {
            return Err(last_error("refresh_overlay_frame"));
        }
    }
    if layered {
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let applied =
            unsafe { SetLayeredWindowAttributes(window, 0, OVERLAY_LAYERED_ALPHA, LWA_ALPHA) };
        if applied == 0 {
            let error = unsafe { GetLastError() };
            return Err(last_error_code("activate_overlay_layered", error));
        }
    }
    Ok(())
}

/// Paints the overlay's whole client area synchronously, at its current size.
///
/// This is what makes `DWMWA_CLOAK` safe, and the order is what matters:
/// cloaking a window that has never been shown and painted at its final size
/// leaves the composition surface permanently empty. The visual then contributes
/// no pixels at all — the composed area shows whatever is behind it — and no
/// later paint brings it back, not even the RDP frames themselves, while every
/// DirectComposition call still reports success.
///
/// A window that has been shown at its final size and painted once keeps
/// streaming live updates through the cloak, both from itself and from its
/// nested GDI children, and a later resize does not disturb that. Painting right
/// before cloaking is therefore sufficient, and needs no settle time.
pub(super) fn redraw_overlay_window(window: *mut c_void) -> Result<(), String> {
    const RDW_INVALIDATE: u32 = 0x0001;
    const RDW_ERASE: u32 = 0x0004;
    const RDW_ALLCHILDREN: u32 = 0x0080;
    const RDW_UPDATENOW: u32 = 0x0100;
    let flags = RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW;
    if unsafe { RedrawWindow(window, ptr::null(), ptr::null_mut(), flags) } == 0 {
        return Err(last_error("RedrawWindow(child RDP overlay)"));
    }
    Ok(())
}

/// Cloaks or uncloaks the overlay through DWM.
///
/// Cloaking takes the window off screen while letting the system keep composing
/// its content into the composition visual.
pub(super) fn set_overlay_cloaked(window: *mut c_void, cloaked: bool) -> Result<(), String> {
    let flag: i32 = i32::from(cloaked);
    let attribute = &flag as *const i32;
    let result = unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_CLOAK,
            attribute.cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    if result < 0 {
        return Err(format!(
            "DwmSetWindowAttribute(DWMWA_CLOAK={cloaked}) failed with HRESULT 0x{:08X}",
            result as u32
        ));
    }
    Ok(())
}

/// Reads back `DWMWA_CLOAKED` so the cloak state can be asserted, not assumed.
pub(super) fn overlay_cloaked(window: *mut c_void) -> Result<i32, String> {
    let mut value: i32 = 0;
    let attribute = &mut value as *mut i32;
    let result = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            attribute.cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    if result < 0 {
        return Err(format!(
            "DwmGetWindowAttribute(DWMWA_CLOAKED) failed with HRESULT 0x{:08X}",
            result as u32
        ));
    }
    Ok(value)
}
