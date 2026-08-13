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
const ERROR_SUCCESS: u32 = 0;

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
