#[cfg(windows)]
use one_core::storage::RemoteDesktopBackendPreference;

#[cfg(windows)]
pub(super) const fn backend_preferences() -> [RemoteDesktopBackendPreference; 3] {
    [
        RemoteDesktopBackendPreference::Auto,
        RemoteDesktopBackendPreference::WindowsNative,
        RemoteDesktopBackendPreference::Canvas,
    ]
}

/// Whether the Windows native RDP backend is compiled into this build. The
/// form keeps the option selectable only when the backend can actually run.
#[cfg(windows)]
pub(super) const fn windows_native_rdp_available() -> bool {
    cfg!(all(feature = "windows-native-rdp", target_os = "windows"))
}

#[cfg(all(test, windows))]
mod tests {
    use one_core::storage::RemoteDesktopBackendPreference;

    use super::backend_preferences;

    #[test]
    fn canvas_is_the_default_backend_preference() {
        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            RemoteDesktopBackendPreference::default()
        );
    }

    #[test]
    fn form_offers_each_backend_preference_without_collapsing_auto() {
        assert_eq!(
            [
                RemoteDesktopBackendPreference::Auto,
                RemoteDesktopBackendPreference::WindowsNative,
                RemoteDesktopBackendPreference::Canvas,
            ],
            backend_preferences()
        );
    }
}
