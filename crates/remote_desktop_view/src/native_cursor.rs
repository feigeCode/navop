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
    #[cfg(test)]
    probe::record(probe::Call::Hide);
    #[cfg(target_os = "windows")]
    windows::hide();
}

pub(crate) fn restore() {
    #[cfg(test)]
    probe::record(probe::Call::Restore);
    #[cfg(target_os = "windows")]
    windows::restore();
}

/// Counts the `SetCursor` calls the remote desktop view makes, so tests can
/// assert who owns the OS cursor without a window server.
#[cfg(test)]
pub(crate) mod probe {
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Call {
        Hide,
        Restore,
    }

    thread_local! {
        static HIDES: Cell<usize> = const { Cell::new(0) };
        static RESTORES: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record(call: Call) {
        match call {
            Call::Hide => HIDES.with(|count| count.set(count.get() + 1)),
            Call::Restore => RESTORES.with(|count| count.set(count.get() + 1)),
        }
    }

    pub(crate) fn count(call: Call) -> usize {
        match call {
            Call::Hide => HIDES.with(Cell::get),
            Call::Restore => RESTORES.with(Cell::get),
        }
    }

    pub(crate) fn reset() {
        HIDES.with(|count| count.set(0));
        RESTORES.with(|count| count.set(0));
    }
}
