use std::cell::RefCell;

use one_core::storage::RemoteDesktopBackendPreference;

#[cfg(feature = "windows-native-rdp")]
use super::presentation::classify_windows_native_create_error;
use super::presentation::{
    RemoteDesktopPlatform, RemoteDesktopPresentation, RemoteDesktopPresentationCreateError,
    RemoteDesktopPresentationCreation, RemoteDesktopPresentationError,
    RemoteDesktopPresentationInitialization, RemoteDesktopPresentationSelection,
    RemoteDesktopPresentationState, WindowsNativeRdpCapability, WindowsNativeRdpProbeFailure,
    WindowsNativeRdpUnavailableReason, create_remote_desktop_presentation_with,
    select_remote_desktop_presentation,
};
use super::presentation_capability::{
    WindowsNativeRdpBuildVariant, WindowsNativeRdpCapabilityCacheEntry,
    WindowsNativeRdpCapabilityCacheKey, cached_windows_native_rdp_capability,
    create_remote_desktop_presentation, current_windows_native_rdp_capability,
};

const fn selection(
    presentation: RemoteDesktopPresentation,
    fallback_reason: Option<WindowsNativeRdpUnavailableReason>,
) -> RemoteDesktopPresentationSelection {
    RemoteDesktopPresentationSelection {
        presentation,
        fallback_reason,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeCreateFailure {
    ClassNotRegistered,
    RequiredInterfaceMissing,
    Unexpected,
}

const fn classify_test_create_failure(
    failure: &NativeCreateFailure,
) -> Option<WindowsNativeRdpUnavailableReason> {
    match failure {
        NativeCreateFailure::ClassNotRegistered => {
            Some(WindowsNativeRdpUnavailableReason::ClassNotRegistered)
        }
        NativeCreateFailure::RequiredInterfaceMissing => {
            Some(WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing)
        }
        NativeCreateFailure::Unexpected => None,
    }
}

#[test]
fn presentation_factory_canvas_does_not_probe_or_create() {
    let calls = RefCell::new(Vec::new());

    let result = create_remote_desktop_presentation_with(
        RemoteDesktopPlatform::Windows,
        RemoteDesktopBackendPreference::Canvas,
        || {
            calls.borrow_mut().push("probe");
            WindowsNativeRdpCapability::Available
        },
        || {
            calls.borrow_mut().push("create");
            Ok::<_, NativeCreateFailure>("native")
        },
        classify_test_create_failure,
    );

    assert_eq!(
        Ok(RemoteDesktopPresentationCreation::Canvas {
            fallback_reason: None
        }),
        result
    );
    assert!(calls.into_inner().is_empty());
}

#[test]
fn presentation_factory_non_windows_does_not_probe_or_create() {
    for preference in [
        RemoteDesktopBackendPreference::Auto,
        RemoteDesktopBackendPreference::WindowsNative,
        RemoteDesktopBackendPreference::Canvas,
    ] {
        let calls = RefCell::new(Vec::new());
        let result = create_remote_desktop_presentation_with(
            RemoteDesktopPlatform::Other,
            preference,
            || {
                calls.borrow_mut().push("probe");
                WindowsNativeRdpCapability::Available
            },
            || {
                calls.borrow_mut().push("create");
                Ok::<_, NativeCreateFailure>("native")
            },
            classify_test_create_failure,
        );

        assert_eq!(
            Ok(RemoteDesktopPresentationCreation::Canvas {
                fallback_reason: None
            }),
            result
        );
        assert!(calls.into_inner().is_empty(), "{preference:?}");
    }
}

#[test]
fn presentation_factory_auto_available_creates_native_once_in_order() {
    let calls = RefCell::new(Vec::new());

    let result = create_remote_desktop_presentation_with(
        RemoteDesktopPlatform::Windows,
        RemoteDesktopBackendPreference::Auto,
        || {
            calls.borrow_mut().push("probe");
            WindowsNativeRdpCapability::Available
        },
        || {
            calls.borrow_mut().push("create");
            Ok::<_, NativeCreateFailure>("native")
        },
        classify_test_create_failure,
    );

    assert_eq!(
        Ok(RemoteDesktopPresentationCreation::Native("native")),
        result
    );
    assert_eq!(vec!["probe", "create"], calls.into_inner());
}

#[test]
fn presentation_factory_auto_falls_back_for_known_create_unavailability() {
    for (failure, reason) in [
        (
            NativeCreateFailure::ClassNotRegistered,
            WindowsNativeRdpUnavailableReason::ClassNotRegistered,
        ),
        (
            NativeCreateFailure::RequiredInterfaceMissing,
            WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing,
        ),
    ] {
        let calls = RefCell::new(Vec::new());
        let result = create_remote_desktop_presentation_with(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::Auto,
            || {
                calls.borrow_mut().push("probe");
                WindowsNativeRdpCapability::Available
            },
            || {
                calls.borrow_mut().push("create");
                Err::<&str, _>(failure)
            },
            classify_test_create_failure,
        );

        assert_eq!(
            Ok(RemoteDesktopPresentationCreation::Canvas {
                fallback_reason: Some(reason)
            }),
            result
        );
        assert_eq!(vec!["probe", "create"], calls.into_inner());
    }
}

#[test]
fn presentation_factory_auto_preserves_unexpected_create_error() {
    let calls = RefCell::new(Vec::new());

    let result = create_remote_desktop_presentation_with(
        RemoteDesktopPlatform::Windows,
        RemoteDesktopBackendPreference::Auto,
        || {
            calls.borrow_mut().push("probe");
            WindowsNativeRdpCapability::Available
        },
        || {
            calls.borrow_mut().push("create");
            Err::<&str, _>(NativeCreateFailure::Unexpected)
        },
        classify_test_create_failure,
    );

    assert_eq!(
        Err(RemoteDesktopPresentationCreateError::NativeCreate(
            NativeCreateFailure::Unexpected
        )),
        result
    );
    assert_eq!(vec!["probe", "create"], calls.into_inner());
}

#[test]
fn presentation_factory_explicit_native_never_falls_back_after_create_failure() {
    for failure in [
        NativeCreateFailure::ClassNotRegistered,
        NativeCreateFailure::RequiredInterfaceMissing,
        NativeCreateFailure::Unexpected,
    ] {
        let result = create_remote_desktop_presentation_with(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::WindowsNative,
            || WindowsNativeRdpCapability::Available,
            || Err::<&str, _>(failure),
            classify_test_create_failure,
        );

        assert_eq!(
            Err(RemoteDesktopPresentationCreateError::NativeCreate(failure)),
            result
        );
    }
}

#[test]
fn presentation_runtime_only_canvas_allows_canvas_backend() {
    assert!(
        RemoteDesktopPresentationInitialization::Canvas {
            fallback_reason: None,
        }
        .allows_canvas_runtime()
    );
    assert!(
        RemoteDesktopPresentationInitialization::Canvas {
            fallback_reason: Some(WindowsNativeRdpUnavailableReason::FeatureDisabled),
        }
        .allows_canvas_runtime()
    );
    assert!(!RemoteDesktopPresentationInitialization::Pending.allows_canvas_runtime());
    assert!(!RemoteDesktopPresentationInitialization::Native.allows_canvas_runtime());
    assert!(!RemoteDesktopPresentationInitialization::Failed.allows_canvas_runtime());
}

#[test]
fn windows_auto_uses_native_when_available() {
    assert_eq!(
        Ok(selection(RemoteDesktopPresentation::NativeWindows, None)),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::Auto,
            WindowsNativeRdpCapability::Available,
        )
    );
}

#[test]
fn windows_auto_falls_back_to_canvas_only_for_pre_connect_unavailability() {
    for reason in [
        WindowsNativeRdpUnavailableReason::FeatureDisabled,
        WindowsNativeRdpUnavailableReason::UnsupportedPlatform,
        WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable,
        WindowsNativeRdpUnavailableReason::ClassNotRegistered,
        WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing,
    ] {
        assert_eq!(
            Ok(selection(RemoteDesktopPresentation::Canvas, Some(reason))),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Windows,
                RemoteDesktopBackendPreference::Auto,
                WindowsNativeRdpCapability::Unavailable(reason),
            )
        );
    }
}

