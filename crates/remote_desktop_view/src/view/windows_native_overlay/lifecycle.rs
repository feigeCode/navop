use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;

use super::ffi::*;
use super::window::{
    create_overlay_window, ensure_owner_clips_children, last_error, position_overlay_window,
    verify_overlay_parent,
};
use super::{
    WindowsNativeOverlay, WindowsNativeOverlayBounds, WindowsNativeOverlayError, diagnostics,
    window_pointer,
};

const SW_HIDE: i32 = 0;
const SW_SHOWNA: i32 = 8;

impl WindowsNativeOverlay {
    pub(crate) fn create(owner: usize, generation: u64) -> Result<Self, WindowsNativeOverlayError> {
        let owner_window = window_pointer(owner);
        validate_owner(owner_window)?;
        let owner_thread = owner_thread(owner_window)?;
        validate_thread(owner_thread, "create_child_overlay")?;
        ensure_owner_clips_children(owner_window)?;

        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        if instance.is_null() {
            return Err(last_error("get_current_module"));
        }
        let overlay = create_overlay_window(owner_window, instance)?;
        verify_overlay_parent(overlay, owner_window)?;

        let overlay = Self {
            owner,
            window: overlay as usize,
            owner_thread,
            generation,
            last_bounds: None,
            requested_visible: false,
            _thread_affinity: PhantomData,
        };
        diagnostics::log_created(&overlay);
        Ok(overlay)
    }

    pub(crate) fn hwnd(&self) -> usize {
        self.window
    }

    /// Whether the overlay HWND is actually visible right now, independent of
    /// any requested visibility. A destroyed overlay reports false.
    pub(crate) fn is_actually_visible(&self) -> bool {
        self.window != 0 && unsafe { IsWindowVisible(window_pointer(self.window)) } != 0
    }

    pub(crate) fn set_bounds(
        &mut self,
        requested: WindowsNativeOverlayBounds,
    ) -> Result<Option<WindowsNativeOverlayBounds>, WindowsNativeOverlayError> {
        self.validate_mutation("set_bounds")?;
        let Some(bounds) = self.clipped_bounds(requested)? else {
            self.last_bounds = None;
            self.hide_actual()?;
            diagnostics::log_bounds(self, requested, None);
            return Ok(None);
        };

        position_overlay_window(window_pointer(self.window), bounds)?;
        self.last_bounds = Some(bounds);
        if self.requested_visible {
            self.show_actual()?;
        }
        diagnostics::log_bounds(self, requested, Some(bounds));
        Ok(Some(bounds))
    }

    pub(crate) fn show(&mut self) -> Result<(), WindowsNativeOverlayError> {
        self.validate_mutation("show")?;
        self.requested_visible = true;
        if self.last_bounds.is_some() && self.owner_can_present() {
            self.show_actual()?;
        }
        diagnostics::log_visibility(self, "show");
        Ok(())
    }

    pub(crate) fn hide(&mut self) -> Result<(), WindowsNativeOverlayError> {
        if self.window == 0 {
            return Ok(());
        }
        self.validate_mutation("hide")?;
        self.requested_visible = false;
        self.hide_actual()?;
        diagnostics::log_visibility(self, "hide");
        Ok(())
    }

    pub(crate) fn log_composition_diagnostics(&self, reason: &'static str) {
        diagnostics::log_composition_diagnostics(self, reason);
    }

    pub(crate) fn close(&mut self) -> Result<(), WindowsNativeOverlayError> {
        if self.window == 0 {
            return Ok(());
        }
        validate_thread(self.owner_thread, "destroy_child_overlay")?;
        self.requested_visible = false;
        let hide_error = self.hide_actual().err();

        let window = window_pointer(self.window);
        if unsafe { IsWindow(window) } != 0 && unsafe { DestroyWindow(window) } == 0 {
            let destroy_error = last_error("destroy_child_overlay");
            if let Some(error) = hide_error {
                tracing::warn!(
                    ?error,
                    ?destroy_error,
                    "failed to hide Windows native RDP overlay before destroy also failed"
                );
            }
            return Err(destroy_error);
        }
        if let Some(error) = hide_error {
            tracing::warn!(
                ?error,
                "failed to hide Windows native RDP overlay before destroy"
            );
        }
        tracing::info!(
            stage = "overlay_destroyed",
            generation = self.generation,
            owner_hwnd = self.owner,
            overlay_hwnd = self.window,
            "destroyed Windows native RDP overlay"
        );
        self.window = 0;
        self.last_bounds = None;
        Ok(())
    }

