use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{align_of, size_of};

pub(crate) const ABI_VERSION: u32 = 1;

pub(crate) type NativeResult = i32;

pub(crate) const RESULT_OK: NativeResult = 0;
pub(crate) const RESULT_INVALID_ARGUMENT: NativeResult = 1;
pub(crate) const RESULT_ABI_MISMATCH: NativeResult = 2;
pub(crate) const RESULT_ALLOCATION_FAILED: NativeResult = 3;
pub(crate) const RESULT_INTERNAL_ERROR: NativeResult = 4;
pub(crate) const RESULT_UNAVAILABLE: NativeResult = 5;
pub(crate) const RESULT_WRONG_THREAD: NativeResult = 6;
pub(crate) const RESULT_CALLBACK_IN_FLIGHT: NativeResult = 7;

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

    assert!(size_of::<NavopRdpCreateOptions>() == 16);
    assert!(align_of::<NavopRdpCreateOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_high) == 12);

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
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NavopRdpBorrowedSecret>() == 16);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 8);
    assert!(size_of::<NavopRdpCredentialBundle>() == 48);
    assert!(align_of::<NavopRdpCredentialBundle>() == 8);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 24);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 40);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 4);
    assert!(size_of::<NavopRdpCredentialBundle>() == 28);
    assert!(align_of::<NavopRdpCredentialBundle>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 16);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 24);
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

#[derive(Clone, Copy)]
pub(crate) struct NativeBindings {
    pub(crate) probe: ProbeFn,
    pub(crate) create: CreateFn,
    pub(crate) destroy: DestroyFn,
    pub(crate) register_event_callback: RegisterEventCallbackFn,
    pub(crate) unregister_event_callback: UnregisterEventCallbackFn,
    pub(crate) apply_credentials: ApplyCredentialsFn,
}

pub(crate) const NATIVE_BINDINGS: NativeBindings = NativeBindings {
    probe,
    create,
    destroy,
    register_event_callback,
    unregister_event_callback,
    apply_credentials,
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
        assert_eq!(size_of::<NativeResult>(), 4);
    }

    #[test]
    fn fixed_width_abi_struct_layout_is_architecture_independent() {
        assert_eq!(size_of::<NavopRdpProbeOptions>(), 8);
        assert_eq!(align_of::<NavopRdpProbeOptions>(), 4);
        assert_eq!(size_of::<NavopRdpProbeResult>(), 16);
        assert_eq!(align_of::<NavopRdpProbeResult>(), 4);
        assert_eq!(size_of::<NavopRdpCreateOptions>(), 16);
        assert_eq!(align_of::<NavopRdpCreateOptions>(), 4);
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
}
