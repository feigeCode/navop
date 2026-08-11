use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use crate::capabilities::WindowsRdpHostCapabilities;
use crate::credential::WindowsRdpCredentialBundle;
use crate::error::{WindowsRdpHostError, WindowsRdpHresult, check_native_result};
use crate::event::{EventBridge, WindowsRdpRawEvent, native_event_callback};
use crate::ffi::{
    CONNECTION_STATE_CONNECTED, CONNECTION_STATE_CONNECTING, CONNECTION_STATE_DISCONNECTED,
    CREATE_STAGE_NONE, NATIVE_BINDINGS, NativeBindings, NativeRdpHost, NativeResult,
    NavopRdpBounds, NavopRdpCreateOptions, NavopRdpCreateWithParentOptions,
    NavopRdpEventCallbackOptions, NavopRdpLastError, NavopRdpProbeOptions, NavopRdpProbeResult,
    REQUEST_CLOSE_CAN_PROCEED, REQUEST_CLOSE_WAIT_FOR_EVENTS, RESULT_OK,
};
use crate::lifecycle::WindowsRdpHostLifecycle;
use crate::options::{WindowsRdpConnectionOptions, WindowsRdpHostOptions, WindowsRdpParentWindow};

/// Connection state reported synchronously by the native RDP control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpConnectionState {
    Disconnected,
    Connected,
    Connecting,
}

/// Result of asking the native RDP control to begin graceful shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpRequestCloseStatus {
    CanProceed,
    WaitForEvents,
}

/// Owns one opaque native RDP host handle.
///
/// The host is intentionally thread-affine in preparation for its future
/// COM/ActiveX ownership. It does not expose native pointers to callers.
pub struct WindowsRdpHost {
    raw: *mut NativeRdpHost,
    generation: u64,
    lifecycle: WindowsRdpHostLifecycle,
    event_bridge: Option<Box<EventBridge>>,
    callback_registered: bool,
    bindings: NativeBindings,
    owner_thread: std::thread::ThreadId,
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

