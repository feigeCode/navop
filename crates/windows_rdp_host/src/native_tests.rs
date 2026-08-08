use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use crate::ffi::{
    ABI_VERSION, CREATE_WITH_PARENT_ABI_VERSION, NativeEventCallback, NativeRdpHost, NativeResult,
    NavopRdpBorrowedSecret, NavopRdpBounds, NavopRdpCreateOptions, NavopRdpCreateWithParentOptions,
    NavopRdpCredentialBundle, NavopRdpEvent, NavopRdpEventCallbackOptions, RESULT_ABI_MISMATCH,
    RESULT_CALLBACK_IN_FLIGHT, RESULT_INVALID_ARGUMENT, RESULT_OK, RESULT_UNAVAILABLE,
    RESULT_WRONG_THREAD,
};

unsafe extern "C" {
    fn navop_rdp_create(
        options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult;
    fn navop_rdp_create_with_parent(
        options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
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
    fn navop_rdp_destroy(host: *mut *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_test_dispatch_event(
        host: *mut NativeRdpHost,
        event: *const NavopRdpEvent,
        payload: *const u8,
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
    if event.payload_len > 0 {
        context.payload =
            unsafe { std::slice::from_raw_parts(payload, event.payload_len as usize).to_vec() };
    }
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

    let valid = event(generation, 2, 3, 0);
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
    let payload = [1, 2, 3, 4];
    let native_event = event(generation, 7, -9, payload.len() as u32);

    assert_eq!(
        unsafe { navop_rdp_test_dispatch_event(host, &native_event, payload.as_ptr()) },
        RESULT_OK
    );
    assert_eq!(context.calls, 1);
    assert_eq!(context.generation, generation);
    assert_eq!(context.kind, 7);
    assert_eq!(context.code, -9);
    assert_eq!(context.payload, payload);

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
}

unsafe extern "C" fn reentrant_destroy_callback(
    context: *mut c_void,
    _event: *const NavopRdpEvent,
    _payload: *const u8,
) {
    let context = unsafe { &mut *context.cast::<ReentrantDestroyContext>() };
    context.calls += 1;
    context.destroy_result = unsafe { navop_rdp_destroy(context.host_slot) };
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
    assert_eq!(host, original_host);

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
