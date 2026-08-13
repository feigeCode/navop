use std::ffi::c_void;

use crate::native_overlay_ffi::*;

use super::window::{ChildBounds, WS_CLIPCHILDREN, last_error};
use super::{NativeOverlay, window_pointer};

const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const GW_HWNDFIRST: u32 = 0;
const GW_HWNDLAST: u32 = 1;
const GW_HWNDNEXT: u32 = 2;
const GW_HWNDPREV: u32 = 3;
const GW_OWNER: u32 = 4;
const GW_CHILD: u32 = 5;
const GA_PARENT: u32 = 1;
const GA_ROOT: u32 = 2;
const GA_ROOTOWNER: u32 = 3;
const CWP_SKIPINVISIBLE: u32 = 0x0001;
const CWP_SKIPDISABLED: u32 = 0x0002;
const CWP_SKIPTRANSPARENT: u32 = 0x0004;
const HIT_TEST_FLAGS: u32 = CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT;
const MAX_DIRECT_CHILDREN: usize = 256;
const MAX_HIT_TEST_DEPTH: usize = 32;
const MAX_ANCESTRY_DEPTH: usize = 64;
const CLASS_NAME_CAPACITY: usize = 256;
const RDP_OUTPUT_CLASS: &str = "OPWindowClass";

pub(super) fn log_overlay_state(
    overlay: &NativeOverlay,
    expected: ChildBounds,
    forced: bool,
) -> Result<(), String> {
    let window = window_pointer(overlay.window);
    let owner = window_pointer(overlay.owner);
    let mut observed = Rect::default();
    if unsafe { GetWindowRect(window, &mut observed) } == 0 {
        return Err(last_error("GetWindowRect(child RDP overlay)"));
    }
    let mut origin = Point {
        x: observed.left,
        y: observed.top,
    };
    if unsafe { ScreenToClient(owner, &mut origin) } == 0 {
        return Err(last_error("ScreenToClient(GPUI owner)"));
    }
    let owner_style = unsafe { GetWindowLongPtrW(owner, GWL_STYLE) } as usize;
    let first = unsafe { GetWindow(window, GW_HWNDFIRST) } as usize;
    let last = unsafe { GetWindow(window, GW_HWNDLAST) } as usize;
    println!(
        "presentation: overlay_state expected={{x={},y={},width={},height={}}} observed={{x={},y={},width={},height={}}} visible={} forced={} owner_style=0x{owner_style:016X} owner_clip_children={} first_sibling=0x{first:016X} last_sibling=0x{last:016X} overlay_is_first={}",
        expected.x,
        expected.y,
        expected.width,
        expected.height,
        origin.x,
        origin.y,
        observed.right - observed.left,
        observed.bottom - observed.top,
        unsafe { IsWindowVisible(window) } != 0,
        forced,
        owner_style & WS_CLIPCHILDREN as usize != 0,
        first == overlay.window,
    );
    Ok(())
}

pub(super) fn log_composition_diagnostics(overlay: &NativeOverlay, reason: &str) {
    let owner = window_pointer(overlay.owner);
    let overlay_window = window_pointer(overlay.window);
    if unsafe { IsWindow(owner) } == 0 || unsafe { IsWindow(overlay_window) } == 0 {
        println!(
            "composition: reason={reason} skipped owner_live={} overlay_live={}",
            unsafe { IsWindow(owner) } != 0,
            unsafe { IsWindow(overlay_window) } != 0
        );
        return;
    }
    println!(
        "composition: reason={reason} owner=0x{:016X} overlay=0x{:016X}",
        overlay.owner, overlay.window
    );
    log_direct_children(owner);
    log_center_hit_test(owner, overlay_window);
}

fn log_direct_children(owner: *mut c_void) {
    let mut child = unsafe { GetWindow(owner, GW_CHILD) };
    let mut index = 0;
    while !child.is_null() && index < MAX_DIRECT_CHILDREN {
        WindowSnapshot::capture(child).log("owner_child", index);
        child = unsafe { GetWindow(child, GW_HWNDNEXT) };
        index += 1;
    }
    println!(
        "composition: owner_direct_child_count={index} truncated={}",
        !child.is_null()
    );
}

fn log_center_hit_test(owner: *mut c_void, overlay: *mut c_void) {
    let Some(center) = window_center(overlay) else {
        println!("composition: center_hit_test unavailable");
        return;
    };
    let global = unsafe { WindowFromPoint(center) };
    println!(
        "composition: center_screen={{x={},y={}}} window_from_point=0x{:016X} class=\"{}\"",
        center.x,
        center.y,
        global as usize,
        class_name(global)
    );
    let path = child_hit_path(owner, center);
    for (index, window) in path.iter().copied().enumerate() {
        WindowSnapshot::capture(window).log("hit_path", index);
    }
    let deepest = path.last().copied().unwrap_or(global);
    let deepest_class = class_name(deepest);
    let global_class = class_name(global);
    let global_in_overlay = is_descendant_or_self(overlay, global);
    println!(
        "composition: center_hit_summary global_in_overlay={} owner_path_reaches_rdp={}",
        global_in_overlay,
        deepest_class == RDP_OUTPUT_CLASS
    );
    if global_in_overlay && global_class == RDP_OUTPUT_CLASS {
        println!(
            "composition: verdict=\"global hit-test reaches native RDP output; framebuffer still not evidenced\" global=0x{:016X} class=\"{}\"",
            global as usize, global_class
        );
    } else if global_in_overlay {
        println!(
            "composition: verdict=\"global hit-test reaches native overlay subtree; inspect RDP child path and framebuffer\" global=0x{:016X} class=\"{}\"",
            global as usize, global_class
        );
    } else if deepest_class == RDP_OUTPUT_CLASS {
        println!(
            "composition: verdict=\"owner-local path reaches RDP output but global hit-test differs; external or sibling coverage likely\" global=0x{:016X} class=\"{}\"",
            global as usize, global_class
        );
    } else {
        println!(
            "composition: verdict=\"compositor / Z-order / clipping coverage remains possible\" deepest=0x{:016X} class=\"{}\"",
            deepest as usize, deepest_class
        );
    }
}

