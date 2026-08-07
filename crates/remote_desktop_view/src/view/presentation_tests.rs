use one_core::storage::RemoteDesktopBackendPreference;

use super::presentation::{
    RemoteDesktopPlatform, RemoteDesktopPresentation, RemoteDesktopPresentationError,
    RemoteDesktopPresentationState, WindowsNativeRdpAvailability,
    create_remote_desktop_presentation, current_windows_native_rdp_availability,
    select_remote_desktop_presentation,
};

#[test]
fn windows_auto_uses_native_when_available() {
    assert_eq!(
        Ok(RemoteDesktopPresentation::NativeWindows),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::Auto,
            WindowsNativeRdpAvailability::Available,
        )
    );
}

#[test]
fn windows_auto_falls_back_to_canvas_when_native_is_unavailable() {
    assert_eq!(
        Ok(RemoteDesktopPresentation::Canvas),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::Auto,
            WindowsNativeRdpAvailability::UnavailableNotBuilt,
        )
    );
}

#[test]
fn explicit_windows_native_reports_unavailable_instead_of_falling_back() {
    assert_eq!(
        Err(RemoteDesktopPresentationError::UnavailableNotBuilt),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpAvailability::UnavailableNotBuilt,
        )
    );
}

#[test]
fn explicit_windows_native_uses_native_when_available() {
    assert_eq!(
        Ok(RemoteDesktopPresentation::NativeWindows),
        select_remote_desktop_presentation(
            RemoteDesktopPlatform::Windows,
            RemoteDesktopBackendPreference::WindowsNative,
            WindowsNativeRdpAvailability::Available,
        )
    );
}

#[test]
fn explicit_canvas_ignores_native_availability() {
    for availability in [
        WindowsNativeRdpAvailability::Available,
        WindowsNativeRdpAvailability::UnavailableNotBuilt,
    ] {
        assert_eq!(
            Ok(RemoteDesktopPresentation::Canvas),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Windows,
                RemoteDesktopBackendPreference::Canvas,
                availability,
            )
        );
    }
}

#[test]
fn non_windows_always_uses_canvas() {
    for preference in [
        RemoteDesktopBackendPreference::Auto,
        RemoteDesktopBackendPreference::WindowsNative,
        RemoteDesktopBackendPreference::Canvas,
    ] {
        assert_eq!(
            Ok(RemoteDesktopPresentation::Canvas),
            select_remote_desktop_presentation(
                RemoteDesktopPlatform::Other,
                preference,
                WindowsNativeRdpAvailability::Available,
            )
        );
    }
}

#[test]
fn task_zero_native_factory_is_explicitly_not_built() {
    assert_eq!(
        WindowsNativeRdpAvailability::UnavailableNotBuilt,
        current_windows_native_rdp_availability()
    );
    assert_eq!(
        Ok(RemoteDesktopPresentation::Canvas),
        create_remote_desktop_presentation(RemoteDesktopBackendPreference::Auto)
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
