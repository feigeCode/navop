use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use crate::ffi::{
    ABI_VERSION, CREATE_STAGE_CREATE_CONTROL, CREATE_STAGE_NONE, CREATE_WITH_PARENT_ABI_VERSION,
    EVENT_CLOSE_CONFIRMED, EVENT_CONNECTED, EVENT_DISCONNECTED, EVENT_FATAL_ERROR,
    EVENT_LOGON_ERROR, EVENT_NETWORK_STATUS_CHANGED, EVENT_RECONNECTING,
    EVENT_REMOTE_DESKTOP_SIZE_CHANGED, EVENT_WARNING, LAST_ERROR_LEGACY_SIZE,
    MAX_EVENT_PAYLOAD_BYTES, NativeEventCallback, NativeRdpHost, NativeResult,
    NavopRdpBorrowedSecret, NavopRdpBorrowedUtf16, NavopRdpBounds, NavopRdpConnectionOptions,
    NavopRdpCreateOptions, NavopRdpCreateWithParentOptions, NavopRdpCredentialBundle,
    NavopRdpEvent, NavopRdpEventCallbackOptions, NavopRdpLastError, RESULT_ABI_MISMATCH,
    RESULT_CALLBACK_IN_FLIGHT, RESULT_INTERNAL_ERROR, RESULT_INVALID_ARGUMENT, RESULT_OK,
    RESULT_UNAVAILABLE, RESULT_WRONG_THREAD,
};

const VT_I4: u16 = 3;
const VT_BOOL: u16 = 11;
const VT_UI4: u16 = 19;
const VT_BYREF: u16 = 0x4000;

unsafe extern "C" {
    fn navop_rdp_create(
        options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult;
    fn navop_rdp_create_with_parent(
        options: *const NavopRdpCreateWithParentOptions,
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
    fn navop_rdp_test_dispatch_event(
        host: *mut NativeRdpHost,
        event: *const NavopRdpEvent,
        payload: *const u8,
    ) -> NativeResult;
    fn navop_rdp_test_invoke_active_x_event(
        host: *mut NativeRdpHost,
        dispatch_id: i32,
        arguments: *mut i32,
        variant_types: *const u16,
        argument_count: u32,
    ) -> NativeResult;
    fn navop_rdp_test_dispatch_disconnect_event(
        host: *mut NativeRdpHost,
        disconnect_code: i32,
        has_extended_code: u32,
        extended_code: i32,
    ) -> NativeResult;
    fn navop_rdp_test_set_last_error(
        host: *mut NativeRdpHost,
        result: NativeResult,
        stage: u32,
        has_hresult: u32,
        hresult: i32,
        has_win32_code: u32,
        win32_code: u32,
    ) -> NativeResult;
}

unsafe extern "system" {
    fn CreateWindowExW(
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
    fn DestroyWindow(window: *mut c_void) -> i32;
    fn IsWindow(window: *mut c_void) -> i32;
}

fn event(generation: u64, kind: u32, code: i32, payload_len: u32) -> NavopRdpEvent {
    NavopRdpEvent {
        struct_size: size_of::<NavopRdpEvent>() as u32,
        abi_version: ABI_VERSION,
        kind,
        reserved: 0,
        generation_low: generation as u32,
        generation_high: (generation >> 32) as u32,
        code,
        payload_len,
    }
}

unsafe fn create_host(generation: u64) -> *mut NativeRdpHost {
    let options = NavopRdpCreateOptions::current(generation);
    let mut host = ptr::null_mut();
    assert_eq!(unsafe { navop_rdp_create(&options, &mut host) }, RESULT_OK);
    assert!(!host.is_null());
    host
}

unsafe fn read_last_error(host: *mut NativeRdpHost) -> NavopRdpLastError {
    let mut error = NavopRdpLastError::current();
    assert_eq!(
        unsafe { navop_rdp_get_last_error(host, &mut error) },
        RESULT_OK
    );
    error
}

unsafe fn register_callback(
    host: *mut NativeRdpHost,
    generation: u64,
    callback: NativeEventCallback,
    context: *mut c_void,
) {
    let options = NavopRdpEventCallbackOptions::current(generation);
    assert_eq!(
        unsafe { navop_rdp_register_event_callback(host, &options, Some(callback), context) },
        RESULT_OK
    );
}

fn assert_dispatch_rejected(
    host: *mut NativeRdpHost,
    event: &NavopRdpEvent,
    payload: *const u8,
    expected: NativeResult,
    context: &RecordingContext,
) {
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, event, payload) },
        expected
    );
    assert_eq!(context.calls, 0);
}

fn assert_invalid_events_rejected(
    host: *mut NativeRdpHost,
    generation: u64,
    context: &RecordingContext,
) {
    let mut invalid = event(generation, 1, 0, 0);
    invalid.struct_size -= 1;
    assert_dispatch_rejected(
        host,
        &invalid,
        ptr::null(),
        RESULT_INVALID_ARGUMENT,
        context,
    );
    invalid = event(generation, 1, 0, 0);
    invalid.abi_version += 1;
    assert_dispatch_rejected(host, &invalid, ptr::null(), RESULT_ABI_MISMATCH, context);
    invalid = event(generation, 1, 0, 0);
    invalid.reserved = 1;
    assert_dispatch_rejected(
        host,
        &invalid,
        ptr::null(),
        RESULT_INVALID_ARGUMENT,
        context,
    );
    invalid = event(generation ^ 1, 1, 0, 0);
    assert_dispatch_rejected(
        host,
        &invalid,
        ptr::null(),
        RESULT_INVALID_ARGUMENT,
        context,
    );
    invalid = event(generation, 1, 0, 1);
    assert_dispatch_rejected(
        host,
        &invalid,
        ptr::null(),
        RESULT_INVALID_ARGUMENT,
        context,
    );
    invalid = event(generation, 1, 0, MAX_EVENT_PAYLOAD_BYTES + 1);
    assert_dispatch_rejected(
        host,
        &invalid,
        ptr::dangling(),
        RESULT_INVALID_ARGUMENT,
        context,
    );
}