    /// Creates a hidden, zero-sized native ActiveX child below `parent`.
    ///
    /// # Safety
    ///
    /// The parent handle must be a valid caller-owned window on the current
    /// owner/UI thread and must remain valid until this host has been
    /// successfully closed or dropped. The host owns only its child window and
    /// never destroys or otherwise takes ownership of `parent`.
    pub unsafe fn create_with_parent(
        parent: WindowsRdpParentWindow,
        options: WindowsRdpHostOptions,
    ) -> Result<Self, WindowsRdpHostError> {
        Self::create_with_parent_with(parent, options, NATIVE_BINDINGS)
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Drains owned native events in FIFO order.
    ///
    /// The queue is destructive. Events received for another generation, or
    /// after closing begins, are never returned.
    pub fn drain_events(&self) -> Vec<WindowsRdpRawEvent> {
        self.event_bridge
            .as_ref()
            .map_or_else(Vec::new, |event_bridge| event_bridge.drain())
    }

    /// Returns the Rust facade's current ownership and callback-admission state.
    pub const fn lifecycle(&self) -> WindowsRdpHostLifecycle {
        self.lifecycle
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.lifecycle, WindowsRdpHostLifecycle::Closed)
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
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let native_credentials = credentials.as_native()?;
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - native_credentials borrows only from credentials for this call.
        // - the native contract does not retain either borrowed pointer after
        //   returning from the synchronous ABI entrypoint.
        let result = unsafe { (self.bindings.apply_credentials)(self.raw, &native_credentials) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Applies the minimal endpoint/display configuration and starts an RDP
    /// connection.
    ///
    /// The UTF-16 server name is borrowed only for the synchronous native call
    /// and is not retained by the native host.
    pub fn connect(
        &mut self,
        options: &WindowsRdpConnectionOptions,
    ) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let native_options = options.as_native()?;
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - native_options owns the UTF-16 storage borrowed by its ABI struct
        //   for the full synchronous call.
        // - the native contract retains neither the struct nor its host pointer.
        let result = unsafe { (self.bindings.connect)(self.raw, &native_options.native) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Returns the current native RDP connection state.
    pub fn connection_state(&mut self) -> Result<WindowsRdpConnectionState, WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let mut state = CONNECTION_STATE_DISCONNECTED;
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - state is a live writable out-parameter for the synchronous call.
        let result = unsafe { (self.bindings.get_connection_state)(self.raw, &mut state) };
        check_host_result(self.bindings, self.raw, result)?;
        match state {
            CONNECTION_STATE_DISCONNECTED => Ok(WindowsRdpConnectionState::Disconnected),
            CONNECTION_STATE_CONNECTED => Ok(WindowsRdpConnectionState::Connected),
            CONNECTION_STATE_CONNECTING => Ok(WindowsRdpConnectionState::Connecting),
            _ => Err(WindowsRdpHostError::InvalidNativeResponse),
        }
    }

    /// Requests graceful native control shutdown without forcing a disconnect.
    pub fn request_close(&mut self) -> Result<WindowsRdpRequestCloseStatus, WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let mut status = REQUEST_CLOSE_CAN_PROCEED;
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - status is a live writable out-parameter for the synchronous call.
        let result = unsafe { (self.bindings.request_close)(self.raw, &mut status) };
        check_host_result(self.bindings, self.raw, result)?;
        match status {
            REQUEST_CLOSE_CAN_PROCEED => Ok(WindowsRdpRequestCloseStatus::CanProceed),
            REQUEST_CLOSE_WAIT_FOR_EVENTS => Ok(WindowsRdpRequestCloseStatus::WaitForEvents),
            _ => Err(WindowsRdpHostError::InvalidNativeResponse),
        }
    }

    /// Forces the native RDP control to disconnect.
    ///
    /// The native boundary treats an already-disconnected control as success.
    pub fn disconnect(&mut self) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        // SAFETY: WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        let result = unsafe { (self.bindings.disconnect)(self.raw) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Sets the ActiveX child bounds in physical pixels relative to the
    /// caller-owned parent client area.
    ///
    /// `x` and `y` may be negative. `width` and `height` must be non-negative,
    /// and zero-sized bounds are valid. This operation is owner-thread
    /// serialized and does not change visibility or activation state.
    pub fn set_bounds(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }
        if width < 0 || height < 0 {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        let bounds = NavopRdpBounds::new(x, y, width, height);
        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - bounds is a current-layout stack value live for the synchronous call.
        // - the native boundary does not retain the borrowed bounds pointer.
        let result = unsafe { (self.bindings.set_bounds)(self.raw, &bounds) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Shows or hides the ActiveX child without activating it.
    ///
    /// Hiding attempts to return focus toward the caller-owned parent when focus is
    /// currently inside the child subtree. This operation is owner-thread
    /// serialized and does not change the Rust lifecycle.
    pub fn set_visible(&mut self, visible: bool) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        // - the C ABI uses an explicit 0/1 integer rather than C++ bool.
        let result = unsafe { (self.bindings.set_visible)(self.raw, u32::from(visible)) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Gives keyboard focus to the visible ActiveX child.
    ///
    /// The native control may transfer focus to a descendant window. This
    /// operation is owner-thread serialized and does not change the lifecycle.
    pub fn focus(&mut self) -> Result<(), WindowsRdpHostError> {
        if !matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) || self.raw.is_null() {
            return Err(WindowsRdpHostError::InvalidArgument);
        }

        // SAFETY:
        // - WindowsRdpHost is !Send + !Sync and owner-thread serialized.
        let result = unsafe { (self.bindings.focus)(self.raw) };
        check_host_result(self.bindings, self.raw, result)
    }

    /// Stops callbacks and destroys the native handle. Repeated calls are safe.
    pub fn close(&mut self) -> Result<(), WindowsRdpHostError> {
        if matches!(self.lifecycle, WindowsRdpHostLifecycle::Closed) {
            return Ok(());
        }

        if matches!(self.lifecycle, WindowsRdpHostLifecycle::Open) {
            self.lifecycle = WindowsRdpHostLifecycle::Closing;
            if let Some(event_bridge) = self.event_bridge.as_ref() {
                event_bridge.begin_closing();
            }
        }

        if self.callback_registered {
            // SAFETY: WindowsRdpHost is !Send + !Sync, so lifecycle calls stay
            // on its owner thread. The native ABI guarantees that successful unregistration
            // retains no callback/context and leaves no callback in flight,
            // which keeps EventBridge alive for the full callback lifetime and
            // makes it safe to release after this call.
            let result = unsafe { (self.bindings.unregister_event_callback)(self.raw) };
            check_host_result(self.bindings, self.raw, result)?;
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
        check_host_result(self.bindings, self.raw, result)?;
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
        self.lifecycle = WindowsRdpHostLifecycle::Closed;
    }

    fn probe_with(
        bindings: NativeBindings,
    ) -> Result<WindowsRdpHostCapabilities, WindowsRdpHostError> {
        let options = NavopRdpProbeOptions::current();
        let mut result = NavopRdpProbeResult::current();
        // SAFETY: options and result are current-layout stack values that stay
        // live for the entire synchronous call. The probe contract only reads
        // options, writes within result's caller-provided size, and retains
        // neither pointer after returning.
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
        // SAFETY: native_options and raw's out-pointer remain valid for the
        // synchronous call and this owner-thread facade is the only caller that
        // can receive the result. A successful create transfers one opaque
        // handle into raw; all error paths leave raw null by ABI contract.
        let result = unsafe { (bindings.create)(&native_options, &mut raw) };
        check_native_result(result)?;
        Self::finish_create(options, bindings, raw)
    }

    fn create_with_parent_with(
        parent: WindowsRdpParentWindow,
        options: WindowsRdpHostOptions,
        bindings: NativeBindings,
    ) -> Result<Self, WindowsRdpHostError> {
        if parent.as_raw() == 0 {
            return Err(WindowsRdpHostError::InvalidArgument);
        }
        let native_options =
            NavopRdpCreateWithParentOptions::current(options.generation(), parent.as_raw());
        let mut raw = ptr::null_mut();
        let mut diagnostic = NavopRdpLastError::current();
        // SAFETY: native_options and raw's out-pointer remain valid for the
        // synchronous call. diagnostic is a current-layout writable out value.
        // The caller's safety contract keeps the borrowed parent window alive
        // for the returned host's full lifetime.
        let result =
            unsafe { (bindings.create_with_parent_v2)(&native_options, &mut raw, &mut diagnostic) };
        check_native_diagnostic(result, &diagnostic)?;
        Self::finish_create(options, bindings, raw)
    }

    fn finish_create(
        options: WindowsRdpHostOptions,
        bindings: NativeBindings,
        mut raw: *mut NativeRdpHost,
    ) -> Result<Self, WindowsRdpHostError> {
        if raw.is_null() {
            return Err(WindowsRdpHostError::NativeReturnedNullHandle);
        }

        let event_bridge = Box::new(EventBridge::new(options.generation()));
        let callback_options = NavopRdpEventCallbackOptions::current(options.generation());
        let callback_context = event_bridge.as_ref() as *const EventBridge as *mut c_void;
        // SAFETY: EventBridge has a stable Box allocation and remains alive
        // while native retains callback_context. Registration is owner-thread
        // serialized. The native callback may borrow that context only during
        // synchronous dispatch. On success, close() releases the callback only
        // after the native quiescence guarantee; on failure, the ABI guarantees
        // that the callback and context were not retained.
        let registration_result = unsafe {
            (bindings.register_event_callback)(
                raw,
                &callback_options,
                Some(native_event_callback),
                callback_context,
            )
        };
        if let Err(error) = check_host_result(bindings, raw, registration_result) {
            event_bridge.begin_closing();
            // SAFETY: failed registration atomically retains neither callback
            // nor context, so EventBridge can be dropped after this best-effort
            // owner-thread cleanup. raw still denotes the sole opaque native
            // allocation and remains valid for destroy. If destroy fails or
            // violates its success clear contract, no Rust owner can safely
            // reclaim the allocation, so it is conservatively leaked rather
            // than hiding the original registration error or risking a second
            // release through an uncertain handle state.
            let _ = unsafe { (bindings.destroy)(&mut raw) };
            return Err(error);
        }

        Ok(Self {
            raw,
            generation: options.generation(),
            lifecycle: WindowsRdpHostLifecycle::Open,
            event_bridge: Some(event_bridge),
            callback_registered: true,
            bindings,
            owner_thread: std::thread::current().id(),
            _thread_affinity: PhantomData,
        })
    }
}

fn check_native_diagnostic(
    result: NativeResult,
    diagnostic: &NavopRdpLastError,
) -> Result<(), WindowsRdpHostError> {
    if result == RESULT_OK {
        return Ok(());
    }
    if !diagnostic.has_current_layout() || diagnostic.result != result {
        return check_native_result(result);
    }
    if diagnostic.stage != CREATE_STAGE_NONE || diagnostic.has_win32_code == 1 {
        return Err(WindowsRdpHostError::NativeDiagnostic {
            result,
            stage: diagnostic.stage,
            hresult: if diagnostic.has_hresult == 1 {
                Some(WindowsRdpHresult::from_code(diagnostic.hresult))
            } else {
                None
            },
            win32_code: if diagnostic.has_win32_code == 1 {
                Some(diagnostic.win32_code)
            } else {
                None
            },
        });
    }
    if diagnostic.has_hresult == 1 {
        return Err(WindowsRdpHostError::NativeHresult {
            result,
            hresult: WindowsRdpHresult::from_code(diagnostic.hresult),
        });
    }
    check_native_result(result)
}

fn check_host_result(
    bindings: NativeBindings,
    raw: *mut NativeRdpHost,
    result: NativeResult,
) -> Result<(), WindowsRdpHostError> {
    if result == RESULT_OK {
        return Ok(());
    }
    if raw.is_null() {
        return check_native_result(result);
    }

    let mut diagnostic = NavopRdpLastError::current();
    // SAFETY: raw is the caller-owned live native handle on failure paths that
    // retain ownership. diagnostic is a current-layout writable out value.
    let read_result = unsafe { (bindings.get_last_error)(raw, &mut diagnostic) };
    if read_result != RESULT_OK {
        return check_native_result(result);
    }
    check_native_diagnostic(result, &diagnostic)
}

impl Drop for WindowsRdpHost {
    fn drop(&mut self) {
        let current_thread = std::thread::current().id();
        if current_thread != self.owner_thread {
            tracing::error!(
                generation = self.generation,
                owner_thread = ?self.owner_thread,
                ?current_thread,
                callback_registered = self.callback_registered,
                "leaking Windows native RDP host after wrong-thread drop"
            );
            if self.callback_registered
                && let Some(event_bridge) = self.event_bridge.take()
            {
                let _ = Box::leak(event_bridge);
            }
            return;
        }

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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ffi::{
        NativeEventCallback, NavopRdpConnectionOptions, NavopRdpCredentialBundle, NavopRdpEvent,
        ProbeFn, RESULT_ALLOCATION_FAILED, RESULT_CALLBACK_IN_FLIGHT, RESULT_INTERNAL_ERROR,
        RESULT_INVALID_ARGUMENT, RESULT_INVALID_STATE,
    };
    use crate::options::WindowsRdpColorDepth;

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
        unregister_results: std::collections::VecDeque<NativeResult>,
        unregister_failures_remaining: usize,
        destroy_failures_remaining: usize,
        nonclearing_destroy_calls_remaining: usize,
        last_diagnostic: Option<NavopRdpLastError>,
        last_error_read_result: NativeResult,
        unregister_calls: usize,
        destroy_calls: usize,
        credential_calls: usize,
        credential_results: std::collections::VecDeque<NativeResult>,
        captured_credentials: Vec<(Vec<u16>, Vec<u16>)>,
        connect_calls: usize,
        connect_results: std::collections::VecDeque<NativeResult>,
        captured_connection_options: Vec<(Vec<u16>, u32, i32, i32, i32)>,
        connection_state_calls: usize,
        connection_state_results: std::collections::VecDeque<NativeResult>,
        connection_states: std::collections::VecDeque<u32>,
        request_close_calls: usize,
        request_close_results: std::collections::VecDeque<NativeResult>,
        request_close_statuses: std::collections::VecDeque<u32>,
        disconnect_calls: usize,
        disconnect_results: std::collections::VecDeque<NativeResult>,
        bounds_calls: usize,
        captured_bounds: Vec<(i32, i32, i32, i32)>,
        bounds_results: std::collections::VecDeque<NativeResult>,
        visible_calls: usize,
        captured_visibility: Vec<bool>,
        visible_results: std::collections::VecDeque<NativeResult>,
        focus_calls: usize,
        focus_results: std::collections::VecDeque<NativeResult>,
        parent_create_calls: usize,
        captured_parent_create: Option<(u64, usize)>,
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
                unregister_results: std::collections::VecDeque::new(),
                unregister_failures_remaining: 0,
                destroy_failures_remaining: 0,
                nonclearing_destroy_calls_remaining: 0,
                last_diagnostic: None,
                last_error_read_result: crate::ffi::RESULT_UNAVAILABLE,
                unregister_calls: 0,
                destroy_calls: 0,
                credential_calls: 0,
                credential_results: std::collections::VecDeque::new(),
                captured_credentials: Vec::new(),
                connect_calls: 0,
                connect_results: std::collections::VecDeque::new(),
                captured_connection_options: Vec::new(),
                connection_state_calls: 0,
                connection_state_results: std::collections::VecDeque::new(),
                connection_states: std::collections::VecDeque::new(),
                request_close_calls: 0,
                request_close_results: std::collections::VecDeque::new(),
                request_close_statuses: std::collections::VecDeque::new(),
                disconnect_calls: 0,
                disconnect_results: std::collections::VecDeque::new(),
                bounds_calls: 0,
                captured_bounds: Vec::new(),
                bounds_results: std::collections::VecDeque::new(),
                visible_calls: 0,
                captured_visibility: Vec::new(),
                visible_results: std::collections::VecDeque::new(),
                focus_calls: 0,
                focus_results: std::collections::VecDeque::new(),
                parent_create_calls: 0,
                captured_parent_create: None,
                call_order: Vec::new(),
            }
        }
    }

    thread_local! {
        static FAKE_NATIVE_STATE: RefCell<FakeNativeState> =
            RefCell::new(FakeNativeState::default());
    }

    static WRONG_THREAD_UNREGISTER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static WRONG_THREAD_DESTROY_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn reset_fake_state() {
        FAKE_NATIVE_STATE.with(|state| {
            *state.borrow_mut() = FakeNativeState::default();
        });
    }

    fn set_fake_diagnostic(diagnostic: NavopRdpLastError) {
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().last_diagnostic = Some(diagnostic);
        });
    }

