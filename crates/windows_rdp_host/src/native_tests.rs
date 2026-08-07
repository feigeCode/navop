use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use crate::ffi::{
    ABI_VERSION, NativeEventCallback, NativeRdpHost, NativeResult, NavopRdpCreateOptions,
    NavopRdpEvent, NavopRdpEventCallbackOptions, RESULT_ABI_MISMATCH, RESULT_CALLBACK_IN_FLIGHT,
    RESULT_INVALID_ARGUMENT, RESULT_OK, RESULT_WRONG_THREAD,
};

unsafe extern "C" {
    fn navop_rdp_create(
        options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult;
    fn navop_rdp_register_event_callback(
        host: *mut NativeRdpHost,
        options: *const NavopRdpEventCallbackOptions,
        callback: Option<NativeEventCallback>,
        callback_context: *mut c_void,
    ) -> NativeResult;
    fn navop_rdp_unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_destroy(host: *mut *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_test_dispatch_event(
        host: *mut NativeRdpHost,
        event: *const NavopRdpEvent,
        payload: *const u8,
    ) -> NativeResult;
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