#[test]
fn windows_auto_does_not_fallback_when_the_capability_probe_fails() {
    let failure = WindowsNativeRdpProbeFailure::AbiMismatch;

    assert_eq!(
        Err(RemoteDesktopPresentationError::NativeProbeFailed(failure)),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::Auto,
            WindowsNativeRdpCapability::ProbeFailed(failure),
        )
    );
}

#[test]
fn explicit_windows_native_reports_unavailable_instead_of_falling_back() {
    for reason in [
        WindowsNativeRdpUnavailableReason::FeatureDisabled,
        WindowsNativeRdpUnavailableReason::UnsupportedPlatform,
        WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable,
        WindowsNativeRdpUnavailableReason::ClassNotRegistered,
        WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing,
    ] {
        assert_eq!(
            Err(RemoteDesktopPresentationError::NativeUnavailable(reason)),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Windows,
                RemoteDesktopBackendPreference::WindowsNative,
                WindowsNativeRdpCapability::Unavailable(reason),
            )
        );
    }
}

#[test]
fn explicit_windows_native_reports_probe_failure_instead_of_falling_back() {
    let failure = WindowsNativeRdpProbeFailure::InvalidNativeResponse;

    assert_eq!(
        Err(RemoteDesktopPresentationError::NativeProbeFailed(failure)),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpCapability::ProbeFailed(failure),
        )
    );
}

