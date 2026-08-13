use std::ffi::c_void;
use std::ptr;

use super::native_overlay_ffi::*;

const WS_CHILD: u32 = 0x4000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
const SS_BLACKRECT: u32 = 0x0000_0004;
const SW_HIDE: i32 = 0;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_FRAMECHANGED: u32 = 0x0020;
const SWP_SHOWWINDOW: u32 = 0x0040;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const GW_HWNDFIRST: u32 = 0;
const GW_HWNDLAST: u32 = 1;
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
struct ChildBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

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

    pub(crate) fn synchronize(
        &mut self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        self.synchronize_impl(local_x, local_y, width, height, false)
    }

    pub(crate) fn refresh(
        &mut self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        self.synchronize_impl(local_x, local_y, width, height, true)
    }

    fn synchronize_impl(
        &mut self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
        force: bool,
    ) -> Result<(), String> {
        let Some(bounds) = self.child_bounds(local_x, local_y, width, height)? else {
            self.hide()?;
            return Ok(());
        };

        let visible = unsafe { IsWindowVisible(window_pointer(self.window)) } != 0;
        let changed = self.last_bounds != Some(bounds) || !self.shown || !visible;
        if !changed && !force {
            return Ok(());
        }
        let positioned = unsafe {
            SetWindowPos(
                window_pointer(self.window),
                // A null insert-after value is HWND_TOP. Reassert the native
                // child above any future GPUI child siblings on forced refresh.
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

        self.shown = true;
        self.last_bounds = Some(bounds);
        self.log_observed_state(bounds, force)?;
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

    fn child_bounds(
        &self,
        local_x: i32,
        local_y: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<ChildBounds>, String> {
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

        Ok(Some(ChildBounds {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }))
    }

    fn log_observed_state(&self, expected: ChildBounds, forced: bool) -> Result<(), String> {
        let mut observed = Rect::default();
        if unsafe { GetWindowRect(window_pointer(self.window), &mut observed) } == 0 {
            return Err(last_error("GetWindowRect(child RDP overlay)"));
        }
        let owner = window_pointer(self.owner);
        let mut observed_origin = Point {
            x: observed.left,
            y: observed.top,
        };
        if unsafe { ScreenToClient(owner, &mut observed_origin) } == 0 {
            return Err(last_error("ScreenToClient(GPUI owner)"));
        }
        let owner_style = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
        let first_sibling =
            unsafe { GetWindow(window_pointer(self.window), GW_HWNDFIRST) } as usize;
        let last_sibling = unsafe { GetWindow(window_pointer(self.window), GW_HWNDLAST) } as usize;
        println!(
            "presentation: overlay_state expected={{x={},y={},width={},height={}}} observed={{x={},y={},width={},height={}}} visible={} forced={} owner_style=0x{owner_style:016X} owner_clip_children={} first_sibling=0x{first_sibling:016X} last_sibling=0x{last_sibling:016X} overlay_is_first={}",
            expected.x,
            expected.y,
            expected.width,
            expected.height,
            observed_origin.x,
            observed_origin.y,
            observed.right - observed.left,
            observed.bottom - observed.top,
            unsafe { IsWindowVisible(window_pointer(self.window)) } != 0,
            forced,
            owner_style & WS_CLIPCHILDREN as usize != 0,
            first_sibling == self.window,
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

fn ensure_owner_clips_children(owner: *mut c_void) -> Result<(), String> {
    let style_before = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if style_before & WS_CLIPCHILDREN as usize != 0 {
        println!(
            "presentation: owner_style hwnd=0x{:016X} before=0x{style_before:016X} after=0x{style_before:016X} clip_children=true changed=false",
            owner as usize
        );
        return Ok(());
    }

    let style_after = style_before | WS_CLIPCHILDREN as usize;
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    let previous = unsafe { SetWindowLongPtrW(owner, GWL_STYLE, style_after as isize) } as usize;
    let style_error = unsafe { GetLastError() };
    if previous == 0 && style_error != ERROR_SUCCESS {
        return Err(format!(
            "SetWindowLongPtrW(GPUI owner, WS_CLIPCHILDREN) failed with Win32 code 0x{style_error:08X} ({style_error})"
        ));
    }
    if unsafe {
        SetWindowPos(
            owner,
            ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    } == 0
    {
        return Err(last_error("SetWindowPos(GPUI owner after WS_CLIPCHILDREN)"));
    }

    let observed = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    if observed & WS_CLIPCHILDREN as usize == 0 {
        return Err(format!(
            "GPUI owner style did not retain WS_CLIPCHILDREN: observed=0x{observed:016X}"
        ));
    }
    println!(
        "presentation: owner_style hwnd=0x{:016X} before=0x{style_before:016X} after=0x{observed:016X} clip_children=true changed=true",
        owner as usize
    );
    Ok(())
}

fn create_overlay_window(
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

fn last_error(operation: &str) -> String {
    let code = unsafe { GetLastError() };
    format!("{operation} failed with Win32 code 0x{code:08X} ({code})")
}