fn credential_bundle(server: Option<&[u16]>, gateway: Option<&[u16]>) -> NavopRdpCredentialBundle {
    fn borrowed_secret(secret: Option<&[u16]>) -> NavopRdpBorrowedSecret {
        match secret {
            Some(secret) if !secret.is_empty() => NavopRdpBorrowedSecret {
                data: secret.as_ptr(),
                len: secret.len() as u32,
            },
            _ => NavopRdpBorrowedSecret {
                data: ptr::null(),
                len: 0,
            },
        }
    }

    NavopRdpCredentialBundle {
        struct_size: size_of::<NavopRdpCredentialBundle>() as u32,
        abi_version: ABI_VERSION,
        server_password: borrowed_secret(server),
        gateway_password: borrowed_secret(gateway),
        flags: 0,
    }
}

fn connection_options(host: &[u16]) -> NavopRdpConnectionOptions {
    NavopRdpConnectionOptions::current(
        NavopRdpBorrowedUtf16 {
            data: host.as_ptr(),
            len: host.len() as u32,
        },
        3389,
        1280,
        720,
        32,
    )
}

fn create_hidden_test_parent() -> *mut c_void {
    const CLASS_NAME: &[u16] = &[
        b'S' as u16,
        b'T' as u16,
        b'A' as u16,
        b'T' as u16,
        b'I' as u16,
        b'C' as u16,
        0,
    ];
    const WINDOW_NAME: &[u16] = &[b'n' as u16, b'a' as u16, b'v' as u16, 0];

    unsafe {
        CreateWindowExW(
            0,
            CLASS_NAME.as_ptr(),
            WINDOW_NAME.as_ptr(),
            0,
            0,
            0,
            32,
            32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    }
}

#[test]
fn native_create_with_parent_rejects_invalid_abi_and_never_returns_a_handle() {
    let generation = 0x1122_3344_aabb_ccdd;
    let mut options = NavopRdpCreateWithParentOptions::current(generation, 1);
    let mut host = 1_usize as *mut NativeRdpHost;

    assert_eq!(
        unsafe { navop_rdp_create_with_parent(ptr::null(), &mut host) },
        RESULT_INVALID_ARGUMENT
    );
    assert!(host.is_null());

    host = 1_usize as *mut NativeRdpHost;
    options.struct_size -= 1;
    assert_eq!(
        unsafe { navop_rdp_create_with_parent(&options, &mut host) },
        RESULT_INVALID_ARGUMENT
    );
    assert!(host.is_null());

    host = 1_usize as *mut NativeRdpHost;
    options = NavopRdpCreateWithParentOptions::current(generation, 1);
    options.abi_version = CREATE_WITH_PARENT_ABI_VERSION + 1;
    assert_eq!(
        unsafe { navop_rdp_create_with_parent(&options, &mut host) },
        RESULT_ABI_MISMATCH
    );
    assert!(host.is_null());

    host = 1_usize as *mut NativeRdpHost;
    options = NavopRdpCreateWithParentOptions::current(generation, 0);
    assert_eq!(
        unsafe { navop_rdp_create_with_parent(&options, &mut host) },
        RESULT_INVALID_ARGUMENT
    );
    assert!(host.is_null());

    host = 1_usize as *mut NativeRdpHost;
    options = NavopRdpCreateWithParentOptions::current(generation, 1);
    assert_eq!(
        unsafe { navop_rdp_create_with_parent(&options, &mut host) },
        RESULT_INVALID_ARGUMENT
    );
    assert!(host.is_null());
}

#[repr(C)]
struct ExtendedLastError {
    base: NavopRdpLastError,
    trailing: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LegacyLastError {
    struct_size: u32,
    abi_version: u32,
    result: NativeResult,
    hresult: i32,
    has_hresult: u32,
    reserved: u32,
}

#[test]
fn native_create_with_parent_v2_populates_and_validates_failure_diagnostics() {
    let generation = 0x1122_3344_aabb_ccdd;
    let mut host = 1_usize as *mut NativeRdpHost;
    let mut extended = ExtendedLastError {
        base: NavopRdpLastError {
            struct_size: size_of::<ExtendedLastError>() as u32,
            ..NavopRdpLastError::current()
        },
        trailing: [0xa5; 16],
    };

    assert_eq!(
        unsafe { navop_rdp_create_with_parent_v2(ptr::null(), &mut host, &mut extended.base,) },
        RESULT_INVALID_ARGUMENT
    );
    assert!(host.is_null());
    assert_eq!(
        extended.base.struct_size,
        size_of::<ExtendedLastError>() as u32
    );
    assert_eq!(extended.base.abi_version, ABI_VERSION);
    assert_eq!(extended.base.result, RESULT_INVALID_ARGUMENT);
    assert_eq!(extended.base.hresult, 0);
    assert_eq!(extended.base.has_hresult, 0);
    assert_eq!(extended.base.reserved, 0);
    assert_eq!(extended.base.stage, CREATE_STAGE_NONE);
    assert_eq!(extended.base.win32_code, 0);
    assert_eq!(extended.base.has_win32_code, 0);
    assert_eq!(extended.trailing, [0xa5; 16]);

    let mut diagnostic = NavopRdpLastError::current();
    assert_eq!(
        unsafe { navop_rdp_create_with_parent_v2(ptr::null(), ptr::null_mut(), &mut diagnostic,) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(diagnostic.result, RESULT_INVALID_ARGUMENT);

    host = 1_usize as *mut NativeRdpHost;
    let mut options = NavopRdpCreateWithParentOptions::current(generation, 1);
    options.abi_version = CREATE_WITH_PARENT_ABI_VERSION + 1;
    diagnostic = NavopRdpLastError::current();
    assert_eq!(
        unsafe { navop_rdp_create_with_parent_v2(&options, &mut host, &mut diagnostic) },
        RESULT_ABI_MISMATCH
    );
    assert!(host.is_null());
    assert_eq!(diagnostic.result, RESULT_ABI_MISMATCH);
    assert_eq!(diagnostic.has_hresult, 0);

    host = 1_usize as *mut NativeRdpHost;
    let mut invalid_layout = NavopRdpLastError {
        struct_size: LAST_ERROR_LEGACY_SIZE - 1,
        result: RESULT_INTERNAL_ERROR,
        hresult: i32::MIN,
        has_hresult: 1,
        reserved: 7,
        ..NavopRdpLastError::current()
    };
    let original_invalid_layout = invalid_layout;
    assert_eq!(
        unsafe { navop_rdp_create_with_parent_v2(ptr::null(), &mut host, &mut invalid_layout,) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(host, 1_usize as *mut NativeRdpHost);
    assert_eq!(invalid_layout, original_invalid_layout);

    assert_eq!(
        unsafe { navop_rdp_create_with_parent_v2(ptr::null(), &mut host, ptr::null_mut(),) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(host, 1_usize as *mut NativeRdpHost);
}

#[test]
fn native_create_with_parent_rejects_a_parent_owned_by_another_thread() {
    let parent = create_hidden_test_parent();
    assert!(!parent.is_null(), "STATIC parent window should be created");
    assert_eq!(unsafe { IsWindow(parent) }, 1);

    let parent_raw = parent as usize;
    let result = std::thread::spawn(move || {
        let options = NavopRdpCreateWithParentOptions::current(7, parent_raw);
        let mut host = ptr::null_mut();
        let result = unsafe { navop_rdp_create_with_parent(&options, &mut host) };
        (result, host.is_null())
    })
    .join()
    .expect("wrong-thread test worker should finish");

    assert_eq!(result.0, RESULT_WRONG_THREAD);
    assert!(result.1);
    assert_eq!(unsafe { IsWindow(parent) }, 1);
    assert_eq!(unsafe { DestroyWindow(parent) }, 1);
}

#[test]
fn native_presentation_controls_validate_lifecycle_thread_and_arguments() {
    let generation = 0x1122_3344_aabb_ccdd;
    let bounds = NavopRdpBounds::new(-4, 8, 640, 480);

    assert_eq!(
        unsafe { navop_rdp_set_bounds(ptr::null_mut(), &bounds) },
        RESULT_INVALID_ARGUMENT
    );
    let mut host = unsafe { create_host(generation) };
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    let negative_width = NavopRdpBounds::new(0, 0, -1, 1);
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, &negative_width) },
        RESULT_INVALID_ARGUMENT
    );
    let negative_height = NavopRdpBounds::new(0, 0, 1, -1);
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, &negative_height) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, &bounds) },
        RESULT_UNAVAILABLE
    );

    assert_eq!(
        unsafe { navop_rdp_set_visible(ptr::null_mut(), 0) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_set_visible(host, 2) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_set_visible(host, 1) },
        RESULT_UNAVAILABLE
    );

    assert_eq!(
        unsafe { navop_rdp_focus(ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { navop_rdp_focus(host) }, RESULT_UNAVAILABLE);

    let host_address = host as usize;
    let wrong_thread_bounds = bounds;
    let wrong_thread_results = std::thread::spawn(move || {
        let host = host_address as *mut NativeRdpHost;
        (
            unsafe { navop_rdp_set_bounds(host, &wrong_thread_bounds) },
            unsafe { navop_rdp_set_visible(host, 1) },
            unsafe { navop_rdp_focus(host) },
        )
    })
    .join()
    .expect("wrong-thread presentation test worker should finish");
    assert_eq!(
        wrong_thread_results,
        (
            RESULT_WRONG_THREAD,
            RESULT_WRONG_THREAD,
            RESULT_WRONG_THREAD
        )
    );

    assert_eq!(
        unsafe {
            navop_rdp_register_event_callback(
                host,
                &NavopRdpEventCallbackOptions::current(generation),
                Some(record_callback),
                ptr::null_mut(),
            )
        },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, &bounds) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_set_visible(host, 0) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { navop_rdp_focus(host) }, RESULT_INVALID_ARGUMENT);
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[derive(Default)]
struct RecordingContext {
    calls: u32,
    generation: u64,
    kind: u32,
    code: i32,
    payload: Vec<u8>,
}

unsafe extern "C" fn record_callback(
    context: *mut c_void,
    event: *const NavopRdpEvent,
    payload: *const u8,
) {
    let context = unsafe { &mut *context.cast::<RecordingContext>() };
    let event = unsafe { &*event };
    context.calls += 1;
    context.generation = u64::from(event.generation_low) | (u64::from(event.generation_high) << 32);
    context.kind = event.kind;
    context.code = event.code;
    context.payload.clear();
    if event.payload_len > 0 {
        context.payload =
            unsafe { std::slice::from_raw_parts(payload, event.payload_len as usize).to_vec() };
    }
}

#[test]
fn native_last_error_preserves_signed_hresult_and_repeated_reads() {
    let mut host = unsafe { create_host(42) };

    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INTERNAL_ERROR,
                CREATE_STAGE_CREATE_CONTROL,
                1,
                i32::MIN,
                1,
                1407,
            )
        },
        RESULT_INTERNAL_ERROR
    );
    let first = unsafe { read_last_error(host) };
    assert_eq!(first.result, RESULT_INTERNAL_ERROR);
    assert_eq!(first.hresult, i32::MIN);
    assert_eq!(first.has_hresult, 1);
    assert_eq!(first.reserved, 0);
    assert_eq!(first.stage, CREATE_STAGE_CREATE_CONTROL);
    assert_eq!(first.win32_code, 1407);
    assert_eq!(first.has_win32_code, 1);
    assert_eq!(unsafe { read_last_error(host) }, first);

    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INVALID_ARGUMENT,
                CREATE_STAGE_NONE,
                2,
                7,
                0,
                0,
            )
        },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { read_last_error(host) }, first);

    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INVALID_ARGUMENT,
                CREATE_STAGE_NONE,
                0,
                0,
                2,
                7,
            )
        },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { read_last_error(host) }, first);

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_owner_operations_replace_or_clear_stale_hresult() {
    let generation = 42;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };

    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INTERNAL_ERROR,
                CREATE_STAGE_CREATE_CONTROL,
                1,
                i32::MIN,
                1,
                1407,
            )
        },
        RESULT_INTERNAL_ERROR
    );
    assert_eq!(
        unsafe { navop_rdp_set_bounds(host, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    let replaced = unsafe { read_last_error(host) };
    assert_eq!(replaced.result, RESULT_INVALID_ARGUMENT);
    assert_eq!(replaced.hresult, 0);
    assert_eq!(replaced.has_hresult, 0);
    assert_eq!(replaced.stage, CREATE_STAGE_NONE);
    assert_eq!(replaced.win32_code, 0);
    assert_eq!(replaced.has_win32_code, 0);

    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INTERNAL_ERROR,
                CREATE_STAGE_CREATE_CONTROL,
                1,
                i32::MIN,
                1,
                1407,
            )
        },
        RESULT_INTERNAL_ERROR
    );
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }
    assert_eq!(
        unsafe { read_last_error(host) },
        NavopRdpLastError::current()
    );

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_wrong_thread_diagnostic_access_preserves_owner_slot() {
    let mut host = unsafe { create_host(42) };
    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INTERNAL_ERROR,
                CREATE_STAGE_CREATE_CONTROL,
                1,
                i32::MIN,
                1,
                1407,
            )
        },
        RESULT_INTERNAL_ERROR
    );
    let expected = unsafe { read_last_error(host) };
    let host_address = host as usize;

    let (set_result, read_result, output, sentinel) = std::thread::spawn(move || {
        let host = host_address as *mut NativeRdpHost;
        let sentinel = NavopRdpLastError {
            result: RESULT_INVALID_ARGUMENT,
            hresult: 17,
            has_hresult: 1,
            reserved: 23,
            stage: 29,
            win32_code: 31,
            has_win32_code: 1,
            ..NavopRdpLastError::current()
        };
        let mut output = sentinel;
        let set_result = unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INVALID_ARGUMENT,
                CREATE_STAGE_NONE,
                0,
                0,
                0,
                0,
            )
        };
        let read_result = unsafe { navop_rdp_get_last_error(host, &mut output) };
        (set_result, read_result, output, sentinel)
    })
    .join()
    .expect("wrong-thread diagnostic worker should finish");

    assert_eq!(set_result, RESULT_WRONG_THREAD);
    assert_eq!(read_result, RESULT_WRONG_THREAD);
    assert_eq!(output, sentinel);
    assert_eq!(unsafe { read_last_error(host) }, expected);

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_last_error_preserves_extended_output_and_rejects_invalid_layout() {
    let mut host = unsafe { create_host(42) };
    assert_eq!(
        unsafe {
            navop_rdp_test_set_last_error(
                host,
                RESULT_INTERNAL_ERROR,
                CREATE_STAGE_CREATE_CONTROL,
                1,
                i32::MIN,
                1,
                1407,
            )
        },
        RESULT_INTERNAL_ERROR
    );

    let mut extended = ExtendedLastError {
        base: NavopRdpLastError {
            struct_size: size_of::<ExtendedLastError>() as u32,
            ..NavopRdpLastError::current()
        },
        trailing: [0x5a; 16],
    };
    assert_eq!(
        unsafe { navop_rdp_get_last_error(host, &mut extended.base) },
        RESULT_OK
    );
    assert_eq!(
        extended.base.struct_size,
        size_of::<ExtendedLastError>() as u32
    );
    assert_eq!(extended.base.abi_version, ABI_VERSION);
    assert_eq!(extended.base.result, RESULT_INTERNAL_ERROR);
    assert_eq!(extended.base.hresult, i32::MIN);
    assert_eq!(extended.base.has_hresult, 1);
    assert_eq!(extended.base.reserved, 0);
    assert_eq!(extended.base.stage, CREATE_STAGE_CREATE_CONTROL);
    assert_eq!(extended.base.win32_code, 1407);
    assert_eq!(extended.base.has_win32_code, 1);
    assert_eq!(extended.trailing, [0x5a; 16]);

    let mut legacy = LegacyLastError {
        struct_size: LAST_ERROR_LEGACY_SIZE,
        abi_version: ABI_VERSION,
        result: RESULT_OK,
        hresult: 0,
        has_hresult: 0,
        reserved: 0,
    };
    assert_eq!(
        unsafe {
            navop_rdp_get_last_error(
                host,
                (&mut legacy as *mut LegacyLastError).cast::<NavopRdpLastError>(),
            )
        },
        RESULT_OK
    );
    assert_eq!(
        legacy,
        LegacyLastError {
            struct_size: LAST_ERROR_LEGACY_SIZE,
            abi_version: ABI_VERSION,
            result: RESULT_INTERNAL_ERROR,
            hresult: i32::MIN,
            has_hresult: 1,
            reserved: 0,
        }
    );

    let mut short = NavopRdpLastError {
        struct_size: LAST_ERROR_LEGACY_SIZE - 1,
        result: RESULT_INVALID_ARGUMENT,
        hresult: 31,
        has_hresult: 1,
        reserved: 37,
        ..NavopRdpLastError::current()
    };
    let original_short = short;
    assert_eq!(
        unsafe { navop_rdp_get_last_error(host, &mut short) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(short, original_short);

    let mut wrong_abi = NavopRdpLastError {
        abi_version: ABI_VERSION + 1,
        result: RESULT_INVALID_ARGUMENT,
        hresult: 41,
        has_hresult: 1,
        reserved: 43,
        ..NavopRdpLastError::current()
    };
    let original_wrong_abi = wrong_abi;
    assert_eq!(
        unsafe { navop_rdp_get_last_error(host, &mut wrong_abi) },
        RESULT_ABI_MISMATCH
    );
    assert_eq!(wrong_abi, original_wrong_abi);
    let mut current_sized_expected = extended.base;
    current_sized_expected.struct_size = size_of::<NavopRdpLastError>() as u32;
    assert_eq!(unsafe { read_last_error(host) }, current_sized_expected);

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_dispatch_rejects_invalid_events_without_poisoning_callback() {
    let generation = 0x1122_3344_aabb_ccdd;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }

    assert_invalid_events_rejected(host, generation, &context);

    let valid = event(generation, EVENT_CONNECTED, 0, 0);
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &valid, ptr::null()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &valid, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(context.calls, 1);
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_dispatch_invokes_the_registered_callback_once() {
    let generation = 0x1122_3344_aabb_ccdd;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }
    let payload = [1920_u32.to_le_bytes(), 1080_u32.to_le_bytes()].concat();
    let native_event = event(
        generation,
        EVENT_REMOTE_DESKTOP_SIZE_CHANGED,
        0,
        payload.len() as u32,
    );

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, payload.as_ptr()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.generation, generation);
    assert_eq!(context.kind, EVENT_REMOTE_DESKTOP_SIZE_CHANGED);
    assert_eq!(context.code, 0);
    assert_eq!(context.payload, payload);

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn active_x_event_sink_maps_known_events_and_ignores_unknown_or_malformed_invocations() {
    let generation = 0x1122_3344_aabb_ccdd;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }

    assert_eq!(
        unsafe { navop_rdp_test_invoke_active_x_event(host, 2, ptr::null_mut(), ptr::null(), 0,) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.kind, EVENT_CONNECTED);
    assert_eq!(context.code, 0);

    let mut size_arguments = [1080, 1920];
    let size_types = [VT_I4, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                12,
                size_arguments.as_mut_ptr(),
                size_types.as_ptr(),
                size_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 2);
    assert_eq!(context.kind, EVENT_REMOTE_DESKTOP_SIZE_CHANGED);
    assert_eq!(
        context.payload,
        [1920_u32.to_le_bytes(), 1080_u32.to_le_bytes()].concat()
    );

    let mut reconnect_arguments = [8, 3, -1, 1234];
    let reconnect_types = [VT_I4, VT_I4, VT_BOOL, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                34,
                reconnect_arguments.as_mut_ptr(),
                reconnect_types.as_ptr(),
                reconnect_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);
    assert_eq!(context.kind, EVENT_RECONNECTING);
    assert_eq!(
        context.payload,
        [3_u32.to_le_bytes(), 8_u32.to_le_bytes()].concat()
    );

    let mut legacy_reconnect_arguments = [2, 4, 5678];
    let legacy_reconnect_types = [VT_I4 | VT_BYREF, VT_I4, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                17,
                legacy_reconnect_arguments.as_mut_ptr(),
                legacy_reconnect_types.as_ptr(),
                legacy_reconnect_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(legacy_reconnect_arguments[0], 0);
    assert_eq!(context.calls, 4);
    assert_eq!(context.kind, EVENT_RECONNECTING);
    assert_eq!(context.payload, 4_u32.to_le_bytes());

    let mut network_arguments = [45, 10_000, 3];
    let network_types = [VT_I4, VT_I4, VT_UI4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                32,
                network_arguments.as_mut_ptr(),
                network_types.as_ptr(),
                network_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 5);
    assert_eq!(context.kind, EVENT_NETWORK_STATUS_CHANGED);
    assert_eq!(context.payload, 3_u32.to_le_bytes());

    let mut disconnect_arguments = [-1234];
    let disconnect_types = [VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                4,
                disconnect_arguments.as_mut_ptr(),
                disconnect_types.as_ptr(),
                disconnect_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 6);
    assert_eq!(context.kind, EVENT_DISCONNECTED);
    assert_eq!(context.code, -1234);

    let mut allow_close = [0];
    let allow_close_types = [VT_BOOL | VT_BYREF];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                15,
                allow_close.as_mut_ptr(),
                allow_close_types.as_ptr(),
                allow_close.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(allow_close, [-1]);
    assert_eq!(context.calls, 7);
    assert_eq!(context.kind, EVENT_CLOSE_CONFIRMED);

    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(host, 10_000, ptr::null_mut(), ptr::null(), 0)
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 7);

    let mut malformed_size_arguments = [1080, 1920];
    let malformed_size_types = [VT_UI4, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                12,
                malformed_size_arguments.as_mut_ptr(),
                malformed_size_types.as_ptr(),
                malformed_size_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 7);

    let mut malformed_reconnect_arguments = [8, 3, 1, 1234];
    let malformed_reconnect_types = [VT_I4, VT_I4, VT_I4, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                34,
                malformed_reconnect_arguments.as_mut_ptr(),
                malformed_reconnect_types.as_ptr(),
                malformed_reconnect_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 7);

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn active_x_diagnostic_events_preserve_signed_codes_and_ignore_malformed_invocations() {
    let generation = 0x1020_3040_5060_7080;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }

    let mut fatal_arguments = [100];
    let fatal_types = [VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                10,
                fatal_arguments.as_mut_ptr(),
                fatal_types.as_ptr(),
                fatal_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.kind, EVENT_FATAL_ERROR);
    assert_eq!(context.code, 100);
    assert!(context.payload.is_empty());

    let mut warning_arguments = [-7];
    let warning_types = [VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                11,
                warning_arguments.as_mut_ptr(),
                warning_types.as_ptr(),
                warning_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 2);
    assert_eq!(context.kind, EVENT_WARNING);
    assert_eq!(context.code, -7);
    assert!(context.payload.is_empty());

    let status_logon_failure = -1_073_741_715;
    let mut logon_arguments = [status_logon_failure];
    let logon_types = [VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                22,
                logon_arguments.as_mut_ptr(),
                logon_types.as_ptr(),
                logon_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);
    assert_eq!(context.kind, EVENT_LOGON_ERROR);
    assert_eq!(context.code, status_logon_failure);
    assert!(context.payload.is_empty());

    assert_eq!(
        unsafe { navop_rdp_test_invoke_active_x_event(host, 10, ptr::null_mut(), ptr::null(), 0) },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);

    let mut excess_arguments = [1, 2];
    let excess_types = [VT_I4, VT_I4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                11,
                excess_arguments.as_mut_ptr(),
                excess_types.as_ptr(),
                excess_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);

    let mut unsigned_arguments = [status_logon_failure];
    let unsigned_types = [VT_UI4];
    assert_eq!(
        unsafe {
            navop_rdp_test_invoke_active_x_event(
                host,
                22,
                unsigned_arguments.as_mut_ptr(),
                unsigned_types.as_ptr(),
                unsigned_arguments.len() as u32,
            )
        },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn active_x_disconnect_dispatch_encodes_optional_extended_reason() {
    let generation = 0x8877_6655_4433_2211;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_disconnect_event(host, -1234, 1, 0x0102_0304,) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.kind, EVENT_DISCONNECTED);
    assert_eq!(context.code, -1234);
    assert_eq!(context.payload, 0x0102_0304_i32.to_le_bytes());

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_disconnect_event(host, i32::MAX, 1, i32::MIN,) },
        RESULT_OK
    );
    assert_eq!(context.calls, 2);
    assert_eq!(context.kind, EVENT_DISCONNECTED);
    assert_eq!(context.code, i32::MAX);
    assert_eq!(context.payload, i32::MIN.to_le_bytes());

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_disconnect_event(host, i32::MIN, 0, i32::MAX,) },
        RESULT_OK
    );
    assert_eq!(context.calls, 3);
    assert_eq!(context.kind, EVENT_DISCONNECTED);
    assert_eq!(context.code, i32::MIN);
    assert!(context.payload.is_empty());

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_disconnect_event(host, 0, 2, 0,) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(context.calls, 3);

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

