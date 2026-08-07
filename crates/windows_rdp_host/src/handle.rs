use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use crate::capabilities::WindowsRdpHostCapabilities;
use crate::error::{WindowsRdpHostError, check_native_result};
use crate::ffi::{
    NATIVE_BINDINGS, NativeBindings, NativeRdpHost, NavopRdpCreateOptions, NavopRdpProbeOptions,
    NavopRdpProbeResult,
};
use crate::options::WindowsRdpHostOptions;

/// Owns one opaque native RDP host handle.
///
/// The host is intentionally thread-affine in preparation for its future
/// COM/ActiveX ownership. It does not expose native pointers to callers.
pub struct WindowsRdpHost {
    raw: *mut NativeRdpHost,
    generation: u64,
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
        self.raw.is_null()
    }

    /// Destroys the native handle. Repeated calls are safe.
    pub fn close(&mut self) -> Result<(), WindowsRdpHostError> {
        let result = unsafe { (self.bindings.destroy)(&mut self.raw) };
        check_native_result(result)?;
        if !self.raw.is_null() {
            return Err(WindowsRdpHostError::NativeDidNotClearHandle);
        }
        Ok(())
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
        let result = unsafe { (bindings.create)(&native_options, &mut raw) };
        check_native_result(result)?;
        if raw.is_null() {
            return Err(WindowsRdpHostError::NativeReturnedNullHandle);
        }

        Ok(Self {
            raw,
            generation: options.generation(),
            bindings,
            _thread_affinity: PhantomData,
        })
    }
}

impl Drop for WindowsRdpHost {
    fn drop(&mut self) {
        let _ = unsafe { (self.bindings.destroy)(&mut self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ffi::{
        NativeResult, ProbeFn, RESULT_ALLOCATION_FAILED, RESULT_INTERNAL_ERROR,
        RESULT_INVALID_ARGUMENT, RESULT_OK,
    };

    static DESTROY_CALLS: AtomicUsize = AtomicUsize::new(0);

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

    unsafe fn fake_destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        DESTROY_CALLS.fetch_add(1, Ordering::SeqCst);
        unsafe {
            *host = ptr::null_mut();
        }
        RESULT_OK
    }

    unsafe fn fake_nonclearing_destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        RESULT_OK
    }

    unsafe fn fake_failed_destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
        if host.is_null() {
            return RESULT_INVALID_ARGUMENT;
        }
        RESULT_INTERNAL_ERROR
    }

    fn bindings_with_destroy(destroy: crate::ffi::DestroyFn) -> NativeBindings {
        NativeBindings {
            probe: fake_probe,
            create: fake_create,
            destroy,
        }
    }

    fn bindings_with_probe(probe: ProbeFn, create: crate::ffi::CreateFn) -> NativeBindings {
        NativeBindings {
            probe,
            create,
            destroy: fake_destroy,
        }
    }

    fn bindings(create: crate::ffi::CreateFn) -> NativeBindings {
        bindings_with_probe(fake_probe, create)
    }

    #[test]
    fn native_success_with_a_wrong_probe_abi_is_rejected() {
        let result =
            WindowsRdpHost::probe_with(bindings_with_probe(fake_wrong_abi_probe, fake_create));

        assert!(matches!(
            result,
            Err(WindowsRdpHostError::InvalidNativeResponse)
        ));
    }

    #[test]
    fn fake_native_create_failure_is_mapped_without_a_handle() {
        let result = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::default(),
            bindings(fake_failed_create),
        );

        assert!(matches!(result, Err(WindowsRdpHostError::AllocationFailed)));
    }

    #[test]
    fn native_success_with_a_null_handle_is_rejected() {
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
    fn close_and_drop_both_use_the_idempotent_destroy_boundary() {
        DESTROY_CALLS.store(0, Ordering::SeqCst);
        let mut host =
            WindowsRdpHost::create_with(WindowsRdpHostOptions::new(42), bindings(fake_create))
                .expect("fake create should succeed");

        assert_eq!(host.generation(), 42);
        assert!(!host.is_closed());
        host.close().expect("first close should succeed");
        assert!(host.is_closed());
        assert_eq!(DESTROY_CALLS.load(Ordering::SeqCst), 1);

        drop(host);
        assert_eq!(DESTROY_CALLS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn close_rejects_native_success_without_handle_clear() {
        let mut host = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::default(),
            bindings_with_destroy(fake_nonclearing_destroy),
        )
        .expect("fake create should succeed");

        assert_eq!(
            host.close(),
            Err(WindowsRdpHostError::NativeDidNotClearHandle)
        );
        host.raw = ptr::null_mut();
    }

    #[test]
    fn close_maps_native_destroy_failure() {
        let mut host = WindowsRdpHost::create_with(
            WindowsRdpHostOptions::default(),
            bindings_with_destroy(fake_failed_destroy),
        )
        .expect("fake create should succeed");

        assert_eq!(host.close(), Err(WindowsRdpHostError::Internal));
        host.raw = ptr::null_mut();
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_probe_is_stably_unavailable() {
        let capabilities = WindowsRdpHost::probe().expect("stub probe should succeed");

        assert!(!capabilities.is_available());
        assert!(matches!(
            WindowsRdpHost::create(WindowsRdpHostOptions::default()),
            Err(WindowsRdpHostError::Unavailable)
        ));
    }
}
