use std::ffi::c_void;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use gpui::{AnyWindowHandle, Context};
use tokio::sync::mpsc;

use super::windows_native::WindowsNativeAdapter;
use super::RemoteDesktopView;

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: usize = 0x0100;
const VK_ESCAPE: u32 = 0x1B;
const HC_ACTION: i32 = 0;

type HHOOK = *mut c_void;
type HookProc = unsafe extern "system" fn(code: i32, wparam: usize, lparam: isize) -> isize;

#[repr(C)]
#[derive(Clone, Copy)]
struct KbdLlHookStruct {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    dw_extra_info: usize,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        lpfn: HookProc,
        h_mod: *mut c_void,
        dw_thread_id: u32,
    ) -> HHOOK;
    fn CallNextHookEx(hhk: HHOOK, code: i32, wparam: usize, lparam: isize) -> isize;
    fn UnhookWindowsHookEx(hhk: HHOOK) -> i32;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetModuleHandleW(module_name: *const u16) -> *mut c_void;
}

struct EscapeHookState {
    hook: usize,
    target_hwnd: usize,
    fullscreen: bool,
    sender: mpsc::UnboundedSender<()>,
}

static ESCAPE_HOOK_STATE: OnceLock<Mutex<Option<EscapeHookState>>> = OnceLock::new();

/// The RDP ActiveX child owns the Win32 keyboard focus inside the fullscreen
/// popup, so GPUI never sees the Escape key and the popup's own keybinding
/// cannot exit fullscreen. A low-level keyboard hook observes every key while
/// the hook is installed and forwards Escape to the GPUI window when the RDP
/// window is foreground and still fullscreen. Other keys always pass through.
pub(crate) fn install_fullscreen_escape(
    window_handle: AnyWindowHandle,
    cx: &mut Context<RemoteDesktopView>,
) {
    cx.spawn(async move |_, cx| {
        let target_hwnd = match window_handle.update(cx, |_, window, _| {
            WindowsNativeAdapter::parent_window_owner(window)
        }) {
            Ok(Ok(hwnd)) => hwnd,
            _ => return,
        };
        let (sender, mut receiver) = mpsc::unbounded_channel();
        if !install_escape_hook(target_hwnd, sender) {
            return;
        }
        while let Some(()) = receiver.recv().await {
            let window_alive = window_handle
                .update(cx, |_, window, _| {
                    if window.is_fullscreen() {
                        window.toggle_fullscreen();
                    }
                })
                .is_ok();
            if !window_alive {
                break;
            }
            // Escape exits fullscreen once; further Escape presses pass through
            // to the RDP session until the window is fullscreen again.
            set_fullscreen_state(false);
        }
        uninstall_escape_hook();
    })
    .detach();
}

fn install_escape_hook(
    target_hwnd: usize,
    sender: mpsc::UnboundedSender<()>,
) -> bool {
    let mut slot = ESCAPE_HOOK_STATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("Windows native RDP escape hook state");
    if slot.is_some() {
        return false;
    }
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, escape_key_hook, module, 0) };
    if hook.is_null() {
        return false;
    }
    *slot = Some(EscapeHookState {
        hook: hook as usize,
        target_hwnd,
        fullscreen: true,
        sender,
    });
    true
}

fn uninstall_escape_hook() {
    if let Some(slot) = ESCAPE_HOOK_STATE.get()
        && let Some(active) = slot
            .lock()
            .expect("Windows native RDP escape hook state")
            .take()
    {
        unsafe {
            UnhookWindowsHookEx(active.hook as HHOOK);
        }
    }
}

fn set_fullscreen_state(fullscreen: bool) {
    if let Some(slot) = ESCAPE_HOOK_STATE.get()
        && let Some(active) = slot
            .lock()
            .expect("Windows native RDP escape hook state")
            .as_mut()
    {
        active.fullscreen = fullscreen;
    }
}

unsafe extern "system" fn escape_key_hook(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if code == HC_ACTION && wparam == WM_KEYDOWN {
        let key = unsafe { &*(lparam as *const KbdLlHookStruct) };
        if key.vk_code == VK_ESCAPE {
            let foreground = unsafe { GetForegroundWindow() } as usize;
            if let Some(slot) = ESCAPE_HOOK_STATE.get()
                && let Ok(active) = slot.lock()
                && let Some(active) = active.as_ref()
                && active.fullscreen
                && active.target_hwnd == foreground
            {
                let _ = active.sender.send(());
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}