fn child_hit_path(owner: *mut c_void, center: Point) -> Vec<*mut c_void> {
    let mut path = Vec::with_capacity(MAX_HIT_TEST_DEPTH);
    let mut parent = owner;
    path.push(parent);
    for _ in 0..MAX_HIT_TEST_DEPTH {
        let mut client_point = center;
        if unsafe { ScreenToClient(parent, &mut client_point) } == 0 {
            break;
        }
        let child = unsafe { ChildWindowFromPointEx(parent, client_point, HIT_TEST_FLAGS) };
        if child.is_null() || child == parent {
            break;
        }
        path.push(child);
        parent = child;
    }
    path
}

fn is_descendant_or_self(ancestor: *mut c_void, window: *mut c_void) -> bool {
    let mut current = window;
    for _ in 0..MAX_ANCESTRY_DEPTH {
        if current.is_null() {
            return false;
        }
        if current == ancestor {
            return true;
        }
        current = unsafe { GetParent(current) };
    }
    false
}

fn window_center(window: *mut c_void) -> Option<Point> {
    let mut rect = Rect::default();
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        return None;
    }
    Some(Point {
        x: rect.left.saturating_add((rect.right - rect.left) / 2),
        y: rect.top.saturating_add((rect.bottom - rect.top) / 2),
    })
}

struct WindowSnapshot {
    window: usize,
    class_name: String,
    parent: usize,
    owner: usize,
    ancestor_parent: usize,
    root: usize,
    root_owner: usize,
    previous: usize,
    next: usize,
    style: usize,
    ex_style: usize,
    visible: bool,
    enabled: bool,
    window_rect: Rect,
    client_rect: Rect,
    thread_id: u32,
    process_id: u32,
    dpi: u32,
}

impl WindowSnapshot {
    fn capture(window: *mut c_void) -> Self {
        let mut process_id = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        Self {
            window: window as usize,
            class_name: class_name(window),
            parent: unsafe { GetParent(window) } as usize,
            owner: unsafe { GetWindow(window, GW_OWNER) } as usize,
            ancestor_parent: unsafe { GetAncestor(window, GA_PARENT) } as usize,
            root: unsafe { GetAncestor(window, GA_ROOT) } as usize,
            root_owner: unsafe { GetAncestor(window, GA_ROOTOWNER) } as usize,
            previous: unsafe { GetWindow(window, GW_HWNDPREV) } as usize,
            next: unsafe { GetWindow(window, GW_HWNDNEXT) } as usize,
            style: unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as usize,
            ex_style: unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize,
            visible: unsafe { IsWindowVisible(window) } != 0,
            enabled: unsafe { IsWindowEnabled(window) } != 0,
            window_rect: read_rect(window, GetWindowRect),
            client_rect: read_rect(window, GetClientRect),
            thread_id,
            process_id,
            dpi: unsafe { GetDpiForWindow(window) },
        }
    }

    fn log(&self, kind: &str, index: usize) {
        println!(
            "composition: {kind} index={index} hwnd=0x{:016X} class=\"{}\" parent=0x{:016X} owner=0x{:016X} ancestor_parent=0x{:016X} root=0x{:016X} root_owner=0x{:016X} previous=0x{:016X} next=0x{:016X} style=0x{:016X} ex_style=0x{:016X} visible={} enabled={} window_rect={} client_rect={} thread_id={} process_id={} dpi={}",
            self.window,
            self.class_name,
            self.parent,
            self.owner,
            self.ancestor_parent,
            self.root,
            self.root_owner,
            self.previous,
            self.next,
            self.style,
            self.ex_style,
            self.visible,
            self.enabled,
            format_rect(self.window_rect),
            format_rect(self.client_rect),
            self.thread_id,
            self.process_id,
            self.dpi,
        );
    }
}

fn class_name(window: *mut c_void) -> String {
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

fn read_rect(
    window: *mut c_void,
    reader: unsafe extern "system" fn(*mut c_void, *mut Rect) -> i32,
) -> Rect {
    let mut rect = Rect::default();
    if unsafe { reader(window, &mut rect) } == 0 {
        return Rect::default();
    }
    rect
}

fn format_rect(rect: Rect) -> String {
    format!(
        "{{left={},top={},right={},bottom={}}}",
        rect.left, rect.top, rect.right, rect.bottom
    )
}
