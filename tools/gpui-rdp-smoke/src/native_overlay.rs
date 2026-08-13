use std::{ffi::c_void, ptr};

use super::native_overlay_ffi::*;

mod diagnostics;
mod window;

use window::{
    ChildBounds, create_overlay_window, ensure_owner_clips_children, last_error,
    position_overlay_window,
};

const SW_HIDE: i32 = 0;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;

pub(crate) struct NativeOverlay {
    owner: usize,
    window: usize,
    last_bounds: Option<ChildBounds>,
    shown: bool,
}

impl NativeOverlay {
    pub(crate) fn create(owner: usize) -> Result<Self, String> {
        let owner_window = window_pointer(owner);
        if owner == 0 || unsafe { IsWindow(owner_window) } == 0 {
            return Err("GPUI owner HWND is not a live window".to_owned());
        }
        ensure_owner_clips_children(owner_window)?;
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        if instance.is_null() {
            return Err(last_error("GetModuleHandleW(current process)"));
        }
        let overlay = create_overlay_window(owner_window, instance)?;
        let overlay_parent = unsafe { GetParent(overlay) } as usize;
        let style = unsafe { GetWindowLongPtrW(overlay, GWL_STYLE) };
        let ex_style = unsafe { GetWindowLongPtrW(overlay, GWL_EXSTYLE) };
        println!(
            "presentation: overlay_created hwnd=0x{:016X} parent=0x{overlay_parent:016X} style=0x{style:016X} ex_style=0x{ex_style:016X}",
            overlay as usize
        );
        if overlay_parent != owner {
            unsafe {
                DestroyWindow(overlay);
            }
            return Err(format!(
                "child RDP overlay retained unexpected parent 0x{overlay_parent:016X}"
            ));
        }

        Ok(Self {
            owner,
            window: overlay as usize,
            last_bounds: None,
            shown: false,
        })
    }

    pub(crate) fn hwnd(&self) -> usize {
        self.window
    }

    pub(crate) fn log_composition_diagnostics(&self, reason: &str) {
        diagnostics::log_composition_diagnostics(self, reason);
    }

    pub(crate) fn synchronize(&mut self, requested: (i32, i32, i32, i32)) -> Result<(), String> {
        self.synchronize_impl(ChildBounds::from(requested), false)
    }

    pub(crate) fn refresh(&mut self, requested: (i32, i32, i32, i32)) -> Result<(), String> {
        self.synchronize_impl(ChildBounds::from(requested), true)
    }

    fn synchronize_impl(&mut self, requested: ChildBounds, force: bool) -> Result<(), String> {
        let Some(bounds) = self.child_bounds(requested)? else {
            self.hide()?;
            return Ok(());
        };

        let visible = unsafe { IsWindowVisible(window_pointer(self.window)) } != 0;
        let changed = self.last_bounds != Some(bounds) || !self.shown || !visible;
        if !changed && !force {
            return Ok(());
        }
        position_overlay_window(window_pointer(self.window), bounds)?;

        self.shown = true;
        self.last_bounds = Some(bounds);
        diagnostics::log_overlay_state(self, bounds, force)?;
        Ok(())
    }

    pub(crate) fn hide(&mut self) -> Result<(), String> {
        if self.window == 0 {
            return Ok(());
        }
        if !self.shown && unsafe { IsWindowVisible(window_pointer(self.window)) } == 0 {
            return Ok(());
        }
        unsafe {
            ShowWindow(window_pointer(self.window), SW_HIDE);
        }
        if unsafe { IsWindowVisible(window_pointer(self.window)) } != 0 {
            return Err("ShowWindow(SW_HIDE) left the child RDP overlay visible".to_owned());
        }
        self.shown = false;
        self.last_bounds = None;
        Ok(())
    }

    pub(crate) fn close(&mut self) -> Result<(), String> {
        if self.window == 0 {
            return Ok(());
        }
        self.hide()?;
        let window = window_pointer(self.window);
        if unsafe { IsWindow(window) } == 0 {
            self.window = 0;
            return Ok(());
        }
        if unsafe { DestroyWindow(window) } == 0 {
            return Err(last_error("DestroyWindow(child RDP overlay)"));
        }
        self.window = 0;
        self.last_bounds = None;
        println!("presentation: overlay_destroyed");
        Ok(())
    }

    fn child_bounds(&self, requested: ChildBounds) -> Result<Option<ChildBounds>, String> {
        let owner = window_pointer(self.owner);
        if unsafe { IsWindow(owner) } == 0 {
            return Err("GPUI owner HWND was destroyed before the RDP overlay".to_owned());
        }
        if unsafe { IsWindowVisible(owner) } == 0 || unsafe { IsIconic(owner) } != 0 {
            return Ok(None);
        }

        let mut client = Rect::default();
        if unsafe { GetClientRect(owner, &mut client) } == 0 {
            return Err(last_error("GetClientRect(GPUI owner)"));
        }
        let left = requested.x.max(client.left);
        let top = requested.y.max(client.top);
        let right = requested.right().min(client.right);
        let bottom = requested.bottom().min(client.bottom);
        if right <= left || bottom <= top {
            return Ok(None);
        }

        Ok(Some(ChildBounds {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }))
    }
}

impl Drop for NativeOverlay {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("ERROR: stage=drop_native_overlay error={error}");
        }
    }
}

fn window_pointer(window: usize) -> *mut c_void {
    window as *mut c_void
}
