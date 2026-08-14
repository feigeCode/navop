use std::ffi::c_void;

use super::ffi::*;
use super::window::WS_CLIPCHILDREN;
use super::{WindowsNativeOverlay, WindowsNativeOverlayBounds, window_pointer};
use snapshot::{WindowLogEntry, WindowSnapshot, class_name};

mod snapshot;

const GW_HWNDFIRST: u32 = 0;
const GW_HWNDLAST: u32 = 1;
const GW_HWNDNEXT: u32 = 2;
const GW_CHILD: u32 = 5;
const CWP_SKIPINVISIBLE: u32 = 0x0001;
const CWP_SKIPDISABLED: u32 = 0x0002;
const CWP_SKIPTRANSPARENT: u32 = 0x0004;
const HIT_TEST_FLAGS: u32 = CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT;
const MAX_DIRECT_CHILDREN: usize = 128;
const MAX_HIT_TEST_DEPTH: usize = 32;
const MAX_ANCESTRY_DEPTH: usize = 64;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const RDP_OUTPUT_CLASS: &str = "OPWindowClass";

#[derive(Clone, Copy)]
pub(super) struct DiagnosticContext<'a> {
    overlay: &'a WindowsNativeOverlay,
    reason: &'static str,
}

pub(super) fn log_created(overlay: &WindowsNativeOverlay) {
    let (owner_style, owner_ex_style) = window_styles(overlay.owner);
    let (style, ex_style) = window_styles(overlay.window);
    let parent = unsafe { GetParent(window_pointer(overlay.window)) } as usize;
    tracing::info!(
        stage = "overlay_created",
        generation = overlay.generation,
        owner_hwnd = overlay.owner,
        overlay_hwnd = overlay.window,
        parent_hwnd = parent,
        owner_style,
        owner_ex_style,
        owner_clip_children = owner_style & WS_CLIPCHILDREN as usize != 0,
        style,
        ex_style,
        owner_thread = overlay.owner_thread,
        dpi = unsafe { GetDpiForWindow(window_pointer(overlay.owner)) },
        "created Windows native RDP child overlay"
    );
}

pub(super) fn log_bounds(
    overlay: &WindowsNativeOverlay,
    requested: WindowsNativeOverlayBounds,
    clipped: Option<WindowsNativeOverlayBounds>,
) {
    let observed = observed_client_bounds(overlay);
    let window = window_pointer(overlay.window);
    let first = unsafe { GetWindow(window, GW_HWNDFIRST) } as usize;
    let last = unsafe { GetWindow(window, GW_HWNDLAST) } as usize;
    tracing::info!(
        stage = "overlay_bounds",
        generation = overlay.generation,
        owner_hwnd = overlay.owner,
        overlay_hwnd = overlay.window,
        requested = ?requested,
        clipped = ?clipped,
        observed = ?observed,
        visible = unsafe { IsWindowVisible(window) } != 0,
        first_sibling = first,
        last_sibling = last,
        overlay_is_first = first == overlay.window,
        requested_visible = overlay.requested_visible,
        "updated Windows native RDP overlay bounds"
    );
}

pub(super) fn log_visibility(overlay: &WindowsNativeOverlay, stage: &'static str) {
    tracing::info!(
        stage,
        generation = overlay.generation,
        owner_hwnd = overlay.owner,
        overlay_hwnd = overlay.window,
        requested_visible = overlay.requested_visible,
        actual_visible = unsafe { IsWindowVisible(window_pointer(overlay.window)) } != 0,
        "updated Windows native RDP overlay visibility"
    );
}

pub(super) fn log_composition_diagnostics(overlay: &WindowsNativeOverlay, reason: &'static str) {
    let context = DiagnosticContext { overlay, reason };
    let owner = window_pointer(overlay.owner);
    let window = window_pointer(overlay.window);
    if unsafe { IsWindow(owner) } == 0 || unsafe { IsWindow(window) } == 0 {
        log_skipped(context, owner, window);
        return;
    }

    log_direct_children(context, owner);
    log_center_hit_test(context, owner, window);
}