struct ReentrantUnregisterContext {
    host: *mut NativeRdpHost,
    calls: u32,
    unregister_result: NativeResult,
}

unsafe extern "C" fn reentrant_unregister_callback(
    context: *mut c_void,
    _event: *const NavopRdpEvent,
    _payload: *const u8,
) {
    let context = unsafe { &mut *context.cast::<ReentrantUnregisterContext>() };
    context.calls += 1;
    context.unregister_result = unsafe { navop_rdp_unregister_event_callback(context.host) };
}

#[test]
fn reentrant_unregister_is_rejected_until_callback_returns() {
    let generation = 42;
    let mut host = unsafe { create_host(generation) };
    let mut context = ReentrantUnregisterContext {
        host,
        calls: 0,
        unregister_result: RESULT_OK,
    };
    unsafe {
        register_callback(
            host,
            generation,
            reentrant_unregister_callback,
            (&mut context as *mut ReentrantUnregisterContext).cast(),
        );
    }
    let native_event = event(generation, 1, 0, 0);

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.unregister_result, RESULT_CALLBACK_IN_FLIGHT);

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(context.calls, 1);
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

struct ReentrantDestroyContext {
    host_slot: *mut *mut NativeRdpHost,
    calls: u32,
    destroy_result: NativeResult,
    nested_dispatch_result: NativeResult,
}

