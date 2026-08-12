use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{align_of, size_of};

pub(crate) const ABI_VERSION: u32 = 1;
pub(crate) const CREATE_WITH_PARENT_ABI_VERSION: u32 = 1;

pub(crate) type NativeResult = i32;

pub(crate) const RESULT_OK: NativeResult = 0;
pub(crate) const RESULT_INVALID_ARGUMENT: NativeResult = 1;
pub(crate) const RESULT_ABI_MISMATCH: NativeResult = 2;
pub(crate) const RESULT_ALLOCATION_FAILED: NativeResult = 3;
pub(crate) const RESULT_INTERNAL_ERROR: NativeResult = 4;
pub(crate) const RESULT_UNAVAILABLE: NativeResult = 5;
pub(crate) const RESULT_WRONG_THREAD: NativeResult = 6;
pub(crate) const RESULT_CALLBACK_IN_FLIGHT: NativeResult = 7;
pub(crate) const RESULT_INVALID_STATE: NativeResult = 8;
#[allow(dead_code)]
pub(crate) const LAST_ERROR_LEGACY_SIZE: u32 = 24;

#[allow(dead_code)]
pub(crate) const CREATE_STAGE_NONE: u32 = 0;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_OLE_INITIALIZE: u32 = 1;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_ATL_AX_WIN_INIT: u32 = 2;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_CREATE_WINDOW: u32 = 3;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_CREATE_CONTROL: u32 = 4;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_QUERY_CLIENT: u32 = 5;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_QUERY_NON_SCRIPTABLE: u32 = 6;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_SET_PARENT: u32 = 7;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_EVENT_SUBSCRIPTION: u32 = 8;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_EXCEPTION: u32 = 9;

pub(crate) const CONNECTION_STATE_DISCONNECTED: u32 = 0;
pub(crate) const CONNECTION_STATE_CONNECTED: u32 = 1;
pub(crate) const CONNECTION_STATE_CONNECTING: u32 = 2;
pub(crate) const REQUEST_CLOSE_CAN_PROCEED: u32 = 0;
pub(crate) const REQUEST_CLOSE_WAIT_FOR_EVENTS: u32 = 1;

pub(crate) const EVENT_CONNECTING: u32 = 1;
pub(crate) const EVENT_CONNECTED: u32 = 2;
pub(crate) const EVENT_LOGIN_COMPLETE: u32 = 3;
pub(crate) const EVENT_RECONNECTING: u32 = 4;
pub(crate) const EVENT_RECONNECTED: u32 = 5;
pub(crate) const EVENT_NETWORK_STATUS_CHANGED: u32 = 6;
pub(crate) const EVENT_REMOTE_DESKTOP_SIZE_CHANGED: u32 = 7;
pub(crate) const EVENT_ENTER_FULLSCREEN: u32 = 8;
pub(crate) const EVENT_LEAVE_FULLSCREEN: u32 = 9;
pub(crate) const EVENT_AUTHENTICATION_WARNING_DISPLAYED: u32 = 10;
pub(crate) const EVENT_AUTHENTICATION_WARNING_DISMISSED: u32 = 11;
pub(crate) const EVENT_WARNING: u32 = 12;
pub(crate) const EVENT_FATAL_ERROR: u32 = 13;
pub(crate) const EVENT_LOGON_ERROR: u32 = 14;
pub(crate) const EVENT_DISCONNECTED: u32 = 15;
pub(crate) const EVENT_CLOSE_CONFIRMED: u32 = 16;
pub(crate) const EVENT_FOCUS_RELEASED: u32 = 17;
pub(crate) const MAX_EVENT_PAYLOAD_BYTES: u32 = 65_536;

#[repr(C)]
pub(crate) struct NativeRdpHost {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, std::marker::PhantomPinned)>,
}

#[repr(C)]
pub(crate) struct NavopRdpProbeOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
}

impl NavopRdpProbeOptions {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpProbeResult {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) available: u32,
    pub(crate) reserved: u32,
}

impl NavopRdpProbeResult {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            available: 0,
            reserved: 0,
        }
    }

    pub(crate) fn has_current_layout(&self) -> bool {
        self.struct_size == size_of::<Self>() as u32
            && self.abi_version == ABI_VERSION
            && self.reserved == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavopRdpLastError {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) result: NativeResult,
    pub(crate) hresult: i32,
    pub(crate) has_hresult: u32,
    pub(crate) reserved: u32,
    pub(crate) stage: u32,
    pub(crate) win32_code: u32,
    pub(crate) has_win32_code: u32,
}

