use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use crate::capabilities::WindowsRdpHostCapabilities;
use crate::credential::WindowsRdpCredentialBundle;
use crate::error::{WindowsRdpHostError, check_native_result};
use crate::event::{EventBridge, native_event_callback};
use crate::ffi::{
    NATIVE_BINDINGS, NativeBindings, NativeRdpHost, NavopRdpCreateOptions,
    NavopRdpEventCallbackOptions, NavopRdpProbeOptions, NavopRdpProbeResult,
};
use crate::options::WindowsRdpHostOptions;

#[derive(Clone, Copy)]
enum HostLifecycle {
    Open,
    Closing,
    Closed,
}

/// Owns one opaque native RDP host handle.
///
/// The host is intentionally thread-affine in preparation for its future
/// COM/ActiveX ownership. It does not expose native pointers to callers.
pub struct WindowsRdpHost {
    raw: *mut NativeRdpHost,
    generation: u64,
    lifecycle: HostLifecycle,
    event_bridge: Option<Box<EventBridge>>,
    callback_registered: bool,
    bindings: NativeBindings,
    _thread_affinity: PhantomData<Rc<()>>,
}

impl WindowsRdpHost {
    /// Probes the versioned native boundary without creating an ActiveX
    /// control or connecting to an RDP server.
    pub fn probe() -> Result<WindowsRdpHostCapabilities, WindowsRdpHostError> {
        Self::probe_with(NATIVE_BINDINGS)
    }

    /// Allocates the opaque native lifecycle handle.
    ///
    /// ActiveX creation, COM initialization, parent native windows, and connection
    /// configuration are deliberately outside this initial ABI slice.
    pub fn create(options: WindowsRdpHostOptions) -> Result<Self, WindowsRdpHostError> {
        Self::create_with(options, NATIVE_BINDINGS)
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.lifecycle, HostLifecycle::Closed)
    }

    /// Applies borrowed UTF-16 credentials during one synchronous native call.
    ///
    /// The native boundary copies each supplied secret into a temporary
    /// zeroizing buffer and must not retain either Rust-owned pointer after
    /// this method returns. Credentials are intentionally not stored on the
    /// host and do not change the host lifecycle.
    pub fn apply_credentials(
        &mut self,
        credentials: &WindowsRdpCredentialBundle,
    ) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, HostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let native_credentials = credentials.as_native()?;
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - native_credentials borrows only from credentials for this call.
        // - the native contract does not retain either borrowed pointer after
        //   returning from the synchronous ABI entrypoint.
        let result = unsafe { (self.bindings.apply_credentials)(self.raw, &native_credentials) };
        check_native_result(result)
    }

    /// Stops callbacks and destroys the native handle. Repeated calls are safe.
    pub fn close(&mut self) -> Result<(), WindowsRdpHostError> {
        if matches!(self.lifecycle, HostLifecycle::Closed) {
            return Ok(());
        }

        if matches!(self.lifecycle, HostLifecycle::Open) {
            self.lifecycle = HostLifecycle::Closing;
            if let Some(event_bridge) = self.event_bridge.as_ref() {
                event_bridge.begin_closing();
            }
        }

        if self.callback_registered {
            // SAFETY: WindowsRdpHost is !Send + !Sync, so lifecycle calls stay
            // on its owner thread. The native ABI guarantees that successful
            // unregistration retains no callback/context and leaves no callback
            // in flight, which keeps EventBridge alive for the full callback
            // lifetime and makes it safe to release after this call.
            let result = unsafe { (self.bindings.unregister_event_callback)(self.raw) };
            check_native_result(result)?;
            self.callback_registered = false;
        }

        if self.raw.is_null() {
            self.finish_closing();
            return Ok(());
        }

        // SAFETY: callback unregistration either succeeded above or was never
        // registered. The opaque handle is still owned by this facade and the
        // native destroy contract accepts its address and clears it on success.
        let result = unsafe { (self.bindings.destroy)(&mut self.raw) };
        check_native_result(result)?;
        if !self.raw.is_null() {
            return Err(WindowsRdpHostError::NativeDidNotClearHandle);
        }
        self.finish_closing();
        Ok(())
    }

    fn finish_closing(&mut self) {
        if let Some(event_bridge) = self.event_bridge.as_ref() {
            event_bridge.mark_closed();
        }
        self.lifecycle = HostLifecycle::Closed;
    }

    fn probe_with(
        bindings: NativeBindings,
    ) -> Result<WindowsRdpHostCapabilities, WindowsRdpHostError> {
        let options = NavopRdpProbeOptions::current();
        let mut result = NavopRdpProbeResult::current();
        let native_result = unsafe { (bindings.probe)(&options, &mut result) };
        check_native_result(native_result)?;
        if !result.has_current_layout() {
            return Err(WindowsRdpHostError::InvalidNativeResponse);
        }

        Ok(WindowsRdpHostCapabilities::new(result.available != 0))
    }

    fn create_with(
        options: WindowsRdpHostOptions,
        bindings: NativeBindings,
    ) -> Result<Self, WindowsRdpHostError> {
        let native_options = NavopRdpCreateOptions::current(options.generation());
        let mut raw = ptr::null_mut();
        // SAFETY: native_options and out_host remain valid for the duration of
        // the synchronous call. A successful create transfers one opaque handle
        // into raw; all error paths leave raw null by ABI contract.
        let result = unsafe { (bindings.create)(&native_options, &mut raw) };
        check_native_result(result)?;
        if raw.is_null() {
            return Err(WindowsRdpHostError::NativeReturnedNullHandle);
        }

        let event_bridge = Box::new(EventBridge::new(options.generation()));
        let callback_options = NavopRdpEventCallbackOptions::current(options.generation());
        let callback_context = event_bridge.as_ref() as *const EventBridge as *mut c_void;
        // SAFETY: EventBridge has a stable Box allocation and remains alive
        // while native retains callback_context. Registration is owner-thread
        // serialized. On success, close() releases the callback only after the
        // native quiescence guarantee; on failure, the ABI guarantees that the
        // callback and context were not retained.
        let registration_result = unsafe {
            (bindings.register_event_callback)(
                raw,
                &callback_options,
                Some(native_event_callback),
                callback_context,
            )
        };
        if let Err(error) = check_native_result(registration_result) {
            event_bridge.begin_closing();
            // SAFETY: failed registration atomically retains neither callback
            // nor context, so EventBridge can be dropped after this best-effort
            // owner-thread cleanup. If destroy fails or violates its success
            // clear contract, the opaque native allocation is intentionally
            // leaked rather than risking an unsafe release or hiding the
            // original registration error.
            let _ = unsafe { (bindings.destroy)(&mut raw) };
            return Err(error);
        }

        Ok(Self {
            raw,
            generation: options.generation(),
            lifecycle: HostLifecycle::Open,
            event_bridge: Some(event_bridge),
            callback_registered: true,
            bindings,
            _thread_affinity: PhantomData,
        })
    }
}