unsafe extern "C" fn reentrant_destroy_callback(
    context: *mut c_void,
    event: *const NavopRdpEvent,
    payload: *const u8,
) {
    let context = unsafe { &mut *context.cast::<ReentrantDestroyContext>() };
    context.calls += 1;
    context.destroy_result = unsafe { navop_rdp_destroy(context.host_slot) };
    context.nested_dispatch_result =
        unsafe { navop_rdp_test_dispatch_event(*context.host_slot, event, payload) };
}

#[test]
fn reentrant_destroy_preserves_the_handle_until_callback_returns() {
    let generation = 42;
    let mut host = unsafe { create_host(generation) };
    let original_host = host;
    let mut context = ReentrantDestroyContext {
        host_slot: &mut host,
        calls: 0,
        destroy_result: RESULT_OK,
        nested_dispatch_result: RESULT_OK,
    };
    unsafe {
        register_callback(
            host,
            generation,
            reentrant_destroy_callback,
            (&mut context as *mut ReentrantDestroyContext).cast(),
        );
    }
    let native_event = event(generation, 2, 0, 0);

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.destroy_result, RESULT_CALLBACK_IN_FLIGHT);
    assert_eq!(context.nested_dispatch_result, RESULT_INVALID_ARGUMENT);
    assert_eq!(host, original_host);
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(context.calls, 1);

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn wrong_thread_dispatch_unregister_and_destroy_are_rejected() {
    let generation = 42;
    let mut context = RecordingContext::default();
    let mut host = unsafe { create_host(generation) };
    unsafe {
        register_callback(
            host,
            generation,
            record_callback,
            (&mut context as *mut RecordingContext).cast(),
        );
    }
    let host_address = host as usize;

    let (dispatch_result, unregister_result, destroy_result, retained_host) =
        std::thread::spawn(move || {
            let host = host_address as *mut NativeRdpHost;
            let native_event = event(generation, 3, 0, 0);
            let dispatch_result =
                unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) };
            let unregister_result = unsafe { navop_rdp_unregister_event_callback(host) };
            let mut owned = host;
            let destroy_result = unsafe { navop_rdp_destroy(&mut owned) };
            (
                dispatch_result,
                unregister_result,
                destroy_result,
                owned as usize,
            )
        })
        .join()
        .expect("wrong-thread test worker should not panic");

    assert_eq!(dispatch_result, RESULT_WRONG_THREAD);
    assert_eq!(unregister_result, RESULT_WRONG_THREAD);
    assert_eq!(destroy_result, RESULT_WRONG_THREAD);
    assert_eq!(retained_host, host_address);
    assert_eq!(context.calls, 0);

    let native_event = event(generation, 4, 0, 0);
    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, ptr::null()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_credentials_accept_empty_and_separate_server_gateway_secrets() {
    let server = [0x0073, 0x0065, 0x0072, 0x0076, 0x0065, 0x0072];
    let gateway = [0x0067, 0x0061, 0x0074, 0x0065];
    let mut host = unsafe { create_host(42) };
    let cases = [
        credential_bundle(None, None),
        credential_bundle(Some(&server), None),
        credential_bundle(None, Some(&gateway)),
        credential_bundle(Some(&server), Some(&gateway)),
    ];

    for credentials in &cases {
        assert_eq!(
            unsafe { navop_rdp_apply_credentials(host, credentials) },
            RESULT_OK
        );
    }

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_credentials_reject_invalid_layout_and_borrowed_secrets() {
    let server = [0x0073, 0x0065, 0x0072, 0x0076, 0x0065, 0x0072];
    let mut host = unsafe { create_host(42) };
    let valid = credential_bundle(Some(&server), None);

    assert_eq!(
        unsafe { navop_rdp_apply_credentials(ptr::null_mut(), &valid) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );

    let mut short = credential_bundle(Some(&server), None);
    short.struct_size -= 1;
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &short) },
        RESULT_INVALID_ARGUMENT
    );

    let mut wrong_abi = credential_bundle(Some(&server), None);
    wrong_abi.abi_version += 1;
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &wrong_abi) },
        RESULT_ABI_MISMATCH
    );

    let mut flags = credential_bundle(Some(&server), None);
    flags.flags = 1;
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &flags) },
        RESULT_INVALID_ARGUMENT
    );

    let mut null_server = credential_bundle(Some(&server), None);
    null_server.server_password = NavopRdpBorrowedSecret {
        data: ptr::null(),
        len: 1,
    };
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &null_server) },
        RESULT_INVALID_ARGUMENT
    );

    let mut null_gateway = credential_bundle(Some(&server), None);
    null_gateway.gateway_password = NavopRdpBorrowedSecret {
        data: ptr::null(),
        len: 1,
    };
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &null_gateway) },
        RESULT_INVALID_ARGUMENT
    );

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_credentials_preserve_owner_thread_and_open_gate_rules() {
    let generation = 42;
    let mut host = unsafe { create_host(generation) };
    let host_address = host as usize;

    let wrong_thread_result = std::thread::spawn(move || {
        let host = host_address as *mut NativeRdpHost;
        let credentials = credential_bundle(None, None);
        unsafe { navop_rdp_apply_credentials(host, &credentials) }
    })
    .join()
    .expect("wrong-thread credential test worker should not panic");
    assert_eq!(wrong_thread_result, RESULT_WRONG_THREAD);

    let credentials = credential_bundle(None, None);
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &credentials) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_apply_credentials(host, &credentials) },
        RESULT_INVALID_ARGUMENT
    );

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}

