use std::marker::PhantomData;
use std::mem::size_of;

pub(crate) const ABI_VERSION: u32 = 1;

pub(crate) type NativeResult = i32;

pub(crate) const RESULT_OK: NativeResult = 0;
pub(crate) const RESULT_INVALID_ARGUMENT: NativeResult = 1;
pub(crate) const RESULT_ABI_MISMATCH: NativeResult = 2;
pub(crate) const RESULT_ALLOCATION_FAILED: NativeResult = 3;
pub(crate) const RESULT_INTERNAL_ERROR: NativeResult = 4;
pub(crate) const RESULT_UNAVAILABLE: NativeResult = 5;

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

pub(crate) type ProbeFn = unsafe fn(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult;
pub(crate) type CreateFn = unsafe fn(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult;
pub(crate) type DestroyFn = unsafe fn(host: *mut *mut NativeRdpHost) -> NativeResult;

#[derive(Clone, Copy)]
pub(crate) struct NativeBindings {
    pub(crate) probe: ProbeFn,
    pub(crate) create: CreateFn,
    pub(crate) destroy: DestroyFn,
}

pub(crate) const NATIVE_BINDINGS: NativeBindings = NativeBindings {
    probe,
    create,
    destroy,
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
        assert_eq!(size_of::<NativeResult>(), 4);
    }

    #[test]
    fn abi_struct_layout_is_architecture_independent() {
        assert_eq!(size_of::<NavopRdpProbeOptions>(), 8);
        assert_eq!(align_of::<NavopRdpProbeOptions>(), 4);
        assert_eq!(size_of::<NavopRdpProbeResult>(), 16);
        assert_eq!(align_of::<NavopRdpProbeResult>(), 4);
        assert_eq!(size_of::<NavopRdpCreateOptions>(), 16);
        assert_eq!(align_of::<NavopRdpCreateOptions>(), 4);
    }

    #[test]
    fn create_options_split_the_generation_without_abi_alignment_risk() {
        let options = NavopRdpCreateOptions::current(0x1122_3344_aabb_ccdd);

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
}