fn log_skipped(context: DiagnosticContext<'_>, owner: *mut c_void, window: *mut c_void) {
    tracing::warn!(
        stage = "composition_skipped",
        generation = context.overlay.generation,
        owner_hwnd = context.overlay.owner,
        overlay_hwnd = context.overlay.window,
        reason = context.reason,
        owner_live = unsafe { IsWindow(owner) } != 0,
        overlay_live = unsafe { IsWindow(window) } != 0,
        "skipped Windows native RDP composition diagnostics"
    );
}

fn log_direct_children(context: DiagnosticContext<'_>, owner: *mut c_void) {
    let mut child = unsafe { GetWindow(owner, GW_CHILD) };
    let mut index = 0;
    while !child.is_null() && index < MAX_DIRECT_CHILDREN {
        let entry = WindowLogEntry::new("owner_child", index);
        WindowSnapshot::capture(child).log(context, entry);
        child = unsafe { GetWindow(child, GW_HWNDNEXT) };
        index += 1;
    }
    tracing::info!(
        stage = "composition_children",
        generation = context.overlay.generation,
        owner_hwnd = context.overlay.owner,
        overlay_hwnd = context.overlay.window,
        reason = context.reason,
        direct_child_count = index,
        truncated = !child.is_null(),
        "enumerated Windows native RDP owner children"
    );
}

fn log_center_hit_test(context: DiagnosticContext<'_>, owner: *mut c_void, window: *mut c_void) {
    let Some(center) = window_center(window) else {
        tracing::warn!(
            stage = "composition_hit_test",
            generation = context.overlay.generation,
            reason = context.reason,
            "could not read overlay bounds for composition hit test"
        );
        return;
    };
    let global = unsafe { WindowFromPoint(center) };
    let path = child_hit_path(owner, center);
    for (index, child) in path.iter().copied().enumerate() {
        let entry = WindowLogEntry::new("hit_path", index);
        WindowSnapshot::capture(child).log(context, entry);
    }
    log_hit_verdict(
        context,
        HitTestResult {
            center,
            global,
            overlay: window,
            path: &path,
        },
    );
}

struct HitTestResult<'a> {
    center: Point,
    global: *mut c_void,
    overlay: *mut c_void,
    path: &'a [*mut c_void],
}

fn log_hit_verdict(context: DiagnosticContext<'_>, result: HitTestResult<'_>) {
    let deepest = result.path.last().copied().unwrap_or(result.global);
    let deepest_class = class_name(deepest);
    let global_class = class_name(result.global);
    let global_in_overlay = is_descendant_or_self(result.overlay, result.global);
    let verdict = composition_verdict(global_in_overlay, &global_class, &deepest_class);
    tracing::info!(
        stage = "composition_verdict",
        generation = context.overlay.generation,
        owner_hwnd = context.overlay.owner,
        overlay_hwnd = context.overlay.window,
        reason = context.reason,
        center_x = result.center.x,
        center_y = result.center.y,
        global_hwnd = result.global as usize,
        global_class = %global_class,
        deepest_hwnd = deepest as usize,
        deepest_class = %deepest_class,
        global_in_overlay,
        verdict,
        "completed Windows native RDP composition hit test"
    );
}

fn composition_verdict(
    global_in_overlay: bool,
    global_class: &str,
    deepest_class: &str,
) -> &'static str {
    if global_in_overlay && global_class == RDP_OUTPUT_CLASS {
        "global_hit_reaches_rdp_output"
    } else if global_in_overlay {
        "global_hit_reaches_overlay_subtree"
    } else if deepest_class == RDP_OUTPUT_CLASS {
        "owner_path_reaches_rdp_but_global_hit_differs"
    } else {
        "z_order_or_clipping_coverage_remains_possible"
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

fn observed_client_bounds(overlay: &WindowsNativeOverlay) -> Option<WindowsNativeOverlayBounds> {
    let mut rect = Rect::default();
    let window = window_pointer(overlay.window);
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        return None;
    }
    let mut origin = Point {
        x: rect.left,
        y: rect.top,
    };
    if unsafe { ScreenToClient(window_pointer(overlay.owner), &mut origin) } == 0 {
        return None;
    }
    Some(WindowsNativeOverlayBounds {
        x: origin.x,
        y: origin.y,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
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

fn window_styles(window: usize) -> (usize, usize) {
    let window = window_pointer(window);
    (
        unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as usize,
        unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize,
    )
}
