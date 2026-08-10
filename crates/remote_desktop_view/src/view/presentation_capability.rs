use std::sync::{Mutex, OnceLock};

use one_core::storage::RemoteDesktopBackendPreference;

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use super::presentation::WindowsNativeRdpProbeFailure;
use super::presentation::{
    RemoteDesktopPlatform, RemoteDesktopPresentationError, RemoteDesktopPresentationSelection,
    WindowsNativeRdpCapability, WindowsNativeRdpUnavailableReason, current_remote_desktop_platform,
    select_remote_desktop_presentation,
};

/// Bump this value whenever the native probe ABI or the cached snapshot
/// semantics change. A changed key invalidates an entry without coupling the
/// cache to a presentation or native-host lifecycle.
const WINDOWS_NATIVE_RDP_PROBE_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsNativeRdpBuildVariant {
    FeatureDisabled,
    UnsupportedPlatform,
    WindowsNative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsNativeRdpCapabilityCacheKey {
    pub(crate) probe_contract_version: u32,
    pub(crate) build_variant: WindowsNativeRdpBuildVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsNativeRdpCapabilityCacheEntry {
    key: WindowsNativeRdpCapabilityCacheKey,
    capability: WindowsNativeRdpCapability,
}

static WINDOWS_NATIVE_RDP_CAPABILITY_CACHE: OnceLock<
    Mutex<Option<WindowsNativeRdpCapabilityCacheEntry>>,
> = OnceLock::new();

pub(crate) fn create_remote_desktop_presentation(
    preference: RemoteDesktopBackendPreference,
) -> Result<RemoteDesktopPresentationSelection, RemoteDesktopPresentationError> {
    let platform = current_remote_desktop_platform();
    let capability = if matches!(platform, RemoteDesktopPlatform::Other)
        || matches!(preference, RemoteDesktopBackendPreference::Canvas)
    {
        // Canvas and non-Windows callers do not need to touch the native probe.
        WindowsNativeRdpCapability::Available
    } else {
        current_windows_native_rdp_capability()
    };

    select_remote_desktop_presentation(platform, preference, capability)
}

pub(crate) fn current_windows_native_rdp_capability() -> WindowsNativeRdpCapability {
    let key = current_windows_native_rdp_capability_cache_key();
    let cache = WINDOWS_NATIVE_RDP_CAPABILITY_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    cached_windows_native_rdp_capability(&mut cache, key, probe_windows_native_rdp_capability)
}

pub(crate) fn cached_windows_native_rdp_capability(
    cache: &mut Option<WindowsNativeRdpCapabilityCacheEntry>,
    key: WindowsNativeRdpCapabilityCacheKey,
    probe: impl FnOnce() -> WindowsNativeRdpCapability,
) -> WindowsNativeRdpCapability {
    if let Some(entry) = cache
        && entry.key == key
    {
        return entry.capability;
    }

    let capability = probe();
    *cache = Some(WindowsNativeRdpCapabilityCacheEntry { key, capability });
    capability
}

#[cfg(not(feature = "windows-native-rdp"))]
const fn current_windows_native_rdp_capability_cache_key() -> WindowsNativeRdpCapabilityCacheKey {
    WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: WINDOWS_NATIVE_RDP_PROBE_CONTRACT_VERSION,
        build_variant: WindowsNativeRdpBuildVariant::FeatureDisabled,
    }
}

#[cfg(all(feature = "windows-native-rdp", not(target_os = "windows")))]
const fn current_windows_native_rdp_capability_cache_key() -> WindowsNativeRdpCapabilityCacheKey {
    WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: WINDOWS_NATIVE_RDP_PROBE_CONTRACT_VERSION,
        build_variant: WindowsNativeRdpBuildVariant::UnsupportedPlatform,
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const fn current_windows_native_rdp_capability_cache_key() -> WindowsNativeRdpCapabilityCacheKey {
    WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: WINDOWS_NATIVE_RDP_PROBE_CONTRACT_VERSION,
        build_variant: WindowsNativeRdpBuildVariant::WindowsNative,
    }
}

#[cfg(not(feature = "windows-native-rdp"))]
const fn probe_windows_native_rdp_capability() -> WindowsNativeRdpCapability {
    WindowsNativeRdpCapability::Unavailable(WindowsNativeRdpUnavailableReason::FeatureDisabled)
}

#[cfg(all(feature = "windows-native-rdp", not(target_os = "windows")))]
const fn probe_windows_native_rdp_capability() -> WindowsNativeRdpCapability {
    WindowsNativeRdpCapability::Unavailable(WindowsNativeRdpUnavailableReason::UnsupportedPlatform)
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn probe_windows_native_rdp_capability() -> WindowsNativeRdpCapability {
    match windows_rdp_host::WindowsRdpHost::probe() {
        Ok(capabilities) if capabilities.is_available() => WindowsNativeRdpCapability::Available,
        Ok(_) | Err(windows_rdp_host::WindowsRdpHostError::Unavailable) => {
            WindowsNativeRdpCapability::Unavailable(
                WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable,
            )
        }
        Err(error) => WindowsNativeRdpCapability::ProbeFailed(map_probe_failure(error)),
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn map_probe_failure(error: windows_rdp_host::WindowsRdpHostError) -> WindowsNativeRdpProbeFailure {
    use windows_rdp_host::WindowsRdpHostError;

    match error {
        WindowsRdpHostError::InvalidArgument => WindowsNativeRdpProbeFailure::InvalidArgument,
        WindowsRdpHostError::AbiMismatch => WindowsNativeRdpProbeFailure::AbiMismatch,
        WindowsRdpHostError::AllocationFailed => WindowsNativeRdpProbeFailure::AllocationFailed,
        WindowsRdpHostError::Internal => WindowsNativeRdpProbeFailure::Internal,
        WindowsRdpHostError::Unavailable => {
            unreachable!("unavailable is handled before probe failure mapping")
        }
        WindowsRdpHostError::WrongThread => WindowsNativeRdpProbeFailure::WrongThread,
        WindowsRdpHostError::CallbackInFlight => WindowsNativeRdpProbeFailure::CallbackInFlight,
        WindowsRdpHostError::InvalidState => WindowsNativeRdpProbeFailure::InvalidState,
        WindowsRdpHostError::NativeReturnedNullHandle => {
            WindowsNativeRdpProbeFailure::NativeReturnedNullHandle
        }
        WindowsRdpHostError::NativeDidNotClearHandle => {
            WindowsNativeRdpProbeFailure::NativeDidNotClearHandle
        }
        WindowsRdpHostError::InvalidNativeResponse => {
            WindowsNativeRdpProbeFailure::InvalidNativeResponse
        }
        WindowsRdpHostError::UnexpectedNativeResult(result) => {
            WindowsNativeRdpProbeFailure::UnexpectedNativeResult(result)
        }
        WindowsRdpHostError::NativeHresult { result, hresult } => {
            WindowsNativeRdpProbeFailure::NativeHresult {
                result,
                hresult: hresult.code(),
            }
        }
    }
}