impl NavopRdpLastError {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            result: RESULT_OK,
            hresult: 0,
            has_hresult: 0,
            reserved: 0,
            stage: CREATE_STAGE_NONE,
            win32_code: 0,
            has_win32_code: 0,
        }
    }

    pub(crate) fn has_current_layout(&self) -> bool {
        self.struct_size >= size_of::<Self>() as u32
            && self.abi_version == ABI_VERSION
            && self.has_hresult <= 1
            && self.has_win32_code <= 1
            && self.reserved == 0
    }
}

#[repr(C)]
pub(crate) struct NavopRdpCreateOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
}

impl NavopRdpCreateOptions {
    pub(crate) fn current(generation: u64) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpCreateWithParentOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
    pub(crate) parent_hwnd: usize,
}

impl NavopRdpCreateWithParentOptions {
    pub(crate) fn current(generation: u64, parent_hwnd: usize) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: CREATE_WITH_PARENT_ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
            parent_hwnd,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavopRdpBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl NavopRdpBounds {
    pub(crate) const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpEvent {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) kind: u32,
    pub(crate) reserved: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
    pub(crate) code: i32,
    pub(crate) payload_len: u32,
}

impl NavopRdpEvent {
    #[cfg(test)]
    pub(crate) fn current(generation: u64, kind: u32, code: i32, payload_len: u32) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            kind,
            reserved: 0,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
            code,
            payload_len,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpEventCallbackOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NavopRdpBorrowedSecret {
    pub(crate) data: *const u16,
    pub(crate) len: u32,
}

#[repr(C)]
pub(crate) struct NavopRdpCredentialBundle {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) server_password: NavopRdpBorrowedSecret,
    pub(crate) gateway_password: NavopRdpBorrowedSecret,
    pub(crate) flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NavopRdpBorrowedUtf16 {
    pub(crate) data: *const u16,
    pub(crate) len: u32,
}

#[repr(C)]
pub(crate) struct NavopRdpConnectionOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) host: NavopRdpBorrowedUtf16,
    pub(crate) port: u32,
    pub(crate) desktop_width: i32,
    pub(crate) desktop_height: i32,
    pub(crate) color_depth: i32,
    pub(crate) flags: u32,
}

impl NavopRdpConnectionOptions {
    pub(crate) fn current(
        host: NavopRdpBorrowedUtf16,
        port: u32,
        desktop_width: i32,
        desktop_height: i32,
        color_depth: i32,
    ) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            host,
            port,
            desktop_width,
            desktop_height,
            color_depth,
            flags: 0,
        }
    }
}