    fn set_fake_last_error_read_result(result: NativeResult) {
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().last_error_read_result = result;
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

    fn emit_active_callback(event: FakeEvent) {
        let (callback, context) = FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            (
                state
                    .active_callback
                    .expect("native callback should still be retained"),
                state.active_context,
            )
        });
        assert!(
            !context.is_null(),
            "native callback context must be retained"
        );
        emit_callback(callback, context, event);
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

    fn parent_create_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().parent_create_calls)
    }

    fn captured_parent_create() -> Option<(u64, usize)> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_parent_create)
    }

    fn captured_credentials() -> Vec<(Vec<u16>, Vec<u16>)> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_credentials.clone())
    }

    fn connect_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().connect_calls)
    }

    fn captured_connection_options() -> Vec<(Vec<u16>, u32, i32, i32, i32)> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_connection_options.clone())
    }

    fn connection_state_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().connection_state_calls)
    }

    fn request_close_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().request_close_calls)
    }

    fn disconnect_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().disconnect_calls)
    }

    fn bounds_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().bounds_calls)
    }

    fn captured_bounds() -> Vec<(i32, i32, i32, i32)> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_bounds.clone())
    }

    fn visible_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().visible_calls)
    }

    fn captured_visibility() -> Vec<bool> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().captured_visibility.clone())
    }

    fn focus_calls() -> usize {
        FAKE_NATIVE_STATE.with(|state| state.borrow().focus_calls)
    }

    fn call_order() -> Vec<&'static str> {
        FAKE_NATIVE_STATE.with(|state| state.borrow().call_order.clone())
    }

    fn active_callback_is_cleared() -> bool {
        FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            state.active_callback.is_none() && state.active_context.is_null()
        })
    }

    fn active_callback_is_retained() -> bool {
        FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            state.active_callback.is_some() && !state.active_context.is_null()
        })
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

    unsafe fn fake_create_with_parent(
        options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult {
        if options.is_null() || out_host.is_null() || out_error.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_error = NavopRdpLastError::current();
        }
        let options = unsafe { &*options };
        let generation =
            u64::from(options.generation_low) | (u64::from(options.generation_high) << 32);
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.parent_create_calls += 1;
            state.captured_parent_create = Some((generation, options.parent_hwnd));
        });
        unsafe {
            *out_host = NonNull::<NativeRdpHost>::dangling().as_ptr();
        }
        RESULT_OK
    }

    unsafe fn fake_null_create_with_parent(
        _options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult {
        if out_host.is_null() || out_error.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        unsafe {
            *out_host = ptr::null_mut();
            *out_error = NavopRdpLastError::current();
        }
        RESULT_OK
    }

    unsafe fn fake_failed_create_with_parent(
        _options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult {
        if !out_host.is_null() {
            unsafe {
                *out_host = ptr::null_mut();
            }
        }
        if !out_error.is_null() {
            unsafe {
                *out_error = NavopRdpLastError {
                    result: RESULT_ALLOCATION_FAILED,
                    ..NavopRdpLastError::current()
                };
            }
        }
        RESULT_ALLOCATION_FAILED
    }

    unsafe fn fake_hresult_failed_create_with_parent(
        _options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult {
        if !out_host.is_null() {
            unsafe {
                *out_host = ptr::null_mut();
            }
        }
        if !out_error.is_null() {
            unsafe {
                *out_error = NavopRdpLastError {
                    result: RESULT_ALLOCATION_FAILED,
                    hresult: i32::MIN,
                    has_hresult: 1,
                    ..NavopRdpLastError::current()
                };
            }
        }
        RESULT_ALLOCATION_FAILED
    }

    unsafe fn fake_get_last_error(
        host: *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult {
        if host.is_null() || out_error.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        FAKE_NATIVE_STATE.with(|state| {
            let state = state.borrow();
            if state.last_error_read_result != RESULT_OK {
                return state.last_error_read_result;
            }
            if let Some(diagnostic) = state.last_diagnostic {
                unsafe {
                    *out_error = diagnostic;
                }
            }
            RESULT_OK
        })
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
            if let Some(result) = state.unregister_results.pop_front() {
                if result != RESULT_OK {
                    return Err(result);
                }
            }
            if state.unregister_failures_remaining > 0 {
                state.unregister_failures_remaining -= 1;
                return Err(RESULT_INTERNAL_ERROR);
            }
            let result = (
                state.active_callback,
                state.active_context,
                state.unregister_event.take(),
            );
            state.active_callback = None;
            state.active_context = ptr::null_mut();
            Ok(result)
        });
        let (callback, context, unregister_event) = match unregister_result {
            Ok(result) => result,
            Err(result) => return result,
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

    unsafe fn record_cross_thread_unregister(host: *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        WRONG_THREAD_UNREGISTER_CALLS.fetch_add(1, Ordering::SeqCst);
        RESULT_OK
    }

    unsafe fn record_cross_thread_destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        WRONG_THREAD_DESTROY_CALLS.fetch_add(1, Ordering::SeqCst);
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

    unsafe fn fake_connect(
        host: *mut NativeRdpHost,
        options: *const NavopRdpConnectionOptions,
    ) -> NativeResult {
        if host.is_null() || options.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }

        let result = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.connect_calls += 1;
            state.connect_results.pop_front().unwrap_or(RESULT_OK)
        });
        if result != RESULT_OK {
            return result;
        }

        // SAFETY: the facade passes a current-layout options value whose
        // borrowed UTF-16 host storage remains live for this synchronous call.
        let options = unsafe { &*options };
        let host_name = if options.host.len == 0 {
            Vec::new()
        } else {
            // SAFETY: the non-empty host slice is backed by NativeConnectionOptions.
            unsafe {
                std::slice::from_raw_parts(options.host.data, options.host.len as usize).to_vec()
            }
        };
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().captured_connection_options.push((
                host_name,
                options.port,
                options.desktop_width,
                options.desktop_height,
                options.color_depth,
            ));
        });
        RESULT_OK
    }

    unsafe fn fake_get_connection_state(
        host: *mut NativeRdpHost,
        out_state: *mut u32,
    ) -> NativeResult {
        if host.is_null() || out_state.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        let (result, state) = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.connection_state_calls += 1;
            (
                state
                    .connection_state_results
                    .pop_front()
                    .unwrap_or(RESULT_OK),
                state
                    .connection_states
                    .pop_front()
                    .unwrap_or(CONNECTION_STATE_DISCONNECTED),
            )
        });
        unsafe {
            *out_state = state;
        }
        result
    }

    unsafe fn fake_request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult {
        if host.is_null() || out_status.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        let (result, status) = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.request_close_calls += 1;
            (
                state.request_close_results.pop_front().unwrap_or(RESULT_OK),
                state
                    .request_close_statuses
                    .pop_front()
                    .unwrap_or(REQUEST_CLOSE_CAN_PROCEED),
            )
        });
        unsafe {
            *out_status = status;
        }
        result
    }

    unsafe fn fake_disconnect(host: *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.disconnect_calls += 1;
            state.disconnect_results.pop_front().unwrap_or(RESULT_OK)
        })
    }

    unsafe fn fake_set_bounds(
        host: *mut NativeRdpHost,
        bounds: *const crate::ffi::NavopRdpBounds,
    ) -> NativeResult {
        if host.is_null() || bounds.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        let bounds = unsafe { &*bounds };
        let result = FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.bounds_calls += 1;
            state
                .captured_bounds
                .push((bounds.x, bounds.y, bounds.width, bounds.height));
            state.bounds_results.pop_front().unwrap_or(RESULT_OK)
        });
        result
    }

    unsafe fn fake_set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult {
        if host.is_null() || visible > 1 {
            return RESULT_INVALID_ARGUMENT;
        }
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.visible_calls += 1;
            state.captured_visibility.push(visible == 1);
            state.visible_results.pop_front().unwrap_or(RESULT_OK)
        })
    }

    unsafe fn fake_focus(host: *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.focus_calls += 1;
            state.focus_results.pop_front().unwrap_or(RESULT_OK)
        })
    }

    fn bindings_with_probe(probe: ProbeFn, create: crate::ffi::CreateFn) -> NativeBindings {
        NativeBindings {
            probe,
            create,
            create_with_parent_v2: fake_create_with_parent,
            get_last_error: fake_get_last_error,
            set_bounds: fake_set_bounds,
            set_visible: fake_set_visible,
            focus: fake_focus,
            destroy: fake_destroy,
            register_event_callback: fake_register_event_callback,
            unregister_event_callback: fake_unregister_event_callback,
            apply_credentials: fake_apply_credentials,
            connect: fake_connect,
            get_connection_state: fake_get_connection_state,
            request_close: fake_request_close,
            disconnect: fake_disconnect,
        }
    }

    fn bindings(create: crate::ffi::CreateFn) -> NativeBindings {
        bindings_with_probe(fake_probe, create)
    }

    fn bindings_with_parent(
        create_with_parent_v2: crate::ffi::CreateWithParentV2Fn,
    ) -> NativeBindings {
        NativeBindings {
            create_with_parent_v2,
            ..bindings(fake_create)
        }
    }

    fn connection_options() -> WindowsRdpConnectionOptions {
        WindowsRdpConnectionOptions::new(
            "rdp.example",
            3390,
            1600,
            900,
            WindowsRdpColorDepth::Bpp24,
        )
        .expect("test connection options should be valid")
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
    fn create_with_parent_rejects_a_zero_parent_before_native_call() {
        reset_fake_state();
        let parent = unsafe { WindowsRdpParentWindow::from_raw(0) };

        let result = WindowsRdpHost::create_with_parent_with(
            parent,
            WindowsRdpHostOptions::new(42),
            bindings_with_parent(fake_create_with_parent),
        );

        assert!(matches!(result, Err(WindowsRdpHostError::InvalidArgument)));
        assert_eq!(parent_create_calls(), 0);
    }

    #[test]
    fn create_with_parent_forwards_generation_and_borrowed_parent() {
        reset_fake_state();
        let generation = 0x1122_3344_aabb_ccdd;
        let parent_raw = 0x1234_usize;
        let parent = unsafe { WindowsRdpParentWindow::from_raw(parent_raw) };
        let mut host = WindowsRdpHost::create_with_parent_with(
            parent,
            WindowsRdpHostOptions::new(generation),
            bindings_with_parent(fake_create_with_parent),
        )
        .expect("fake parent create should succeed");

        assert_eq!(captured_parent_create(), Some((generation, parent_raw)));
        host.close().expect("fake host should close");
        assert_eq!(destroy_calls(), 1);
    }

    #[test]
    fn create_with_parent_maps_native_failure_without_registering_callback() {
        reset_fake_state();
        let parent = unsafe { WindowsRdpParentWindow::from_raw(0x1234) };

        let result = WindowsRdpHost::create_with_parent_with(
            parent,
            WindowsRdpHostOptions::default(),
            bindings_with_parent(fake_failed_create_with_parent),
        );

        assert!(matches!(result, Err(WindowsRdpHostError::AllocationFailed)));
        assert_eq!(parent_create_calls(), 0);
        assert_eq!(unregister_calls(), 0);
        assert_eq!(destroy_calls(), 0);
    }

    #[test]
    fn host_operation_hresult_is_preserved_without_changing_lifecycle() {
        reset_fake_state();
        set_fake_last_error_read_result(RESULT_OK);
        set_fake_diagnostic(NavopRdpLastError {
            result: RESULT_INVALID_STATE,
            hresult: i32::MIN,
            has_hresult: 1,
            ..NavopRdpLastError::current()
        });
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .connect_results
                .push_back(RESULT_INVALID_STATE);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::NativeHresult {
                result: RESULT_INVALID_STATE,
                hresult: WindowsRdpHresult::from_code(i32::MIN),
            })
        );
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn host_operation_native_diagnostic_is_preserved_without_changing_lifecycle() {
        reset_fake_state();
        set_fake_last_error_read_result(RESULT_OK);
        set_fake_diagnostic(NavopRdpLastError {
            result: RESULT_INVALID_STATE,
            hresult: i32::MIN,
            has_hresult: 1,
            stage: crate::ffi::CREATE_STAGE_CREATE_WINDOW,
            win32_code: 1407,
            has_win32_code: 1,
            ..NavopRdpLastError::current()
        });
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .connect_results
                .push_back(RESULT_INVALID_STATE);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::NativeDiagnostic {
                result: RESULT_INVALID_STATE,
                stage: crate::ffi::CREATE_STAGE_CREATE_WINDOW,
                hresult: Some(WindowsRdpHresult::from_code(i32::MIN)),
                win32_code: Some(1407),
            })
        );
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn invalid_or_mismatched_diagnostics_fall_back_to_stable_result() {
        reset_fake_state();
        set_fake_last_error_read_result(RESULT_OK);
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().connect_results.extend([
                RESULT_INVALID_STATE,
                RESULT_INVALID_STATE,
                RESULT_INVALID_STATE,
                RESULT_INVALID_STATE,
            ]);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        set_fake_diagnostic(NavopRdpLastError {
            struct_size: 23,
            result: RESULT_INVALID_STATE,
            hresult: 7,
            has_hresult: 1,
            ..NavopRdpLastError::current()
        });
        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::InvalidState)
        );

        set_fake_diagnostic(NavopRdpLastError {
            result: RESULT_INTERNAL_ERROR,
            hresult: 8,
            has_hresult: 1,
            ..NavopRdpLastError::current()
        });
        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::InvalidState)
        );

        set_fake_diagnostic(NavopRdpLastError {
            result: RESULT_INVALID_STATE,
            hresult: 9,
            has_hresult: 0,
            ..NavopRdpLastError::current()
        });
        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::InvalidState)
        );

        set_fake_diagnostic(NavopRdpLastError {
            result: RESULT_INVALID_STATE,
            stage: crate::ffi::CREATE_STAGE_CREATE_WINDOW,
            has_win32_code: 2,
            ..NavopRdpLastError::current()
        });
        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::InvalidState)
        );
    }

    #[test]
    fn diagnostic_read_failure_falls_back_without_hiding_the_native_result() {
        reset_fake_state();
        set_fake_last_error_read_result(RESULT_INTERNAL_ERROR);
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .connect_results
                .push_back(RESULT_INVALID_STATE);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.connect(&connection_options()),
            Err(WindowsRdpHostError::InvalidState)
        );
    }

    #[test]
    fn create_with_parent_hresult_is_checked_before_callback_registration() {
        reset_fake_state();
        let parent = unsafe { WindowsRdpParentWindow::from_raw(0x1234) };

        let result = WindowsRdpHost::create_with_parent_with(
            parent,
            WindowsRdpHostOptions::default(),
            bindings_with_parent(fake_hresult_failed_create_with_parent),
        );

        assert!(matches!(
            result,
            Err(WindowsRdpHostError::NativeHresult {
                result: RESULT_ALLOCATION_FAILED,
                hresult,
            }) if hresult.code() == i32::MIN
        ));
        assert_eq!(parent_create_calls(), 0);
        assert_eq!(unregister_calls(), 0);
        assert_eq!(destroy_calls(), 0);
    }

    #[test]
    fn create_with_parent_rejects_a_successful_null_handle() {
        reset_fake_state();
        let parent = unsafe { WindowsRdpParentWindow::from_raw(0x1234) };

        let result = WindowsRdpHost::create_with_parent_with(
            parent,
            WindowsRdpHostOptions::default(),
            bindings_with_parent(fake_null_create_with_parent),
        );

        assert!(matches!(
            result,
            Err(WindowsRdpHostError::NativeReturnedNullHandle)
        ));
        assert_eq!(destroy_calls(), 0);
    }

    #[test]
    fn close_then_drop_destroys_the_native_handle_once() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.generation(), 42);
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
        assert!(!host.is_closed());
        host.close().expect("first close should succeed");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
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
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(!host.is_closed());
        host.close()
            .expect("non-clearing destroy should be retryable");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
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
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(!host.is_closed());
        host.close().expect("destroy failure should be retryable");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
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
    fn connection_options_are_copied_during_the_synchronous_native_call() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");
        let options = connection_options();

        host.connect(&options)
            .expect("connection options should be forwarded");
        drop(options);

        assert_eq!(connect_calls(), 1);
        assert_eq!(
            captured_connection_options(),
            vec![("rdp.example".encode_utf16().collect(), 3390, 1600, 900, 24,)]
        );
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn connection_failures_map_without_changing_host_lifecycle() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .connect_results
                .extend([RESULT_INVALID_STATE, RESULT_INTERNAL_ERROR]);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");
        let options = connection_options();

        assert_eq!(
            host.connect(&options),
            Err(WindowsRdpHostError::InvalidState)
        );
        assert_eq!(host.connect(&options), Err(WindowsRdpHostError::Internal));
        assert_eq!(connect_calls(), 2);
        assert!(captured_connection_options().is_empty());
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn connection_state_and_request_close_map_only_known_native_values() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.connection_states.extend([
                CONNECTION_STATE_DISCONNECTED,
                CONNECTION_STATE_CONNECTED,
                CONNECTION_STATE_CONNECTING,
                99,
            ]);
            state.request_close_statuses.extend([
                REQUEST_CLOSE_CAN_PROCEED,
                REQUEST_CLOSE_WAIT_FOR_EVENTS,
                99,
            ]);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.connection_state(),
            Ok(WindowsRdpConnectionState::Disconnected)
        );
        assert_eq!(
            host.connection_state(),
            Ok(WindowsRdpConnectionState::Connected)
        );
        assert_eq!(
            host.connection_state(),
            Ok(WindowsRdpConnectionState::Connecting)
        );
        assert_eq!(
            host.connection_state(),
            Err(WindowsRdpHostError::InvalidNativeResponse)
        );
        assert_eq!(
            host.request_close(),
            Ok(WindowsRdpRequestCloseStatus::CanProceed)
        );
        assert_eq!(
            host.request_close(),
            Ok(WindowsRdpRequestCloseStatus::WaitForEvents)
        );
        assert_eq!(
            host.request_close(),
            Err(WindowsRdpHostError::InvalidNativeResponse)
        );
        assert_eq!(connection_state_calls(), 4);
        assert_eq!(request_close_calls(), 3);
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn disconnect_is_forwarded_repeatedly_with_deterministic_results() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().disconnect_results.extend([
                RESULT_OK,
                RESULT_OK,
                RESULT_INTERNAL_ERROR,
            ]);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.disconnect(), Ok(()));
        assert_eq!(host.disconnect(), Ok(()));
        assert_eq!(host.disconnect(), Err(WindowsRdpHostError::Internal));
        assert_eq!(disconnect_calls(), 3);
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn connection_operations_are_rejected_before_native_when_closing_or_closed() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");
        let options = connection_options();

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert_eq!(
            host.connect(&options),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.connection_state(),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.request_close(),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(host.disconnect(), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!(
            (
                connect_calls(),
                connection_state_calls(),
                request_close_calls(),
                disconnect_calls(),
            ),
            (0, 0, 0, 0)
        );

        host.close().expect("closing host should remain retryable");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
        assert_eq!(
            host.connect(&options),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.connection_state(),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.request_close(),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(host.disconnect(), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!(
            (
                connect_calls(),
                connection_state_calls(),
                request_close_calls(),
                disconnect_calls(),
            ),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn presentation_controls_forward_bounds_visibility_and_focus() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        host.set_bounds(-10, 20, 800, 600)
            .expect("bounds should be forwarded");
        host.set_visible(true).expect("show should be forwarded");
        host.focus().expect("focus should be forwarded");
        host.set_visible(false).expect("hide should be forwarded");

        assert_eq!(bounds_calls(), 1);
        assert_eq!(captured_bounds(), vec![(-10, 20, 800, 600)]);
        assert_eq!(visible_calls(), 2);
        assert_eq!(captured_visibility(), vec![true, false]);
        assert_eq!(focus_calls(), 1);
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
    }

    #[test]
    fn negative_presentation_dimensions_are_rejected_before_native_call() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.set_bounds(0, 0, -1, 10),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.set_bounds(0, 0, 10, -1),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(bounds_calls(), 0);
        assert!(captured_bounds().is_empty());
    }

    #[test]
    fn presentation_failures_map_without_changing_lifecycle() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.bounds_results.push_back(RESULT_ALLOCATION_FAILED);
            state.visible_results.push_back(RESULT_INTERNAL_ERROR);
            state.focus_results.push_back(RESULT_INVALID_ARGUMENT);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(
            host.set_bounds(0, 0, 640, 480),
            Err(WindowsRdpHostError::AllocationFailed)
        );
        assert_eq!(host.set_visible(true), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.focus(), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Open);
        assert_eq!(bounds_calls(), 1);
        assert_eq!(visible_calls(), 1);
        assert_eq!(focus_calls(), 1);
    }

    #[test]
    fn presentation_controls_are_rejected_before_native_when_closing_or_closed() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::default(), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert_eq!(
            host.set_bounds(0, 0, 640, 480),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.set_visible(true),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(host.focus(), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!((bounds_calls(), visible_calls(), focus_calls()), (0, 0, 0));

        host.close().expect("closing host should remain retryable");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
        assert_eq!(
            host.set_bounds(0, 0, 640, 480),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            host.set_visible(false),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(host.focus(), Err(WindowsRdpHostError::InvalidArgument));
        assert_eq!((bounds_calls(), visible_calls(), focus_calls()), (0, 0, 0));
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
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(!host.is_closed());
        emit_last_callback(fake_event(42, 9, 90, &[9]));
        assert!(host.drain_events().is_empty());

        host.close()
            .expect("unregister failure should be retryable");

        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
        assert!(host.is_closed());
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 1);
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "unregister", "destroy"]
        );
    }

    #[test]
    fn callback_in_flight_close_is_retryable_without_destroying_or_reopening_event_gate() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state
                .borrow_mut()
                .unregister_results
                .push_back(RESULT_CALLBACK_IN_FLIGHT);
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::CallbackInFlight));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(!host.is_closed());
        assert!(active_callback_is_retained());
        assert_eq!(unregister_calls(), 1);
        assert_eq!(destroy_calls(), 0);
        assert_eq!(call_order(), vec!["register", "unregister"]);

        emit_active_callback(fake_event(42, 9, 90, &[9]));
        assert!(host.drain_events().is_empty());

        host.close()
            .expect("callback-in-flight close should be retryable");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
        assert!(host.is_closed());
        assert!(active_callback_is_cleared());
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 1);
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "unregister", "destroy"]
        );

        host.close().expect("closed host should remain idempotent");
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 1);
    }

    #[test]
    fn close_retries_unregister_then_destroy_failures_without_reopening_callback_gate() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.unregister_failures_remaining = 1;
            state.destroy_failures_remaining = 1;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(active_callback_is_retained());
        assert_eq!(destroy_calls(), 0);
        emit_active_callback(fake_event(42, 9, 90, &[9]));
        assert!(host.drain_events().is_empty());

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        assert!(active_callback_is_cleared());
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 1);

        host.close()
            .expect("destroy failure should remain retryable after unregister");
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closed);
        assert!(host.is_closed());
        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 2);
        assert_eq!(
            call_order(),
            vec!["register", "unregister", "unregister", "destroy", "destroy",]
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
    fn drop_preserves_callback_context_when_unregister_keeps_failing() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().unregister_failures_remaining = 2;
        });
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        assert_eq!(host.lifecycle(), WindowsRdpHostLifecycle::Closing);
        drop(host);

        assert_eq!(unregister_calls(), 2);
        assert_eq!(destroy_calls(), 0);
        emit_active_callback(fake_event(42, 8, 80, &[8]));
        assert_eq!(call_order(), vec!["register", "unregister", "unregister"]);
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
            host.drain_events(),
            vec![crate::event::WindowsRdpRawEvent {
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

        assert!(host.drain_events().is_empty());
    }

    #[test]
    fn current_generation_events_are_drained_in_order() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        emit_last_callback(fake_event(42, 1, 10, &[1]));
        emit_last_callback(fake_event(42, 2, 20, &[2]));

        let events = host.drain_events();
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].kind, events[0].code), (1, 10));
        assert_eq!((events[1].kind, events[1].code), (2, 20));
        assert!(host.drain_events().is_empty());
    }

    #[test]
    fn unknown_kind_raw_code_and_payload_are_retained_unchanged() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        emit_last_callback(fake_event(42, u32::MAX, i32::MIN, &[0xaa, 0x00, 0xff]));

        assert_eq!(
            host.drain_events(),
            vec![crate::event::WindowsRdpRawEvent {
                generation: 42,
                kind: u32::MAX,
                code: i32::MIN,
                payload: vec![0xaa, 0x00, 0xff],
            }]
        );
    }

    #[test]
    fn stale_generation_does_not_block_a_current_event() {
        reset_fake_state();
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        emit_last_callback(fake_event(41, 1, 10, &[1]));
        emit_last_callback(fake_event(42, 2, 20, &[2]));

        let events = host.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].kind, events[0].code), (2, 20));
    }

    #[test]
    fn close_clears_queued_events_before_rejecting_late_callbacks() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");
        emit_last_callback(fake_event(42, 1, 10, &[1]));

        host.close().expect("close should succeed");
        emit_last_callback(fake_event(42, 2, 20, &[2]));

        assert!(host.drain_events().is_empty());
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

        assert!(host.drain_events().is_empty());
    }

    #[test]
    fn callback_after_close_is_dropped() {
        reset_fake_state();
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");
        host.close().expect("close should succeed");

        emit_last_callback(fake_event(42, 4, 40, &[4]));

        assert!(host.drain_events().is_empty());
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

        assert!(host.drain_events().is_empty());
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
    fn drop_on_wrong_thread_never_unregisters_or_destroys() {
        reset_fake_state();
        WRONG_THREAD_UNREGISTER_CALLS.store(0, Ordering::SeqCst);
        WRONG_THREAD_DESTROY_CALLS.store(0, Ordering::SeqCst);
        let cross_thread_bindings = NativeBindings {
            unregister_event_callback: record_cross_thread_unregister,
            destroy: record_cross_thread_destroy,
            ..bindings(fake_create)
        };
        let host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), cross_thread_bindings)
                .expect("fake create should succeed");

        // This intentionally violates the facade's !Send contract to exercise
        // Drop's fail-closed defense against an unsafe ownership escape. The
        // allocation has one owner and is not accessed concurrently.
        let host = Box::into_raw(Box::new(host)) as usize;
        std::thread::spawn(move || unsafe {
            drop(Box::from_raw(host as *mut WindowsRdpHost));
        })
        .join()
        .expect("wrong-thread drop harness should not panic");

        assert_eq!(WRONG_THREAD_UNREGISTER_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(WRONG_THREAD_DESTROY_CALLS.load(Ordering::SeqCst), 0);
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

    #[test]
    fn registration_failure_preserves_original_error_when_destroy_does_not_clear_handle() {
        reset_fake_state();
        FAKE_NATIVE_STATE.with(|state| {
            state.borrow_mut().nonclearing_destroy_calls_remaining = 1;
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
        assert!(active_callback_is_cleared());
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
