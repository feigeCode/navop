use std::ffi::c_void;
use std::ptr;

use super::ffi::*;
use super::{WindowsNativeOverlayBounds, WindowsNativeOverlayError};

const WS_CHILD: u32 = 0x4000_0000;
pub(super) const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
/// Required for `IDCompositionDevice::CreateSurfaceFromHwnd`, which only wraps
/// the rasterization of a layered window.
///
/// It is deliberately NOT part of the creation styles: a layered window stays
/// invisible until `SetLayeredWindowAttributes` or `UpdateLayeredWindow` is
/// called for it. Adding the style at creation time would therefore make the
/// "composition unavailable, keep the plain child window" fallback invisible
/// rather than plain. `set_overlay_layered` applies it only once the session is
/// actually about to be composed, and takes it back when that fails.
///
/// The style alone is not enough: `CreateSurfaceFromHwnd` wraps the window's
/// *composition*, and a layered window that never received an alpha composites
/// as fully transparent — the visual would then present nothing at all, which
/// is exactly what the composed session looked like (a blank rectangle where
/// the remote desktop should be, with every DirectComposition call reporting
/// success). `set_overlay_layered` therefore also activates the layered content
/// with `SetLayeredWindowAttributes(.., 255, LWA_ALPHA)`.
const WS_EX_LAYERED: u32 = 0x0008_0000;
/// `SetLayeredWindowAttributes` flag: use `alpha` as the window's opacity.
const LWA_ALPHA: u32 = 0x0000_0002;
/// The overlay itself stays opaque: the DWM cloak is what takes it off screen,
/// so activation must not introduce transparency of its own.
const OVERLAY_LAYERED_ALPHA: u8 = 255;
const SS_BLACKRECT: u32 = 0x0000_0004;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
/// `DWMWA_CLOAK`: hides the window from the screen while the DWM keeps
/// composing it. `ShowWindow(SW_HIDE)` would drop the rasterization instead.
const DWMWA_CLOAK: u32 = 13;
const ERROR_SUCCESS: u32 = 0;
const OVERLAY_INITIAL_ORIGIN: i32 = 0;
const OVERLAY_INITIAL_EXTENT: i32 = 1;

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

pub(super) fn ensure_owner_clips_children(
    owner: *mut c_void,
) -> Result<(), WindowsNativeOverlayError> {
    let style_before = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if style_before & WS_CLIPCHILDREN as usize != 0 {
        log_owner_style(owner, style_before, style_before, false);
        return Ok(());
    }

    set_owner_clip_style(owner, style_before)?;
    let observed = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if observed & WS_CLIPCHILDREN as usize == 0 {
        return Err(WindowsNativeOverlayError::new(
            "verify_owner_clip_children",
            format!("owner style did not retain WS_CLIPCHILDREN: style=0x{observed:016X}"),
        ));
    }
    log_owner_style(owner, style_before, observed, true);
    Ok(())
}

fn set_owner_clip_style(
    owner: *mut c_void,
    style_before: usize,
) -> Result<(), WindowsNativeOverlayError> {
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    let style_after = style_before | WS_CLIPCHILDREN as usize;
    let previous = unsafe { SetWindowLongPtrW(owner, GWL_STYLE, style_after as isize) };
    let error = unsafe { GetLastError() };
    if previous == 0 && error != ERROR_SUCCESS {
        return Err(last_error_code("set_owner_clip_children", error));
    }

    let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
    let positioned = unsafe { SetWindowPos(owner, ptr::null_mut(), 0, 0, 0, 0, flags) };
    if positioned == 0 {
        return Err(last_error("refresh_owner_frame"));
    }
    Ok(())
}

fn log_owner_style(owner: *mut c_void, before: usize, after: usize, changed: bool) {
    tracing::info!(
        stage = "owner_style",
        owner_hwnd = owner as usize,
        style_before = before,
        style_after = after,
        clip_children = true,
        changed,
        "configured Windows native RDP owner clipping"
    );
}