const _: () = {
    assert!(size_of::<NativeResult>() == 4);

    assert!(size_of::<NavopRdpProbeOptions>() == 8);
    assert!(align_of::<NavopRdpProbeOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpProbeOptions, abi_version) == 4);

    assert!(size_of::<NavopRdpProbeResult>() == 16);
    assert!(align_of::<NavopRdpProbeResult>() == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, available) == 8);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, reserved) == 12);

    assert!(size_of::<NavopRdpLastError>() == 36);
    assert!(align_of::<NavopRdpLastError>() == 4);
    assert!(std::mem::offset_of!(NavopRdpLastError, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpLastError, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpLastError, result) == 8);
    assert!(std::mem::offset_of!(NavopRdpLastError, hresult) == 12);
    assert!(std::mem::offset_of!(NavopRdpLastError, has_hresult) == 16);
    assert!(std::mem::offset_of!(NavopRdpLastError, reserved) == 20);
    assert!(std::mem::offset_of!(NavopRdpLastError, stage) == 24);
    assert!(std::mem::offset_of!(NavopRdpLastError, win32_code) == 28);
    assert!(std::mem::offset_of!(NavopRdpLastError, has_win32_code) == 32);

    assert!(size_of::<NavopRdpCreateOptions>() == 16);
    assert!(align_of::<NavopRdpCreateOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_high) == 12);

    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, generation_high) == 12);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, parent_hwnd) == 16);

    assert!(size_of::<NavopRdpBounds>() == 16);
    assert!(align_of::<NavopRdpBounds>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBounds, x) == 0);
    assert!(std::mem::offset_of!(NavopRdpBounds, y) == 4);
    assert!(std::mem::offset_of!(NavopRdpBounds, width) == 8);
    assert!(std::mem::offset_of!(NavopRdpBounds, height) == 12);

    assert!(size_of::<NavopRdpEvent>() == 32);
    assert!(align_of::<NavopRdpEvent>() == 4);
    assert!(std::mem::offset_of!(NavopRdpEvent, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpEvent, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpEvent, kind) == 8);
    assert!(std::mem::offset_of!(NavopRdpEvent, reserved) == 12);
    assert!(std::mem::offset_of!(NavopRdpEvent, generation_low) == 16);
    assert!(std::mem::offset_of!(NavopRdpEvent, generation_high) == 20);
    assert!(std::mem::offset_of!(NavopRdpEvent, code) == 24);
    assert!(std::mem::offset_of!(NavopRdpEvent, payload_len) == 28);

    assert!(size_of::<NavopRdpEventCallbackOptions>() == 16);
    assert!(align_of::<NavopRdpEventCallbackOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_high) == 12);

    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, data) == 0);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, server_password) == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, data) == 0);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, host) == 8);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NavopRdpCreateWithParentOptions>() == 24);
    assert!(align_of::<NavopRdpCreateWithParentOptions>() == 8);
    assert!(size_of::<NavopRdpBorrowedSecret>() == 16);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 8);
    assert!(size_of::<NavopRdpCredentialBundle>() == 48);
    assert!(align_of::<NavopRdpCredentialBundle>() == 8);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 24);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 40);
    assert!(size_of::<NavopRdpBorrowedUtf16>() == 16);
    assert!(align_of::<NavopRdpBorrowedUtf16>() == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len) == 8);
    assert!(size_of::<NavopRdpConnectionOptions>() == 48);
    assert!(align_of::<NavopRdpConnectionOptions>() == 8);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, port) == 24);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width) == 28);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height) == 32);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, color_depth) == 36);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, flags) == 40);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NavopRdpCreateWithParentOptions>() == 20);
    assert!(align_of::<NavopRdpCreateWithParentOptions>() == 4);
    assert!(size_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 4);
    assert!(size_of::<NavopRdpCredentialBundle>() == 28);
    assert!(align_of::<NavopRdpCredentialBundle>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 16);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 24);
    assert!(size_of::<NavopRdpBorrowedUtf16>() == 8);
    assert!(align_of::<NavopRdpBorrowedUtf16>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len) == 4);
    assert!(size_of::<NavopRdpConnectionOptions>() == 36);
    assert!(align_of::<NavopRdpConnectionOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, port) == 16);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width) == 20);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height) == 24);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, color_depth) == 28);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, flags) == 32);
};

impl NavopRdpEventCallbackOptions {
    pub(crate) fn current(generation: u64) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
        }
    }
}

pub(crate) type NativeEventCallback =
    unsafe extern "C" fn(context: *mut c_void, event: *const NavopRdpEvent, payload: *const u8);