impl Drop for WindowsRdpHost {
    fn drop(&mut self) {
        if self.close().is_err() && self.callback_registered {
            if let Some(event_bridge) = self.event_bridge.take() {
                let _ = Box::leak(event_bridge);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use super::*;
    use crate::ffi::{
        NativeEventCallback, NativeResult, NavopRdpCredentialBundle, NavopRdpEvent, ProbeFn,
        RESULT_ALLOCATION_FAILED, RESULT_INTERNAL_ERROR, RESULT_INVALID_ARGUMENT, RESULT_OK,
    };

    #[derive(Clone)]
    struct FakeEvent {
        generation: u64,
        kind: u32,
        code: i32,
        payload: Vec<u8>,
    }

    struct FakeNativeState {
        active_callback: Option<NativeEventCallback>,
        active_context: *mut c_void,
        last_callback: Option<NativeEventCallback>,
        last_context: *mut c_void,
        synchronous_event: Option<FakeEvent>,
        unregister_event: Option<FakeEvent>,
        unregister_failures_remaining: usize,
        destroy_failures_remaining: usize,
        nonclearing_destroy_calls_remaining: usize,
        unregister_calls: usize,
        destroy_calls: usize,
        credential_calls: usize,
        credential_results: std::collections::VecDeque<NativeResult>,
        captured_credentials: Vec<(Vec<u16>, Vec<u16>)>,
        call_order: Vec<&'static str>,
    }

    impl Default for FakeNativeState {
        fn default() -> Self {
            Self {
                active_callback: None,
                active_context: ptr::null_mut(),
                last_callback: None,
                last_context: ptr::null_mut(),
                synchronous_event: None,
                unregister_event: None,
                unregister_failures_remaining: 0,
                destroy_failures_remaining: 0,
                nonclearing_destroy_calls_remaining: 0,
                unregister_calls: 0,
                destroy_calls: 0,
                credential_calls: 0,
                credential_results: std::collections::VecDeque::new(),
                captured_credentials: Vec::new(),
                call_order: Vec::new(),
            }
        }
    }

    thread_local! {
        static FAKE_NATIVE_STATE: RefCell<FakeNativeState> =
            RefCell::new(FakeNativeState::default());
    }

    fn reset_fake_state() {
        FAKE_NATIVE_STATE.with(|state| {
            *state.borrow_mut() = FakeNativeState::default();
        });
    }

    fn fake_event(generation: u64, kind: u32, code: i32, payload: &[u8]) -> FakeEvent {
        FakeEvent {
            generation,
            kind,
            code,
            payload: payload.to_vec(),
        }
    }

    fn emit_callback(callback: NativeEventCallback, context: *mut c_void, mut event: FakeEvent) {
        let native_event = NavopRdpEvent::current(
            event.generation,
            event.kind,
            event.code,
            event.payload.len() as u32,
        );
        let payload = if event.payload.is_empty() {
            ptr::null()
        } else {
            event.payload.as_ptr()
        };
        unsafe {
            callback(context, &native_event, payload);
        }
        event.payload.fill(0);
    }

    fn emit_last_callback(event: FakeEvent) {
        let (callback, context) = FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            (
                state.last_callback.expect("callback should be registered"),
                state.last_context,
            )
        });
        emit_callback(callback, context, event);
    }

    fn drain_events(host: &WindowsRdpHost) -> Vec<crate::event::OwnedNativeEvent> {
        host.event_bridge
            .as_ref()
            .expect("event bridge should remain owned by the host")
            .drain()
    }

    fn destroy_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().destroy_calls)
    }

    fn unregister_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().unregister_calls)
    }

    fn credential_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().credential_calls)
    }

    fn captured_credentials() -> Vec<(Vec<u16>, Vec<u16>)> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_credentials.clone())
    }

    fn call_order() -> Vec<&'static str> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().call_order.clone())
    }

    unsafe fn fake_probe(
        _options: *const NavopRdpProbeOptions,
        out_result: *mut NavopRdpProbeResult,
    ) -> NativeResult {
        if out_result.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_result = NavopRdpProbeResult::current();
            (*out_result).available = 1;
        }
        RESULT_OK
    }

    unsafe fn fake_create(
        _options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult {
        if out_host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_host = NonNull::<NativeRdpHost>::dangling().as_ptr();
        }
        RESULT_OK
    }

    unsafe fn fake_wrong_abi_probe(
        _options: *const NavopRdpProbeOptions,
        out_result: *mut NavopRdpProbeResult,
    ) -> NativeResult {
        if out_result.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_result = NavopRdpProbeResult::current();
            (*out_result).abi_version += 1;
        }
        RESULT_OK
    }

    unsafe fn fake_null_create(
        _options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult {
        if out_host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_host = ptr::null_mut();
        }
        RESULT_OK
    }

    unsafe fn fake_failed_create(
        _options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult {
        if !out_host.is_null() {
            unsafe {
                *out_host = ptr::null_mut();
            }
        }
        RESULT_ALLOCATION_FAILED
    }

    unsafe fn fake_register_event_callback(
        host: *mut NativeRdpHost,
        options: *const NavopRdpEventCallbackOptions,
        callback: Option<NativeEventCallback>,
        callback_context: *mut c_void,
    ) -> NativeResult {
        if host.is_null() || options.is_null() || callback.is_none() {
            return RESULT_INVALID_ARGUMENT;
        }
        let callback = callback.expect("callback was checked above");
        let generation_low = unsafe { std::ptr::addr_of!((*options).generation_low).read() };
        let generation_high = unsafe { std::ptr::addr_of!((*options).generation_high).read() };
        let generation = u64::from(generation_low) | (u64::from(generation_high) << 32);

        let synchronous_event = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.call_order.push("register");
            state.active_callback = Some(callback);
            state.active_context = callback_context;
            state.last_callback = Some(callback);
            state.last_context = callback_context;
            state.synchronous_event.take()
        });
        if let Some(event) = synchronous_event {
            assert_eq!(event.generation, generation);
            emit_callback(callback, callback_context, event);
        }
        RESULT_OK
    }

    unsafe fn fake_failed_register_event_callback(
        _host: *mut NativeRdpHost,
        _options: *const NavopRdpEventCallbackOptions,
        _callback: Option<NativeEventCallback>,
        _callback_context: *mut c_void,
    ) -> NativeResult {
        FAKE_NATIVE_STATE.with(|state| state.borrow_mut().call_order.push("register"));
        RESULT_INTERNAL_ERROR
    }

    unsafe fn fake_invalid_register_event_callback(
        _host: *mut NativeRdpHost,
        _options: *const NavopRdpEventCallbackOptions,
        _callback: Option<NativeEventCallback>,
        _callback_context: *mut c_void,
    ) -> NativeResult {
        FAKE_NATIVE_STATE.with(|state| state.borrow_mut().call_order.push("register"));
        RESULT_INVALID_ARGUMENT
    }

    unsafe fn fake_unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        let unregister_result = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.call_order.push("unregister");
            state.unregister_calls += 1;
            if state.unregister_failures_remaining > 0 {
                state.unregister_failures_remaining -= 1;
                return None;
            }
            let result = (
                state.active_callback,
                state.active_context,
                state.unregister_event.take(),
            );
            state.active_callback = None;
            state.active_context = ptr::null_mut();
            Some(result)
        });
        let Some((callback, context, unregister_event)) = unregister_result else {
            return RESULT_INTERNAL_ERROR;
        };
        if let (Some(callback), Some(event)) = (callback, unregister_event) {
            emit_callback(callback, context, event);
        }
        RESULT_OK
    }

    unsafe fn fake_destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        let (destroy_result, should_clear) = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.destroy_calls += 1;
            state.call_order.push("destroy");
            if state.destroy_failures_remaining > 0 {
                state.destroy_failures_remaining -= 1;
                return (RESULT_INTERNAL_ERROR, false);
            }
            if state.nonclearing_destroy_calls_remaining > 0 {
                state.nonclearing_destroy_calls_remaining -= 1;
                return (RESULT_OK, false);
            }
            (RESULT_OK, true)
        });
        if destroy_result != RESULT_OK {
            return destroy_result;
        }
        if !should_clear {
            return RESULT_OK;
        }
        unsafe {
            *host = ptr::null_mut();
        }
        RESULT_OK
    }

    unsafe fn fake_apply_credentials(
        host: *mut NativeRdpHost,
        credentials: *const NavopRdpCredentialBundle,
    ) -> NativeResult {
        if host.is_null() || credentials.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }

        let result = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.credential_calls += 1;
            state.credential_results.pop_front().unwrap_or(RESULT_OK)
        });
        if result != RESULT_OK {
            return result;
        }

        // SAFETY: the production facade passes a valid current-layout bundle
        // whose borrowed pointers remain valid for the duration of this fake
        // synchronous call. The fake copies both slices and retains no pointer.
        let credentials = unsafe { &*credentials };
        let server = if credentials.server_password.len == 0 {
            Vec::new()
        } else {
            // SAFETY: non-empty credential slices are backed by the owner
            // bundle's live UTF-16 vectors during this synchronous call.
            unsafe {
                std::slice::from_raw_parts(
                    credentials.server_password.data,
                    credentials.server_password.len as usize,
                )
                .to_vec()
            }
        };
        let gateway = if credentials.gateway_password.len == 0 {
            Vec::new()
        } else {
            // SAFETY: non-empty credential slices are backed by the owner
            // bundle's live UTF-16 vectors during this synchronous call.
            unsafe {
                std::slice::from_raw_parts(
                    credentials.gateway_password.data,
                    credentials.gateway_password.len as usize,
                )
                .to_vec()
            }
        };
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .captured_credentials
                .push((server, gateway));
        });
        RESULT_OK
    }

    fn bindings_with_probe(probe: ProbeFn, create: crate::ffi::CreateFn) -> NativeBindings {
        NativeBindings {
            probe,
            create,
            destroy: fake_destroy,
            register_event_callback: fake_register_event_callback,
            unregister_event_callback: fake_unregister_event_callback,
            apply_credentials: fake_apply_credentials,
        }
    }

    fn bindings(create: crate::ffi::CreateFn) -> NativeBindings {
        bindings_with_probe(fake_probe, create)
    }

    #[test]
    fn native_success_with_a_wrong_probe_abi_is_rejected() {
        reset_fake_state();
        let result =
            WindowsRdpHost::probe_with(bindings_with_probe(fake_wrong_abi_probe, fake_create));

        assert!(matches!(
            result,
            Err(WindowsRdpHostError::InvalidNativeResponse)
        ));
    }

    #[test]
    fn fake_native_create_failure_is_mapped_without_a_handle() {
        reset_fake_state();
        let result = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::default(),
            bindings(fake_failed_create),
        );

        assert!(matches!(result, Err(WindowsRdpHostError::AllocationFailed)));
    }

    #[test]
    fn native_success_with_a_null_handle_is_rejected() {
        reset_fake_state();
        let result = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::default(),
            bindings(fake_null_create),
        );

        assert!(matches!(
            result,
            Err(WindowsRdpHostError::NativeReturnedNullHandle)
        ));
    }

    #[test]
    fn close_then_drop_destroys_the_native_handle_once() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.generation(), 42);
        assert!(!host.is_closed());
        host.close().expect("first close should succeed");
        assert!(host.is_closed());
        assert_eq!(destroy_calls(), 1);

        drop(host);
        assert_eq!(destroy_calls(), 1);
    }

    #[test]
    fn repeated_close_is_a_rust_side_noop() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        host.close().expect("first close should succeed");
        host.close().expect("repeated close should succeed");

        assert!(host.is_closed());
        assert_eq!(destroy_calls(), 1);
    }

    #[test]
    fn close_rejects_native_success_without_handle_clear() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().nonclearing_destroy_calls_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.close(),
            Err(WindowsRdpHostError::NativeDidNotClearHandle)
        );
        assert!(!host.is_closed());
        host.close()
            .expect("non-clearing destroy should be retryable");
        assert!(host.is_closed());
        assert_eq!(destroy_calls(), 2);
    }

    #[test]
    fn close_maps_native_destroy_failure() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().destroy_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert!(!host.is_closed());
        host.close().expect("destroy failure should be retryable");
        assert!(host.is_closed());
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "destroy", "destroy"]
        );
        assert_eq!(unregister_calls(), 1);
        assert_eq!(destroy_calls(), 2);
    }

    #[test]
    fn credentials_are_applied_as_separate_borrowed_utf16_values() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        host.apply_credentials(
            &WindowsRdpCredentialBundle::new().with_server_password("server-only".to_owned()),
        )
        .expect("server-only credentials should apply");
        host.apply_credentials(
            &WindowsRdpCredentialBundle::new().with_gateway_password("gateway-only".to_owned()),
        )
        .expect("Gateway-only credentials should apply");
        let both = WindowsRdpCredentialBundle::new()
            .with_server_password("server-secret".to_owned())
            .with_gateway_password("gateway-secret".to_owned());
        host.apply_credentials(&both)
            .expect("server and Gateway credentials should apply");
        host.apply_credentials(&WindowsRdpCredentialBundle::new())
            .expect("empty credentials should apply");

        assert_eq!(credential_calls(), 4);
        assert_eq!(
            captured_credentials(),
            vec![
                ("server-only".encode_utf16().collect(), Vec::new()),
                (Vec::new(), "gateway-only".encode_utf16().collect()),
                (
                    "server-secret".encode_utf16().collect(),
                    "gateway-secret".encode_utf16().collect(),
                ),
                (Vec::new(), Vec::new()),
            ]
        );
    }

    #[test]
    fn credential_failures_map_without_changing_lifecycle_or_storing_owner() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state
                .credential_results
                .extend([RESULT_ALLOCATION_FAILED, RESULT_INTERNAL_ERROR]);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");
        let credentials = WindowsRdpCredentialBundle::new()
            .with_server_password("allocation-failure".to_owned())
            .with_gateway_password("internal-failure".to_owned());

        assert_eq!(
            host.apply_credentials(&credentials),
            Err(WindowsRdpHostError::AllocationFailed)
        );
        assert_eq!(
            host.apply_credentials(&credentials),
            Err(WindowsRdpHostError::Internal)
        );
        assert!(!host.is_closed());
        assert_eq!(credential_calls(), 2);
        assert!(captured_credentials().is_empty());
        drop(credentials);
        host.close().expect("host should still close normally");
    }

    #[test]
    fn credentials_are_rejected_before_native_call_when_host_is_closing_or_closed() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_failures_remaining = 1;
        });
        let mut closing_host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");
        let credentials =
            WindowsRdpCredentialBundle::new().with_server_password("closing".to_owned());

        assert_eq!(closing_host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(
            closing_host.apply_credentials(&credentials),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(credential_calls(), 0);
        closing_host
            .close()
            .expect("closing host should be retryable");
        assert!(closing_host.is_closed());
        assert_eq!(
            closing_host.apply_credentials(&credentials),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(credential_calls(), 0);
    }

    #[test]
    fn unregister_failure_can_be_retried_without_reopening_the_event_gate() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert!(!host.is_closed());
        emit_last_callback(fake_event(42, 9, 90, &[9]));
        assert!(drain_events(&host).is_empty());

        host.close()
            .expect("unregister failure should be retryable");

        assert!(host.is_closed());
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 1);
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "unregister", "destroy"]
        );
    }

    #[test]
    fn drop_retries_a_prior_explicit_close_failure() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().destroy_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        drop(host);

        assert_eq!(unregister_calls(), 1);
        assert_eq!(destroy_calls(), 2);
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "destroy", "destroy"]
        );
    }

    #[test]
    fn synchronous_registration_callback_queues_an_owned_event() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().synchronous_event = Some(fake_event(42, 7, -9, &[1, 2, 3]));
        });

        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            drain_events(&host),
            vec![crate::event::OwnedNativeEvent {
                generation: 42,
                kind: 7,
                code: -9,
                payload: vec![1, 2, 3],
            }]
        );
    }

    #[test]
    fn stale_generation_event_is_dropped() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        emit_last_callback(fake_event(41, 7, 0, &[1]));

        assert!(drain_events(&host).is_empty());
    }

    #[test]
    fn current_generation_events_are_drained_in_order() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        emit_last_callback(fake_event(42, 1, 10, &[1]));
        emit_last_callback(fake_event(42, 2, 20, &[2]));

        let events = drain_events(&host);
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].kind, events[0].code), (1, 10));
        assert_eq!((events[1].kind, events[1].code), (2, 20));
    }

    #[test]
    fn callback_during_unregister_is_dropped_by_the_closing_gate() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_event = Some(fake_event(42, 3, 30, &[3]));
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        host.close().expect("close should succeed");

        assert!(drain_events(&host).is_empty());
    }

    #[test]
    fn callback_after_close_is_dropped() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");
        host.close().expect("close should succeed");

        emit_last_callback(fake_event(42, 4, 40, &[4]));

        assert!(drain_events(&host).is_empty());
    }

    #[test]
    fn malformed_payload_is_dropped() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");
        let (callback, context) = FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            (
                state.last_callback.expect("callback should be registered"),
                state.last_context,
            )
        });
        let event = NavopRdpEvent::current(42, 5, 50, 1);

        unsafe {
            callback(context, &event, ptr::null());
        }

        assert!(drain_events(&host).is_empty());
    }

    #[test]
    fn unregister_happens_before_destroy() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        host.close().expect("close should succeed");

        assert_eq!(call_order(), vec!["register", "unregister", "destroy"]);
    }

    #[test]
    fn drop_uses_the_same_unregister_before_destroy_path() {
        reset_fake_state();
        {
            let _host =
                WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                    .expect("fake create should succeed");
        }

        assert_eq!(call_order(), vec!["register", "unregister", "destroy"]);
    }

    #[test]
    fn registration_failure_cleans_up_the_native_handle() {
        reset_fake_state();
        let failed_registration_bindings = NativeBindings {
            register_event_callback: fake_failed_register_event_callback,
            ..bindings(fake_create)
        };

        let result = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::new(42),
            failed_registration_bindings,
        );

        assert!(matches!(result, Err(WindowsRdpHostError::Internal)));
        assert_eq!(call_order(), vec!["register", "destroy"]);
        assert_eq!(destroy_calls(), 1);
    }

    #[test]
    fn registration_failure_preserves_the_original_error_when_cleanup_fails() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().destroy_failures_remaining = 1;
        });
        let failed_registration_bindings = NativeBindings {
            register_event_callback: fake_invalid_register_event_callback,
            ..bindings(fake_create)
        };

        let result = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::new(42),
            failed_registration_bindings,
        );

        assert!(matches!(result, Err(WindowsRdpHostError::InvalidArgument)));
        assert_eq!(call_order(), vec!["register", "destroy"]);
        assert_eq!(unregister_calls(), 0);
        assert_eq!(destroy_calls(), 1);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_probe_is_stably_unavailable() {
        reset_fake_state();
        let capabilities = WindowsRdpHost::probe().expect("stub probe should succeed");

        assert!(!capabilities.is_available());
        assert!(matches!(
            WindowsRdpHost::create(WindowsRdpHostOptions::default()),
            Err(WindowsRdpHostError::Unavailable)
        ));
    }
}