#[test]
fn explicit_windows_native_uses_native_when_available() {
    assert_eq!(
        Ok(selection(RemoteDesktopPresentation::NativeWindows, None)),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpCapability::Available,
        )
    );
}

#[test]
fn explicit_canvas_ignores_native_capability() {
    for capability in [
        WindowsNativeRdpCapability::Available,
        WindowsNativeRdpCapability::Unavailable(
            WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable,
        ),
        WindowsNativeRdpCapability::ProbeFailed(
            WindowsNativeRdpProbeFailure::InvalidNativeResponse,
        ),
    ] {
        assert_eq!(
            Ok(selection(RemoteDesktopPresentation::Canvas, None)),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Windows,
                RemoteDesktopBackendPreference::Canvas,
                capability,
            )
        );
    }
}

#[test]
fn non_windows_always_uses_canvas_without_a_fallback_reason() {
    for preference in [
        RemoteDesktopBackendPreference::Auto,
        RemoteDesktopBackendPreference::WindowsNative,
        RemoteDesktopBackendPreference::Canvas,
    ] {
        assert_eq!(
            Ok(selection(RemoteDesktopPresentation::Canvas, None)),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Other,
                preference,
                WindowsNativeRdpCapability::ProbeFailed(WindowsNativeRdpProbeFailure::Internal,),
            )
        );
    }
}

#[test]
fn capability_cache_probes_once_for_the_same_version_and_caches_failures() {
    let key = WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: 1,
        build_variant: WindowsNativeRdpBuildVariant::WindowsNative,
    };
    let expected = WindowsNativeRdpCapability::ProbeFailed(
        WindowsNativeRdpProbeFailure::InvalidNativeResponse,
    );
    let mut cache = None;
    let mut probe_calls = 0;

    let first = cached_windows_native_rdp_capability(&mut cache, key, || {
        probe_calls += 1;
        expected
    });
    let second = cached_windows_native_rdp_capability(&mut cache, key, || {
        probe_calls += 1;
        WindowsNativeRdpCapability::Available
    });

    assert_eq!(expected, first);
    assert_eq!(expected, second);
    assert_eq!(1, probe_calls);
}

#[test]
fn capability_cache_invalidates_when_the_probe_contract_version_changes() {
    let first_key = WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: 1,
        build_variant: WindowsNativeRdpBuildVariant::WindowsNative,
    };
    let second_key = WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: 2,
        build_variant: WindowsNativeRdpBuildVariant::WindowsNative,
    };
    let mut cache: Option<WindowsNativeRdpCapabilityCacheEntry> = None;
    let mut probe_calls = 0;

    let first = cached_windows_native_rdp_capability(&mut cache, first_key, || {
        probe_calls += 1;
        WindowsNativeRdpCapability::Available
    });
    let second = cached_windows_native_rdp_capability(&mut cache, second_key, || {
        probe_calls += 1;
        WindowsNativeRdpCapability::Unavailable(
            WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable,
        )
    });

    assert_eq!(WindowsNativeRdpCapability::Available, first);
    assert_eq!(
        WindowsNativeRdpCapability::Unavailable(
            WindowsNativeRdpUnavailableReason::ProbeReportedUnavailable
        ),
        second
    );
    assert_eq!(2, probe_calls);
}

