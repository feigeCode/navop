use std::ffi::c_void;
use std::ptr;

use super::ffi::*;
use super::{WindowsNativeOverlayBounds, WindowsNativeOverlayError};

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
const GWL_STYLE: i32 = -16;
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
