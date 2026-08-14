use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Point {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub(super) fn GetCurrentThreadId() -> u32;
    pub(super) fn GetLastError() -> u32;
    pub(super) fn GetModuleHandleW(module_name: *const u16) -> *mut c_void;
    pub(super) fn SetLastError(error_code: u32);
}

#[link(name = "user32")]
unsafe extern "system" {
    pub(super) fn ChildWindowFromPointEx(
        parent: *mut c_void,
        point: Point,
        flags: u32,
    ) -> *mut c_void;
    pub(super) fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        parameter: *mut c_void,
    ) -> *mut c_void;
    pub(super) fn DestroyWindow(window: *mut c_void) -> i32;
    pub(super) fn GetAncestor(window: *mut c_void, flags: u32) -> *mut c_void;
    pub(super) fn GetClassNameW(window: *mut c_void, class_name: *mut u16, max_count: i32) -> i32;
    pub(super) fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
    pub(super) fn GetDpiForWindow(window: *mut c_void) -> u32;
    pub(super) fn GetParent(window: *mut c_void) -> *mut c_void;
    pub(super) fn GetWindow(window: *mut c_void, command: u32) -> *mut c_void;
    pub(super) fn GetWindowLongPtrW(window: *mut c_void, index: i32) -> isize;
    pub(super) fn GetWindowRect(window: *mut c_void, rect: *mut Rect) -> i32;
    pub(super) fn GetWindowThreadProcessId(window: *mut c_void, process_id: *mut u32) -> u32;
    pub(super) fn IsIconic(window: *mut c_void) -> i32;
    pub(super) fn IsWindow(window: *mut c_void) -> i32;
    pub(super) fn IsWindowEnabled(window: *mut c_void) -> i32;
    pub(super) fn IsWindowVisible(window: *mut c_void) -> i32;
    pub(super) fn ScreenToClient(window: *mut c_void, point: *mut Point) -> i32;
    pub(super) fn SetWindowLongPtrW(window: *mut c_void, index: i32, value: isize) -> isize;
    pub(super) fn SetWindowPos(
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    pub(super) fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    pub(super) fn WindowFromPoint(point: Point) -> *mut c_void;
}
