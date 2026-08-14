use one_core::storage::RemoteDesktopBackendPreference;

pub(super) const fn normalize_for_form(
    preference: RemoteDesktopBackendPreference,
) -> RemoteDesktopBackendPreference {
    match preference {
        RemoteDesktopBackendPreference::WindowsNative => {
            RemoteDesktopBackendPreference::WindowsNative
        }
        RemoteDesktopBackendPreference::Auto | RemoteDesktopBackendPreference::Canvas => {
            RemoteDesktopBackendPreference::Canvas
        }
    }
}

pub(super) const fn toggle_windows_native(
    preference: RemoteDesktopBackendPreference,
) -> RemoteDesktopBackendPreference {
    if windows_native_enabled(preference) {
        RemoteDesktopBackendPreference::Canvas
    } else {
        RemoteDesktopBackendPreference::WindowsNative
    }
}

pub(super) const fn windows_native_enabled(preference: RemoteDesktopBackendPreference) -> bool {
    matches!(preference, RemoteDesktopBackendPreference::WindowsNative)
}

#[cfg(test)]
mod tests {
    use one_core::storage::RemoteDesktopBackendPreference;

    use super::{normalize_for_form, toggle_windows_native, windows_native_enabled};

    #[test]
    fn canvas_is_the_default_backend_preference() {
        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            RemoteDesktopBackendPreference::default()
        );
    }

    #[test]
    fn form_normalizes_automatic_selection_to_canvas() {
        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            normalize_for_form(RemoteDesktopBackendPreference::Auto)
        );
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            normalize_for_form(RemoteDesktopBackendPreference::WindowsNative)
        );
    }

    #[test]
    fn native_rdp_checkbox_toggles_between_native_and_canvas() {
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            toggle_windows_native(RemoteDesktopBackendPreference::Canvas)
        );
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            toggle_windows_native(RemoteDesktopBackendPreference::Auto)
        );
        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            toggle_windows_native(RemoteDesktopBackendPreference::WindowsNative)
        );
        assert!(windows_native_enabled(
            RemoteDesktopBackendPreference::WindowsNative
        ));
        assert!(!windows_native_enabled(
            RemoteDesktopBackendPreference::Canvas
        ));
    }
}