#[test]
fn native_connection_entrypoints_validate_inputs_thread_and_open_gate() {
    let endpoint = [
        b'r' as u16,
        b'd' as u16,
        b'p' as u16,
        b'.' as u16,
        b't' as u16,
        b'e' as u16,
        b's' as u16,
        b't' as u16,
    ];
    let mut host = unsafe { create_host(42) };
    let valid = connection_options(&endpoint);

    assert_eq!(
        unsafe { navop_rdp_connect(ptr::null_mut(), &valid) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_connect(host, ptr::null()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_get_connection_state(ptr::null_mut(), ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_request_close(ptr::null_mut(), ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { navop_rdp_disconnect(ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );

    let mut short = connection_options(&endpoint);
    short.struct_size -= 1;
    assert_eq!(
        unsafe { navop_rdp_connect(host, &short) },
        RESULT_INVALID_ARGUMENT
    );

    let mut wrong_abi = connection_options(&endpoint);
    wrong_abi.abi_version += 1;
    assert_eq!(
        unsafe { navop_rdp_connect(host, &wrong_abi) },
        RESULT_ABI_MISMATCH
    );

    let mut flags = connection_options(&endpoint);
    flags.flags = 1;
    assert_eq!(
        unsafe { navop_rdp_connect(host, &flags) },
        RESULT_INVALID_ARGUMENT
    );

    let mut empty_host = connection_options(&endpoint);
    empty_host.host.len = 0;
    assert_eq!(
        unsafe { navop_rdp_connect(host, &empty_host) },
        RESULT_INVALID_ARGUMENT
    );

    let mut oversized_host = connection_options(&endpoint);
    oversized_host.host.len = 256;
    assert_eq!(
        unsafe { navop_rdp_connect(host, &oversized_host) },
        RESULT_INVALID_ARGUMENT
    );

    let mut null_host = connection_options(&endpoint);
    null_host.host.data = ptr::null();
    assert_eq!(
        unsafe { navop_rdp_connect(host, &null_host) },
        RESULT_INVALID_ARGUMENT
    );

    let embedded_nul_endpoint = [b'r' as u16, 0, b'p' as u16];
    let embedded_nul = connection_options(&embedded_nul_endpoint);
    assert_eq!(
        unsafe { navop_rdp_connect(host, &embedded_nul) },
        RESULT_INVALID_ARGUMENT
    );

    for port in [0, 65_536] {
        let mut invalid = connection_options(&endpoint);
        invalid.port = port;
        assert_eq!(
            unsafe { navop_rdp_connect(host, &invalid) },
            RESULT_INVALID_ARGUMENT
        );
    }

    for (width, height) in [(0, 720), (-1, 720), (1280, 0), (1280, -1)] {
        let mut invalid = connection_options(&endpoint);
        invalid.desktop_width = width;
        invalid.desktop_height = height;
        assert_eq!(
            unsafe { navop_rdp_connect(host, &invalid) },
            RESULT_INVALID_ARGUMENT
        );
    }

    for color_depth in [0, 14, 25, 64] {
        let mut invalid = connection_options(&endpoint);
        invalid.color_depth = color_depth;
        assert_eq!(
            unsafe { navop_rdp_connect(host, &invalid) },
            RESULT_INVALID_ARGUMENT
        );
    }

    assert_eq!(
        unsafe { navop_rdp_connect(host, &valid) },
        RESULT_UNAVAILABLE
    );

    let mut state = u32::MAX;
    assert_eq!(
        unsafe { navop_rdp_get_connection_state(host, &mut state) },
        RESULT_UNAVAILABLE
    );
    assert_eq!(state, 0);
    assert_eq!(
        unsafe { navop_rdp_get_connection_state(host, ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );

    let mut status = u32::MAX;
    assert_eq!(
        unsafe { navop_rdp_request_close(host, &mut status) },
        RESULT_UNAVAILABLE
    );
    assert_eq!(status, 0);
    assert_eq!(
        unsafe { navop_rdp_request_close(host, ptr::null_mut()) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(unsafe { navop_rdp_disconnect(host) }, RESULT_UNAVAILABLE);

    let host_address = host as usize;
    let endpoint_for_thread = endpoint;
    let wrong_thread_results = std::thread::spawn(move || {
        let host = host_address as *mut NativeRdpHost;
        let options = connection_options(&endpoint_for_thread);
        let mut state = u32::MAX;
        let mut status = u32::MAX;
        (
            unsafe { navop_rdp_connect(host, &options) },
            unsafe { navop_rdp_get_connection_state(host, &mut state) },
            state,
            unsafe { navop_rdp_request_close(host, &mut status) },
            status,
            unsafe { navop_rdp_disconnect(host) },
        )
    })
    .join()
    .expect("wrong-thread connection test worker should not panic");
    assert_eq!(
        wrong_thread_results,
        (
            RESULT_WRONG_THREAD,
            RESULT_WRONG_THREAD,
            0,
            RESULT_WRONG_THREAD,
            0,
            RESULT_WRONG_THREAD,
        )
    );

    assert_eq!(
        unsafe { navop_rdp_unregister_event_callback(host) },
        RESULT_OK
    );
    assert_eq!(
        unsafe { navop_rdp_connect(host, &valid) },
        RESULT_INVALID_ARGUMENT
    );
    state = u32::MAX;
    assert_eq!(
        unsafe { navop_rdp_get_connection_state(host, &mut state) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(state, 0);
    status = u32::MAX;
    assert_eq!(
        unsafe { navop_rdp_request_close(host, &mut status) },
        RESULT_INVALID_ARGUMENT
    );
    assert_eq!(status, 0);
    assert_eq!(
        unsafe { navop_rdp_disconnect(host) },
        RESULT_INVALID_ARGUMENT
    );

    assert_eq!(unsafe { navop_rdp_destroy(&mut host) }, RESULT_OK);
    assert!(host.is_null());
}