#[test]
fn capability_cache_invalidates_when_the_build_variant_changes() {
    let feature_disabled_key = WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: 1,
        build_variant: WindowsNativeRdpBuildVariant::FeatureDisabled,
    };
    let windows_native_key = WindowsNativeRdpCapabilityCacheKey {
        probe_contract_version: 1,
        build_variant: WindowsNativeRdpBuildVariant::WindowsNative,
    };
    let mut cache: Option<WindowsNativeRdpCapabilityCacheEntry> = None;
    let mut probe_calls = 0;

    let first = cached_windows_native_rdp_capability(&mut cache, feature_disabled_key, || {
        probe_calls += 1;
        WindowsNativeRdpCapability::Unavailable(WindowsNativeRdpUnavailableReason::FeatureDisabled)
    });
    let second = cached_windows_native_rdp_capability(&mut cache, windows_native_key, || {
        probe_calls += 1;
        WindowsNativeRdpCapability::Available
    });

    assert_eq!(
        WindowsNativeRdpCapability::Unavailable(WindowsNativeRdpUnavailableReason::FeatureDisabled),
        first
    );
    assert_eq!(WindowsNativeRdpCapability::Available, second);
    assert_eq!(2, probe_calls);
}

#[cfg(not(feature = "windows-native-rdp"))]
#[test]
fn feature_off_is_stably_reported_as_pre_connect_unavailable() {
    assert_eq!(
        WindowsNativeRdpCapability::Unavailable(WindowsNativeRdpUnavailableReason::FeatureDisabled),
        current_windows_native_rdp_capability()
    );
}

#[cfg(all(feature = "windows-native-rdp", not(target_os = "windows")))]
#[test]
fn non_windows_is_stably_reported_as_pre_connect_unavailable() {
    assert_eq!(
        WindowsNativeRdpCapability::Unavailable(
            WindowsNativeRdpUnavailableReason::UnsupportedPlatform
        ),
        current_windows_native_rdp_capability()
    );
}

#[cfg(not(target_os = "windows"))]
#[test]
fn current_non_windows_factory_uses_canvas_without_claiming_a_fallback() {
    assert_eq!(
        Ok(selection(RemoteDesktopPresentation::Canvas, None)),
        create_remote_desktop_presentation(RemoteDesktopBackendPreference::Auto)
    );
}

#[cfg(feature = "windows-native-rdp")]
#[test]
fn create_time_hresult_classification_only_allows_known_unavailability() {
    use windows_rdp_host::{WindowsRdpHostError, WindowsRdpHresult};

    const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
    const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
    const E_FAIL: i32 = 0x8000_4005_u32 as i32;

    assert_eq!(
        Some(WindowsNativeRdpUnavailableReason::ClassNotRegistered),
        classify_windows_native_create_error(WindowsRdpHostError::NativeHresult {
            result: -4,
            hresult: WindowsRdpHresult::from_code(REGDB_E_CLASSNOTREG),
        })
    );
    assert_eq!(
        Some(WindowsNativeRdpUnavailableReason::RequiredInterfaceMissing),
        classify_windows_native_create_error(WindowsRdpHostError::NativeHresult {
            result: -4,
            hresult: WindowsRdpHresult::from_code(E_NOINTERFACE),
        })
    );
    assert_eq!(
        None,
        classify_windows_native_create_error(WindowsRdpHostError::NativeHresult {
            result: -4,
            hresult: WindowsRdpHresult::from_code(E_FAIL),
        })
    );
    assert_eq!(
        None,
        classify_windows_native_create_error(WindowsRdpHostError::Internal)
    );
}

#[test]
fn lifecycle_state_contract_covers_creation_through_release() {
    let states = [
        RemoteDesktopPresentationState::Created,
        RemoteDesktopPresentationState::NativeChildCreated,
        RemoteDesktopPresentationState::Connecting,
        RemoteDesktopPresentationState::Active,
        RemoteDesktopPresentationState::Inactive,
        RemoteDesktopPresentationState::Reconnecting,
        RemoteDesktopPresentationState::Closing,
        RemoteDesktopPresentationState::NativeChildDestroyed,
        RemoteDesktopPresentationState::Released,
    ];

    assert_eq!(
        Some(&RemoteDesktopPresentationState::Created),
        states.first()
    );
    assert_eq!(
        Some(&RemoteDesktopPresentationState::Released),
        states.last()
    );
}
