use std::ffi::c_void;
use std::ptr;

use super::native_overlay_ffi::*;

const WS_POPUP: u32 = 0x8000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const SW_HIDE: i32 = 0;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const GW_OWNER: u32 = 4;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;

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
struct ScreenBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

pub(crate) struct NativeOverlay {
    owner: usize,
    window: usize,
    last_bounds: Option<ScreenBounds>,
    shown: bool,
}

impl NativeOverlay {
    pub(crate) fn create(owner: usize) -> Result<Self, String> {
        let owner_window = window_pointer(owner);
        if owner == 0 || unsafe { IsWindow(owner_window) } == 0 {
            return Err("GPUI owner HWND is not a live window".to_owned());
        }
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        if instance.is_null() {
            return Err(last_error("GetModuleHandleW(current process)"));
        }
        let overlay = create_overlay_window(owner_window, instance)?;
        let overlay_owner = unsafe { GetWindow(overlay, GW_OWNER) } as usize;
        let style = unsafe { GetWindowLongPtrW(overlay, GWL_STYLE) };
        let ex_style = unsafe { GetWindowLongPtrW(overlay, GWL_EXSTYLE) };
        println!(
            "presentation: overlay_created hwnd=0x{:016X} owner=0x{overlay_owner:016X} style=0x{style:016X} ex_style=0x{ex_style:016X}",
            overlay as usize
        );
        if overlay_owner != owner {
            unsafe {
                DestroyWindow(overlay);
            }
            return Err(format!(
                "owned RDP overlay retained unexpected owner 0x{overlay_owner:016X}"
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

    pub(crate) fn synchronize(
        &mut self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        let Some(bounds) = self.screen_bounds(local_x, local_y, width, height)? else {
            self.hide()?;
            return Ok(());
        };

        let visible = unsafe { IsWindowVisible(window_pointer(self.window)) } != 0;
        let changed = self.last_bounds != Some(bounds) || !self.shown || !visible;
        if !changed {
            return Ok(());
        }
        let positioned = unsafe {
            SetWindowPos(
                window_pointer(self.window),
                ptr::null_mut(),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        if positioned == 0 {
            return Err(last_error("SetWindowPos(owned RDP overlay)"));
        }

        self.shown = true;
        self.last_bounds = Some(bounds);
        self.log_observed_bounds(bounds)?;
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
            return Err("ShowWindow(SW_HIDE) left the owned RDP overlay visible".to_owned());
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
            return Err(last_error("DestroyWindow(owned RDP overlay)"));
        }
        self.window = 0;
        self.last_bounds = None;
        println!("presentation: overlay_destroyed");
        Ok(())
    }

    fn screen_bounds(
        &self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<ScreenBounds>, String> {
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
        let left = local_x.max(client.left);
        let top = local_y.max(client.top);
        let right = local_x.saturating_add(width).min(client.right);
        let bottom = local_y.saturating_add(height).min(client.bottom);
        if right <= left || bottom <= top {
            return Ok(None);
        }

        let mut origin = Point { x: left, y: top };
        if unsafe { ClientToScreen(owner, &mut origin) } == 0 {
            return Err(last_error("ClientToScreen(GPUI owner)"));
        }
        Ok(Some(ScreenBounds {
            x: origin.x,
            y: origin.y,
            width: right - left,
            height: bottom - top,
        }))
    }

    fn log_observed_bounds(&self, expected: ScreenBounds) -> Result<(), String> {
        let mut observed = Rect::default();
        if unsafe { GetWindowRect(window_pointer(self.window), &mut observed) } == 0 {
            return Err(last_error("GetWindowRect(owned RDP overlay)"));
        }
        println!(
            "presentation: overlay_bounds expected={{x={},y={},width={},height={}}} observed={{left={},top={},right={},bottom={}}} visible={}",
            expected.x,
            expected.y,
            expected.width,
            expected.height,
            observed.left,
            observed.top,
            observed.right,
            observed.bottom,
            unsafe { IsWindowVisible(window_pointer(self.window)) } != 0
        );
        Ok(())
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

fn create_overlay_window(owner: *mut c_void, instance: *mut c_void) -> Result<*mut c_void, String> {
    let overlay = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            STATIC_CLASS.as_ptr(),
            OVERLAY_TITLE.as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            owner,
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    if overlay.is_null() {
        return Err(last_error("CreateWindowExW(owned RDP overlay)"));
    }
    Ok(overlay)
}

fn last_error(operation: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{operation} failed with Win32 code 0x{code:08X} ({code})")
}
