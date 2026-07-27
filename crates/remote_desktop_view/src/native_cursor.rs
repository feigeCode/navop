#[cfg(target_os = "windows")]
mod windows {
    use std::ffi::c_void;

    const IDC_ARROW: usize = 32_512;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetCursor(cursor: *mut c_void) -> *mut c_void;
        fn LoadCursorW(instance: *mut c_void, cursor_name: *const u16) -> *mut c_void;
    }

    pub(super) fn hide() {
        // SAFETY: A null HCURSOR is the Win32 contract for clearing the
        // thread's current cursor. This runs only on GPUI's window thread.
        unsafe {
            SetCursor(std::ptr::null_mut());
        }
    }

    pub(super) fn restore() {
        // SAFETY: A null module handle plus IDC_ARROW requests the shared
        // system arrow cursor; the returned shared handle must not be freed.
        let arrow = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16) };
        if !arrow.is_null() {
            // SAFETY: `arrow` is a valid shared cursor handle returned above.
            unsafe {
                SetCursor(arrow);
            }
        }
    }
}

pub(crate) fn hide() {
    #[cfg(target_os = "windows")]
    windows::hide();
}

pub(crate) fn restore() {
    #[cfg(target_os = "windows")]
    windows::restore();
}