pub(crate) type ProbeFn = unsafe fn(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult;
pub(crate) type CreateFn = unsafe fn(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult;
pub(crate) type CreateWithParentV2Fn = unsafe fn(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult;
pub(crate) type GetLastErrorFn =
    unsafe fn(host: *mut NativeRdpHost, out_error: *mut NavopRdpLastError) -> NativeResult;
pub(crate) type SetBoundsFn =
    unsafe fn(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult;
pub(crate) type SetVisibleFn = unsafe fn(host: *mut NativeRdpHost, visible: u32) -> NativeResult;
pub(crate) type FocusFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;
pub(crate) type DestroyFn = unsafe fn(host: *mut *mut NativeRdpHost) -> NativeResult;
pub(crate) type RegisterEventCallbackFn = unsafe fn(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    callback_context: *mut c_void,
) -> NativeResult;
pub(crate) type UnregisterEventCallbackFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;
pub(crate) type ApplyCredentialsFn = unsafe fn(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult;
pub(crate) type ConnectFn =
    unsafe fn(host: *mut NativeRdpHost, options: *const NavopRdpConnectionOptions) -> NativeResult;
pub(crate) type GetConnectionStateFn =
    unsafe fn(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult;
pub(crate) type RequestCloseFn =
    unsafe fn(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult;
pub(crate) type DisconnectFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;

#[derive(Clone, Copy)]
pub(crate) struct NativeBindings {
    pub(crate) probe: ProbeFn,
    pub(crate) create: CreateFn,
    pub(crate) create_with_parent_v2: CreateWithParentV2Fn,
    pub(crate) get_last_error: GetLastErrorFn,
    pub(crate) set_bounds: SetBoundsFn,
    pub(crate) set_visible: SetVisibleFn,
    pub(crate) focus: FocusFn,
    pub(crate) destroy: DestroyFn,
    pub(crate) register_event_callback: RegisterEventCallbackFn,
    pub(crate) unregister_event_callback: UnregisterEventCallbackFn,
    pub(crate) apply_credentials: ApplyCredentialsFn,
    pub(crate) connect: ConnectFn,
    pub(crate) get_connection_state: GetConnectionStateFn,
    pub(crate) request_close: RequestCloseFn,
    pub(crate) disconnect: DisconnectFn,
}

pub(crate) const NATIVE_BINDINGS: NativeBindings = NativeBindings {
    probe,
    create,
    create_with_parent_v2,
    get_last_error,
    set_bounds,
    set_visible,
    focus,
    destroy,
    register_event_callback,
    unregister_event_callback,
    apply_credentials,
    connect,
    get_connection_state,
    request_close,
    disconnect,
};

#[cfg(windows_rdp_host_native)]
unsafe extern "C" {
    fn navop_rdp_probe(
        options: *const NavopRdpProbeOptions,
        out_result: *mut NavopRdpProbeResult,
    ) -> NativeResult;
    fn navop_rdp_create(
        options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult;
    fn navop_rdp_create_with_parent_v2(
        options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult;
    fn navop_rdp_get_last_error(
        host: *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult;
    fn navop_rdp_set_bounds(
        host: *mut NativeRdpHost,
        bounds: *const NavopRdpBounds,
    ) -> NativeResult;
    fn navop_rdp_set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult;
    fn navop_rdp_focus(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_register_event_callback(
        host: *mut NativeRdpHost,
        options: *const NavopRdpEventCallbackOptions,
        callback: Option<NativeEventCallback>,
        callback_context: *mut c_void,
    ) -> NativeResult;
    fn navop_rdp_unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_apply_credentials(
        host: *mut NativeRdpHost,
        credentials: *const NavopRdpCredentialBundle,
    ) -> NativeResult;
    fn navop_rdp_connect(
        host: *mut NativeRdpHost,
        options: *const NavopRdpConnectionOptions,
    ) -> NativeResult;
    fn navop_rdp_get_connection_state(
        host: *mut NativeRdpHost,
        out_state: *mut u32,
    ) -> NativeResult;
    fn navop_rdp_request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult;
    fn navop_rdp_disconnect(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_destroy(host: *mut *mut NativeRdpHost) -> NativeResult;
}

#[cfg(windows_rdp_host_native)]
unsafe fn probe(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult {
    unsafe { navop_rdp_probe(options, out_result) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn create(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult {
    unsafe { navop_rdp_create(options, out_host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn create_with_parent_v2(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    unsafe { navop_rdp_create_with_parent_v2(options, out_host, out_error) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn get_last_error(
    host: *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    unsafe { navop_rdp_get_last_error(host, out_error) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn set_bounds(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult {
    unsafe { navop_rdp_set_bounds(host, bounds) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult {
    unsafe { navop_rdp_set_visible(host, visible) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn focus(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_focus(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn register_event_callback(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    callback_context: *mut c_void,
) -> NativeResult {
    unsafe { navop_rdp_register_event_callback(host, options, callback, callback_context) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_unregister_event_callback(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn apply_credentials(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult {
    unsafe { navop_rdp_apply_credentials(host, credentials) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn connect(
    host: *mut NativeRdpHost,
    options: *const NavopRdpConnectionOptions,
) -> NativeResult {
    unsafe { navop_rdp_connect(host, options) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn get_connection_state(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult {
    unsafe { navop_rdp_get_connection_state(host, out_state) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult {
    unsafe { navop_rdp_request_close(host, out_status) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn disconnect(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_disconnect(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_destroy(host) }
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn probe(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult {
    if options.is_null() || out_result.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpProbeOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    let caller_result_size = unsafe { std::ptr::addr_of!((*out_result).struct_size).read() };
    if caller_result_size < size_of::<NavopRdpProbeResult>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let result_abi_version = unsafe { std::ptr::addr_of!((*out_result).abi_version).read() };
    if result_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    unsafe {
        *out_result = NavopRdpProbeResult {
            struct_size: caller_result_size,
            abi_version: ABI_VERSION,
            available: 0,
            reserved: 0,
        };
    }
    RESULT_OK
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn create(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult {
    if out_host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_host = std::ptr::null_mut();
    }
    if options.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpCreateOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn create_with_parent_v2(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    if out_error.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    let error_size = unsafe { std::ptr::addr_of!((*out_error).struct_size).read() };
    if error_size < LAST_ERROR_LEGACY_SIZE {
        return RESULT_INVALID_ARGUMENT;
    }
    let error_abi = unsafe { std::ptr::addr_of!((*out_error).abi_version).read() };
    if error_abi != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    unsafe {
        (*out_error).struct_size = error_size;
        (*out_error).abi_version = ABI_VERSION;
        (*out_error).result = RESULT_OK;
        (*out_error).hresult = 0;
        (*out_error).has_hresult = 0;
        (*out_error).reserved = 0;
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, stage) as u32 + size_of::<u32>() as u32
        {
            (*out_error).stage = CREATE_STAGE_NONE;
        }
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, win32_code) as u32 + size_of::<u32>() as u32
        {
            (*out_error).win32_code = 0;
        }
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, has_win32_code) as u32
                + size_of::<u32>() as u32
        {
            (*out_error).has_win32_code = 0;
        }
    }
    if out_host.is_null() {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_host = std::ptr::null_mut();
    }
    if options.is_null() {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpCreateWithParentOptions>() as u32 {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != CREATE_WITH_PARENT_ABI_VERSION {
        unsafe { (*out_error).result = RESULT_ABI_MISMATCH };
        return RESULT_ABI_MISMATCH;
    }
    if unsafe { std::ptr::addr_of!((*options).parent_hwnd).read() } == 0 {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }

    unsafe { (*out_error).result = RESULT_UNAVAILABLE };
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn get_last_error(
    host: *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    if host.is_null() || out_error.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn set_bounds(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult {
    if host.is_null() || bounds.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let width = unsafe { std::ptr::addr_of!((*bounds).width).read() };
    let height = unsafe { std::ptr::addr_of!((*bounds).height).read() };
    if width < 0 || height < 0 {
        return RESULT_INVALID_ARGUMENT;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult {
    if host.is_null() || visible > 1 {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn focus(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn register_event_callback(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    _callback_context: *mut c_void,
) -> NativeResult {
    if host.is_null() || options.is_null() || callback.is_none() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpEventCallbackOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_OK
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn apply_credentials(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult {
    if host.is_null() || credentials.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let struct_size = unsafe { std::ptr::addr_of!((*credentials).struct_size).read() };
    if struct_size < size_of::<NavopRdpCredentialBundle>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let abi_version = unsafe { std::ptr::addr_of!((*credentials).abi_version).read() };
    if abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    let flags = unsafe { std::ptr::addr_of!((*credentials).flags).read() };
    if flags != 0 {
        return RESULT_INVALID_ARGUMENT;
    }

    let server_password = unsafe { std::ptr::addr_of!((*credentials).server_password).read() };
    if server_password.len > 0 && server_password.data.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    let gateway_password = unsafe { std::ptr::addr_of!((*credentials).gateway_password).read() };
    if gateway_password.len > 0 && gateway_password.data.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn connect(
    host: *mut NativeRdpHost,
    options: *const NavopRdpConnectionOptions,
) -> NativeResult {
    if host.is_null() || options.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if struct_size < size_of::<NavopRdpConnectionOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    let flags = unsafe { std::ptr::addr_of!((*options).flags).read() };
    if flags != 0 {
        return RESULT_INVALID_ARGUMENT;
    }

    let host_name = unsafe { std::ptr::addr_of!((*options).host).read() };
    if host_name.len == 0
        || host_name.len as usize > crate::options::WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS
        || host_name.data.is_null()
    {
        return RESULT_INVALID_ARGUMENT;
    }
    let host_slice = unsafe { std::slice::from_raw_parts(host_name.data, host_name.len as usize) };
    if host_slice.contains(&0) {
        return RESULT_INVALID_ARGUMENT;
    }

    let port = unsafe { std::ptr::addr_of!((*options).port).read() };
    let desktop_width = unsafe { std::ptr::addr_of!((*options).desktop_width).read() };
    let desktop_height = unsafe { std::ptr::addr_of!((*options).desktop_height).read() };
    let color_depth = unsafe { std::ptr::addr_of!((*options).color_depth).read() };
    if !(1..=u32::from(u16::MAX)).contains(&port)
        || desktop_width <= 0
        || desktop_height <= 0
        || !matches!(color_depth, 8 | 15 | 16 | 24 | 32)
    {
        return RESULT_INVALID_ARGUMENT;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn get_connection_state(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult {
    if out_state.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_state = CONNECTION_STATE_DISCONNECTED;
    }
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult {
    if out_status.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_status = REQUEST_CLOSE_CAN_PROCEED;
    }
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn disconnect(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *host = std::ptr::null_mut();
    }
    RESULT_OK
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::*;

    #[test]
    fn abi_constants_match_the_native_header() {
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(RESULT_OK, 0);
        assert_eq!(RESULT_INVALID_ARGUMENT, 1);
        assert_eq!(RESULT_ABI_MISMATCH, 2);
        assert_eq!(RESULT_ALLOCATION_FAILED, 3);
        assert_eq!(RESULT_INTERNAL_ERROR, 4);
        assert_eq!(RESULT_UNAVAILABLE, 5);
        assert_eq!(RESULT_WRONG_THREAD, 6);
        assert_eq!(RESULT_CALLBACK_IN_FLIGHT, 7);
        assert_eq!(RESULT_INVALID_STATE, 8);
        assert_eq!(size_of::<NativeResult>(), 4);
    }

    #[test]
    fn fixed_width_abi_struct_layout_is_architecture_independent() {
        assert_eq!(size_of::<NavopRdpProbeOptions>(), 8);
        assert_eq!(align_of::<NavopRdpProbeOptions>(), 4);
        assert_eq!(size_of::<NavopRdpProbeResult>(), 16);
        assert_eq!(align_of::<NavopRdpProbeResult>(), 4);
        assert_eq!(size_of::<NavopRdpLastError>(), 36);
        assert_eq!(align_of::<NavopRdpLastError>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, struct_size), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, abi_version), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, result), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, hresult), 12);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, has_hresult), 16);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, reserved), 20);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, stage), 24);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, win32_code), 28);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, has_win32_code), 32);
        assert_eq!(size_of::<NavopRdpCreateOptions>(), 16);
        assert_eq!(align_of::<NavopRdpCreateOptions>(), 4);
        assert_eq!(size_of::<NavopRdpBounds>(), 16);
        assert_eq!(align_of::<NavopRdpBounds>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, x), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, y), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, width), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, height), 12);
        assert_eq!(size_of::<NavopRdpEvent>(), 32);
        assert_eq!(align_of::<NavopRdpEvent>(), 4);
        assert_eq!(size_of::<NavopRdpEventCallbackOptions>(), 16);
        assert_eq!(align_of::<NavopRdpEventCallbackOptions>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, struct_size), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, abi_version), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, kind), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, reserved), 12);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, generation_low), 16);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, generation_high), 20);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, code), 24);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, payload_len), 28);
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, abi_version),
            4
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_low),
            8
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_high),
            12
        );
    }

    #[test]
    fn credential_layout_matches_the_current_pointer_width() {
        assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, data), 0);
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, abi_version),
            4
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, server_password),
            8
        );

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedSecret>(), 16);
            assert_eq!(align_of::<NavopRdpBorrowedSecret>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, len), 8);
            assert_eq!(size_of::<NavopRdpCredentialBundle>(), 48);
            assert_eq!(align_of::<NavopRdpCredentialBundle>(), 8);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password),
                24
            );
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, flags), 40);
        }

        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedSecret>(), 8);
            assert_eq!(align_of::<NavopRdpBorrowedSecret>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, len), 4);
            assert_eq!(size_of::<NavopRdpCredentialBundle>(), 28);
            assert_eq!(align_of::<NavopRdpCredentialBundle>(), 4);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password),
                16
            );
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, flags), 24);
        }
    }

    #[test]
    fn connection_layout_matches_the_current_pointer_width() {
        assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, data), 0);
        assert_eq!(
            std::mem::offset_of!(NavopRdpConnectionOptions, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpConnectionOptions, abi_version),
            4
        );
        assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, host), 8);

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedUtf16>(), 16);
            assert_eq!(align_of::<NavopRdpBorrowedUtf16>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len), 8);
            assert_eq!(size_of::<NavopRdpConnectionOptions>(), 48);
            assert_eq!(align_of::<NavopRdpConnectionOptions>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, port), 24);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width),
                28
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height),
                32
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, color_depth),
                36
            );
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, flags), 40);
        }

        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedUtf16>(), 8);
            assert_eq!(align_of::<NavopRdpBorrowedUtf16>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len), 4);
            assert_eq!(size_of::<NavopRdpConnectionOptions>(), 36);
            assert_eq!(align_of::<NavopRdpConnectionOptions>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, port), 16);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width),
                20
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height),
                24
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, color_depth),
                28
            );
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, flags), 32);
        }
    }

    #[test]
    fn create_options_split_the_generation_without_abi_alignment_risk() {
        let options = NavopRdpCreateOptions::current(0x1122_3344_aabb_ccdd);

        assert_eq!(options.generation_low, 0xaabb_ccdd);
        assert_eq!(options.generation_high, 0x1122_3344);
    }

    #[test]
    fn callback_options_split_the_generation_without_abi_alignment_risk() {
        let options = NavopRdpEventCallbackOptions::current(0x1122_3344_aabb_ccdd);

        assert_eq!(options.generation_low, 0xaabb_ccdd);
        assert_eq!(options.generation_high, 0x1122_3344);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_probe_preserves_an_extended_caller_result_size() {
        let options = NavopRdpProbeOptions::current();
        let mut result = NavopRdpProbeResult::current();
        result.struct_size += 16;

        let native_result = unsafe { probe(&options, &mut result) };

        assert_eq!(native_result, RESULT_OK);
        assert_eq!(
            result.struct_size,
            size_of::<NavopRdpProbeResult>() as u32 + 16
        );
        assert_eq!(result.abi_version, ABI_VERSION);
        assert_eq!(result.reserved, 0);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_credentials_validate_size_before_version_and_borrowed_pointers() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let mut credentials = NavopRdpCredentialBundle {
            struct_size: size_of::<NavopRdpCredentialBundle>() as u32,
            abi_version: ABI_VERSION,
            server_password: NavopRdpBorrowedSecret {
                data: std::ptr::null(),
                len: 0,
            },
            gateway_password: NavopRdpBorrowedSecret {
                data: std::ptr::null(),
                len: 0,
            },
            flags: 0,
        };

        credentials.struct_size = 4;
        credentials.abi_version += 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );

        credentials.struct_size = size_of::<NavopRdpCredentialBundle>() as u32;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_ABI_MISMATCH
        );

        credentials.abi_version = ABI_VERSION;
        credentials.server_password.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );

        credentials.server_password.len = 0;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_connection_validates_layout_values_and_outputs_before_unavailable() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let host_name: Vec<u16> = "rdp.example".encode_utf16().collect();
        let mut options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );

        options.struct_size = 4;
        options.abi_version += 1;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.struct_size = size_of::<NavopRdpConnectionOptions>() as u32;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_ABI_MISMATCH);

        options.abi_version = ABI_VERSION;
        options.host.data = std::ptr::null();
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.host.data = host_name.as_ptr();
        options.port = 0;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.port = 3389;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        let mut state = u32::MAX;
        assert_eq!(
            unsafe { get_connection_state(std::ptr::null_mut(), &mut state) },
            RESULT_INVALID_ARGUMENT
        );
        assert_eq!(state, CONNECTION_STATE_DISCONNECTED);

        let mut status = u32::MAX;
        assert_eq!(
            unsafe { request_close(std::ptr::null_mut(), &mut status) },
            RESULT_INVALID_ARGUMENT
        );
        assert_eq!(status, REQUEST_CLOSE_CAN_PROCEED);
    }
}
