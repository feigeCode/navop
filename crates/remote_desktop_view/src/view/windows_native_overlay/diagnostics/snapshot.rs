use std::ffi::c_void;

use super::super::ffi::*;
use super::super::window_pointer;
use super::DiagnosticContext;

const GW_HWNDNEXT: u32 = 2;
const GW_HWNDPREV: u32 = 3;
const GW_OWNER: u32 = 4;
const GA_PARENT: u32 = 1;
const GA_ROOT: u32 = 2;
const GA_ROOTOWNER: u32 = 3;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const CLASS_NAME_CAPACITY: usize = 256;

pub(super) struct WindowLogEntry {
    kind: &'static str,
    index: usize,
}

impl WindowLogEntry {
    pub(super) fn new(kind: &'static str, index: usize) -> Self {
        Self { kind, index }
    }
}

pub(super) struct WindowSnapshot {
    window: usize,
    class_name: String,
    parent: usize,
    owner: usize,
    root: usize,
    root_owner: usize,
    previous: usize,
    next: usize,
    style: usize,
    ex_style: usize,
    visible: bool,
    enabled: bool,
    thread_id: u32,
    process_id: u32,
    dpi: u32,
}

impl WindowSnapshot {
    pub(super) fn capture(window: *mut c_void) -> Self {
        let mut process_id = 0;
        Self {
            window: window as usize,
            class_name: class_name(window),
            parent: unsafe { GetAncestor(window, GA_PARENT) } as usize,
            owner: unsafe { GetWindow(window, GW_OWNER) } as usize,
            root: unsafe { GetAncestor(window, GA_ROOT) } as usize,
            root_owner: unsafe { GetAncestor(window, GA_ROOTOWNER) } as usize,
            previous: unsafe { GetWindow(window, GW_HWNDPREV) } as usize,
            next: unsafe { GetWindow(window, GW_HWNDNEXT) } as usize,
            style: unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as usize,
            ex_style: unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize,
            visible: unsafe { IsWindowVisible(window) } != 0,
            enabled: unsafe { IsWindowEnabled(window) } != 0,
            thread_id: unsafe { GetWindowThreadProcessId(window, &mut process_id) },
            process_id,
            dpi: unsafe { GetDpiForWindow(window) },
        }
    }

    pub(super) fn log(&self, context: DiagnosticContext<'_>, entry: WindowLogEntry) {
        tracing::info!(
            stage = "composition_window",
            generation = context.overlay.generation,
            reason = context.reason,
            kind = entry.kind,
            index = entry.index,
            hwnd = self.window,
            class = %self.class_name,
            parent_hwnd = self.parent,
            owner_hwnd = self.owner,
            root_hwnd = self.root,
            root_owner_hwnd = self.root_owner,
            previous_hwnd = self.previous,
            next_hwnd = self.next,
            style = self.style,
            ex_style = self.ex_style,
            visible = self.visible,
            enabled = self.enabled,
            thread_id = self.thread_id,
            process_id = self.process_id,
            dpi = self.dpi,
            "captured Windows native RDP composition window"
        );
    }
}

pub(super) fn class_name(window: *mut c_void) -> String {
    if window.is_null() {
        return "<null>".to_owned();
    }
    let mut buffer = [0_u16; CLASS_NAME_CAPACITY];
    let length = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 0 {
        return "<unknown>".to_owned();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}