    pub(crate) fn abandon(&mut self, reason: &'static str) {
        if self.window == 0 {
            return;
        }
        tracing::error!(
            stage = "overlay_abandoned",
            generation = self.generation,
            owner_hwnd = self.owner,
            overlay_hwnd = self.window,
            reason,
            "leaking Windows native RDP overlay to preserve the live host parent"
        );
        self.window = 0;
        self.last_bounds = None;
    }

    fn validate_mutation(&self, stage: &'static str) -> Result<(), WindowsNativeOverlayError> {
        validate_thread(self.owner_thread, stage)?;
        if unsafe { IsWindow(window_pointer(self.window)) } == 0 {
            return Err(WindowsNativeOverlayError::new(
                stage,
                "overlay HWND is no longer live".to_owned(),
            ));
        }
        if unsafe { IsWindow(window_pointer(self.owner)) } == 0 {
            return Err(WindowsNativeOverlayError::new(
                stage,
                "GPUI owner HWND was destroyed before the overlay".to_owned(),
            ));
        }
        Ok(())
    }

    fn clipped_bounds(
        &self,
        requested: WindowsNativeOverlayBounds,
    ) -> Result<Option<WindowsNativeOverlayBounds>, WindowsNativeOverlayError> {
        if requested.width <= 0 || requested.height <= 0 || !self.owner_can_present() {
            return Ok(None);
        }
        let mut client = Rect::default();
        if unsafe { GetClientRect(window_pointer(self.owner), &mut client) } == 0 {
            return Err(last_error("read_owner_client_bounds"));
        }
        Ok(intersect_bounds(requested, client))
    }

    fn owner_can_present(&self) -> bool {
        let owner = window_pointer(self.owner);
        unsafe { IsWindowVisible(owner) != 0 && IsIconic(owner) == 0 }
    }

    fn show_actual(&self) -> Result<(), WindowsNativeOverlayError> {
        let window = window_pointer(self.window);
        unsafe {
            ShowWindow(window, SW_SHOWNA);
        }
        if unsafe { IsWindowVisible(window) } == 0 {
            return Err(WindowsNativeOverlayError::new(
                "show_child_overlay",
                "ShowWindow(SW_SHOWNA) left the overlay hidden".to_owned(),
            ));
        }
        Ok(())
    }

    fn hide_actual(&self) -> Result<(), WindowsNativeOverlayError> {
        let window = window_pointer(self.window);
        if unsafe { IsWindowVisible(window) } == 0 {
            return Ok(());
        }
        unsafe {
            ShowWindow(window, SW_HIDE);
        }
        if unsafe { IsWindowVisible(window) } != 0 {
            return Err(WindowsNativeOverlayError::new(
                "hide_child_overlay",
                "ShowWindow(SW_HIDE) left the overlay visible".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Drop for WindowsNativeOverlay {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::error!(
                stage = "drop_child_overlay",
                generation = self.generation,
                owner_hwnd = self.owner,
                overlay_hwnd = self.window,
                ?error,
                "failed to destroy Windows native RDP overlay during drop"
            );
        }
    }
}

fn validate_owner(owner: *mut c_void) -> Result<(), WindowsNativeOverlayError> {
    if owner.is_null() || unsafe { IsWindow(owner) } == 0 {
        return Err(WindowsNativeOverlayError::new(
            "validate_owner",
            "GPUI owner HWND is not a live window".to_owned(),
        ));
    }
    Ok(())
}

fn owner_thread(owner: *mut c_void) -> Result<u32, WindowsNativeOverlayError> {
    let thread = unsafe { GetWindowThreadProcessId(owner, ptr::null_mut()) };
    if thread == 0 {
        return Err(last_error("read_owner_thread"));
    }
    Ok(thread)
}

fn validate_thread(expected: u32, stage: &'static str) -> Result<(), WindowsNativeOverlayError> {
    let actual = unsafe { GetCurrentThreadId() };
    if actual == expected {
        return Ok(());
    }
    Err(WindowsNativeOverlayError::new(
        stage,
        format!("wrong UI thread: expected={expected}, actual={actual}"),
    ))
}

fn intersect_bounds(
    requested: WindowsNativeOverlayBounds,
    client: Rect,
) -> Option<WindowsNativeOverlayBounds> {
    let left = requested.x.max(client.left);
    let top = requested.y.max(client.top);
    let right = requested.right().min(client.right);
    let bottom = requested.bottom().min(client.bottom);
    (right > left && bottom > top).then_some(WindowsNativeOverlayBounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}