pub(super) fn create_overlay_window(
    parent: *mut c_void,
    instance: *mut c_void,
) -> Result<*mut c_void, WindowsNativeOverlayError> {
    let overlay = unsafe {
        CreateWindowExW(
            WS_EX_NOPARENTNOTIFY,
            STATIC_CLASS.as_ptr(),
            OVERLAY_TITLE.as_ptr(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT,
            OVERLAY_INITIAL_ORIGIN,
            OVERLAY_INITIAL_ORIGIN,
            OVERLAY_INITIAL_EXTENT,
            OVERLAY_INITIAL_EXTENT,
            parent,
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    if overlay.is_null() {
        return Err(last_error("create_child_overlay"));
    }
    Ok(overlay)
}

/// Paints the overlay's whole client area synchronously, at its current size.
///
/// This is what makes cloaking safe, and the order it imposes is the difference
/// between a working session and an apparently empty one. `CreateSurfaceFromHwnd`
/// wraps the overlay's rasterization, and cloaking a window that has never been
/// on screen at its final size leaves that rasterization permanently empty: the
/// visual then contributes no pixels at all, so the composed area shows whatever
/// is behind it, and no later paint brings it back — not even the RDP frames
/// themselves — while every DirectComposition call still reports success.
///
/// Painting first fixes it. A window that has been shown at its final size and
/// painted once keeps streaming live updates through the cloak, from itself and
/// from its nested child windows, and a later resize does not disturb that. The
/// paint has to be synchronous, so the caller can cloak in the same turn and
/// needs no settle delay.
pub(super) fn redraw_overlay_window(window: *mut c_void) -> Result<(), WindowsNativeOverlayError> {
    const RDW_INVALIDATE: u32 = 0x0001;
    const RDW_ERASE: u32 = 0x0004;
    const RDW_ALLCHILDREN: u32 = 0x0080;
    const RDW_UPDATENOW: u32 = 0x0100;
    let flags = RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW;
    let redrawn = unsafe { RedrawWindow(window, ptr::null(), ptr::null_mut(), flags) };
    if redrawn == 0 {
        return Err(last_error("redraw_child_overlay"));
    }
    Ok(())
}

/// Takes the overlay off screen without stopping the DWM from composing it.
///
/// Only meaningful once the overlay's rasterization has been attached to a
/// DirectComposition visual: cloaking a plain child window would just hide the
/// session.
pub(super) fn set_overlay_cloaked(
    window: *mut c_void,
    cloaked: bool,
) -> Result<(), WindowsNativeOverlayError> {
    let value: i32 = i32::from(cloaked);
    let result = unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_CLOAK,
            ptr::from_ref(&value).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    if result < 0 {
        return Err(WindowsNativeOverlayError::new(
            if cloaked {
                "cloak_child_overlay"
            } else {
                "uncloak_child_overlay"
            },
            format!(
                "DwmSetWindowAttribute(DWMWA_CLOAK, {value}) failed: HRESULT 0x{:08X}",
                result as u32
            ),
        ));
    }
    Ok(())
}

/// Adds or removes `WS_EX_LAYERED` on the overlay.
///
/// `CreateSurfaceFromHwnd` only wraps the rasterization of a layered window, so
/// the style is what makes composition possible. It is applied here, right
/// before an attach, and removed again when the attach does not happen: a
/// layered window that nothing composes would never become visible.
pub(super) fn set_overlay_layered(
    window: *mut c_void,
    layered: bool,
) -> Result<(), WindowsNativeOverlayError> {
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
    let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
    if style_after != style_before {
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let previous = unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, style_after as isize) };
        let error = unsafe { GetLastError() };
        if previous == 0 && error != ERROR_SUCCESS {
            return Err(last_error_code(stage, error));
        }
        if unsafe { SetWindowPos(window, ptr::null_mut(), 0, 0, 0, 0, flags) } == 0 {
            // Put the previous style back so a failed refresh cannot leave the
            // window half-switched.
            unsafe {
                SetWindowLongPtrW(window, GWL_EXSTYLE, style_before as isize);
                SetWindowPos(window, ptr::null_mut(), 0, 0, 0, 0, flags);
            }
            return Err(last_error("refresh_overlay_frame"));
        }

        let observed = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize;
        if observed & WS_EX_LAYERED as usize != style_after & WS_EX_LAYERED as usize {
            return Err(WindowsNativeOverlayError::new(
                stage,
                format!(
                    "overlay extended style did not retain WS_EX_LAYERED={layered}: \
                     observed=0x{observed:016X}"
                ),
            ));
        }
    }

    if layered {
        // `SetWindowLongPtrW` only marks the window as layered; until it is given
        // an alpha the window composites as fully transparent, so the surface
        // `CreateSurfaceFromHwnd` wraps holds no visible pixels and the composed
        // session presents an empty rectangle — with every DirectComposition call
        // still reporting success. This call is what makes the wrapped
        // rasterization visible again. Failing here is reported so the session
        // falls back to the plain child window instead of composing nothing.
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        if unsafe { SetLayeredWindowAttributes(window, 0, OVERLAY_LAYERED_ALPHA, LWA_ALPHA) } == 0
        {
            let error = unsafe { GetLastError() };
            return Err(last_error_code("activate_overlay_layered", error));
        }
    }
    Ok(())
}

pub(super) fn verify_overlay_parent(
    overlay: *mut c_void,
    owner: *mut c_void,
) -> Result<(), WindowsNativeOverlayError> {
    let actual = unsafe { GetParent(overlay) };
    if actual == owner {
        return Ok(());
    }
    unsafe {
        DestroyWindow(overlay);
    }
    Err(WindowsNativeOverlayError::new(
        "verify_overlay_parent",
        format!(
            "GetParent(overlay) returned 0x{:016X}, expected 0x{:016X}",
            actual as usize, owner as usize
        ),
    ))
}

pub(super) fn position_overlay_window(
    window: *mut c_void,
    bounds: WindowsNativeOverlayBounds,
) -> Result<(), WindowsNativeOverlayError> {
    let positioned = unsafe {
        SetWindowPos(
            window,
            ptr::null_mut(),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE,
        )
    };
    if positioned == 0 {
        return Err(last_error("position_child_overlay"));
    }
    Ok(())
}

pub(super) fn last_error(stage: &'static str) -> WindowsNativeOverlayError {
    last_error_code(stage, unsafe { GetLastError() })
}

fn last_error_code(stage: &'static str, code: u32) -> WindowsNativeOverlayError {
    WindowsNativeOverlayError::new(stage, format!("Win32 code 0x{code:08X} ({code})"))
}
