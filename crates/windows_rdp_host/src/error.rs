use std::fmt;

use crate::ffi;

/// High-level category for a native disconnect diagnostic.
///
/// The category is intentionally separate from user-facing text so the UI can
/// localize it without embedding native strings or codes in the event queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpDiagnosticCategory {
    UserInitiated,
    Authentication,
    CertificateOrSecurity,
    Gateway,
    ServerPolicy,
    Network,
    NativeUnavailable,
    Unknown,
}

/// A raw-code-preserving disconnect diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpDisconnectReason {
    category: WindowsRdpDiagnosticCategory,
    disconnect_code: i32,
    extended_code: Option<i32>,
}

impl WindowsRdpDisconnectReason {
    pub const fn new(
        category: WindowsRdpDiagnosticCategory,
        disconnect_code: i32,
        extended_code: Option<i32>,
    ) -> Self {
        Self {
            category,
            disconnect_code,
            extended_code,
        }
    }

    pub const fn unknown(disconnect_code: i32, extended_code: Option<i32>) -> Self {
        Self::new(
            WindowsRdpDiagnosticCategory::Unknown,
            disconnect_code,
            extended_code,
        )
    }

    pub(crate) const fn from_native_codes(
        disconnect_code: i32,
        extended_code: Option<i32>,
    ) -> Self {
        let extended_category = match extended_code {
            Some(code) => classify_extended_disconnect_code(code),
            None => None,
        };
        let category = match extended_category {
            Some(category) => category,
            None => match classify_disconnect_code(disconnect_code) {
                Some(category) => category,
                None => WindowsRdpDiagnosticCategory::Unknown,
            },
        };

        Self::new(category, disconnect_code, extended_code)
    }

    pub const fn category(self) -> WindowsRdpDiagnosticCategory {
        self.category
    }

    pub const fn disconnect_code(self) -> i32 {
        self.disconnect_code
    }

    pub const fn extended_code(self) -> Option<i32> {
        self.extended_code
    }
}

// `IMsTscAxEvents::OnDisconnected` and `ExtendedDisconnectReasonCode` use
// independent numeric spaces. Keep these tables explicit and conservative:
// known extended reasons take precedence, while unknown extended values fall
// back to the primary disconnect reason. Raw values are always retained.
const fn classify_disconnect_code(code: i32) -> Option<WindowsRdpDiagnosticCategory> {
    match code {
        1 | 2 => Some(WindowsRdpDiagnosticCategory::UserInitiated),
        2055 | 2567 | 2823 | 3079 | 3335 | 3591 | 3847 | 4615 | 7175 | 8711 => {
            Some(WindowsRdpDiagnosticCategory::Authentication)
        }
        1030 | 1286 | 1542 | 1798 | 2822 | 3078 | 6919 => {
            Some(WindowsRdpDiagnosticCategory::CertificateOrSecurity)
        }
        5639 | 5895 | 8455 => Some(WindowsRdpDiagnosticCategory::ServerPolicy),
        260 | 264 | 516 | 520 | 772 | 776 | 1028 | 1288 | 1540 | 1796 | 2052 | 2308 => {
            Some(WindowsRdpDiagnosticCategory::Network)
        }
        _ => None,
    }
}

const fn classify_extended_disconnect_code(code: i32) -> Option<WindowsRdpDiagnosticCategory> {
    match code {
        1 | 2 | 11 | 12 => Some(WindowsRdpDiagnosticCategory::UserInitiated),
        10 | 768 => Some(WindowsRdpDiagnosticCategory::Authentication),
        8 | 264 => Some(WindowsRdpDiagnosticCategory::CertificateOrSecurity),
        3 | 4 | 5 | 7 | 9 | 257 | 258 | 265 | 266 => {
            Some(WindowsRdpDiagnosticCategory::ServerPolicy)
        }
        262 => Some(WindowsRdpDiagnosticCategory::Network),
        // Other documented licensing/internal/protocol values do not map
        // cleanly to the stable public categories and remain raw Unknown.
        _ => None,
    }
}

/// Deterministic failures returned by the versioned native host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpHostError {
    InvalidArgument,
    AbiMismatch,
    AllocationFailed,
    Internal,
    Unavailable,
    WrongThread,
    CallbackInFlight,
    InvalidState,
    NativeReturnedNullHandle,
    NativeDidNotClearHandle,
    InvalidNativeResponse,
    UnexpectedNativeResult(i32),
}

impl fmt::Display for WindowsRdpHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument => formatter.write_str("invalid Windows RDP host argument"),
            Self::AbiMismatch => formatter.write_str("Windows RDP host ABI mismatch"),
            Self::AllocationFailed => formatter.write_str("Windows RDP host allocation failed"),
            Self::Internal => formatter.write_str("Windows RDP host internal failure"),
            Self::Unavailable => formatter.write_str("Windows RDP host is unavailable"),
            Self::WrongThread => {
                formatter.write_str("Windows RDP host called from the wrong thread")
            }
            Self::CallbackInFlight => formatter.write_str("Windows RDP host callback is in flight"),
            Self::InvalidState => {
                formatter.write_str("Windows RDP host operation is invalid in the current state")
            }
            Self::NativeReturnedNullHandle => {
                formatter.write_str("Windows RDP host returned a null handle")
            }
            Self::NativeDidNotClearHandle => {
                formatter.write_str("Windows RDP host destroy did not clear its handle")
            }
            Self::InvalidNativeResponse => {
                formatter.write_str("Windows RDP host returned an invalid ABI response")
            }
            Self::UnexpectedNativeResult(result) => {
                write!(formatter, "unexpected Windows RDP host result {result}")
            }
        }
    }
}

impl std::error::Error for WindowsRdpHostError {}

pub(crate) fn check_native_result(result: ffi::NativeResult) -> Result<(), WindowsRdpHostError> {
    match result {
        ffi::RESULT_OK => Ok(()),
        ffi::RESULT_INVALID_ARGUMENT => Err(WindowsRdpHostError::InvalidArgument),
        ffi::RESULT_ABI_MISMATCH => Err(WindowsRdpHostError::AbiMismatch),
        ffi::RESULT_ALLOCATION_FAILED => Err(WindowsRdpHostError::AllocationFailed),
        ffi::RESULT_INTERNAL_ERROR => Err(WindowsRdpHostError::Internal),
        ffi::RESULT_UNAVAILABLE => Err(WindowsRdpHostError::Unavailable),
        ffi::RESULT_WRONG_THREAD => Err(WindowsRdpHostError::WrongThread),
        ffi::RESULT_CALLBACK_IN_FLIGHT => Err(WindowsRdpHostError::CallbackInFlight),
        ffi::RESULT_INVALID_STATE => Err(WindowsRdpHostError::InvalidState),
        other => Err(WindowsRdpHostError::UnexpectedNativeResult(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_results_have_deterministic_error_mapping() {
        assert_eq!(check_native_result(ffi::RESULT_OK), Ok(()));
        assert_eq!(
            check_native_result(ffi::RESULT_INVALID_ARGUMENT),
            Err(WindowsRdpHostError::InvalidArgument)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_ABI_MISMATCH),
            Err(WindowsRdpHostError::AbiMismatch)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_ALLOCATION_FAILED),
            Err(WindowsRdpHostError::AllocationFailed)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_INTERNAL_ERROR),
            Err(WindowsRdpHostError::Internal)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_UNAVAILABLE),
            Err(WindowsRdpHostError::Unavailable)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_WRONG_THREAD),
            Err(WindowsRdpHostError::WrongThread)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_CALLBACK_IN_FLIGHT),
            Err(WindowsRdpHostError::CallbackInFlight)
        );
        assert_eq!(
            check_native_result(ffi::RESULT_INVALID_STATE),
            Err(WindowsRdpHostError::InvalidState)
        );
        assert_eq!(
            check_native_result(99),
            Err(WindowsRdpHostError::UnexpectedNativeResult(99))
        );
    }

    #[test]
    fn disconnect_reason_preserves_unknown_raw_codes_without_native_text() {
        let reason = WindowsRdpDisconnectReason::unknown(i32::MIN, Some(i32::MAX));

        assert_eq!(reason.category(), WindowsRdpDiagnosticCategory::Unknown);
        assert_eq!(reason.disconnect_code(), i32::MIN);
        assert_eq!(reason.extended_code(), Some(i32::MAX));
    }

    #[test]
    fn primary_disconnect_codes_map_only_high_confidence_categories() {
        let cases = [
            (1, WindowsRdpDiagnosticCategory::UserInitiated),
            (2055, WindowsRdpDiagnosticCategory::Authentication),
            (1030, WindowsRdpDiagnosticCategory::CertificateOrSecurity),
            (5639, WindowsRdpDiagnosticCategory::ServerPolicy),
            (2308, WindowsRdpDiagnosticCategory::Network),
        ];

        for (code, expected) in cases {
            let reason = WindowsRdpDisconnectReason::from_native_codes(code, None);
            assert_eq!(reason.category(), expected, "disconnect code {code}");
            assert_eq!(reason.disconnect_code(), code);
            assert_eq!(reason.extended_code(), None);
        }

        for code in [0, 3, 2056, 2310, i32::MIN, i32::MAX] {
            assert_eq!(
                WindowsRdpDisconnectReason::from_native_codes(code, None).category(),
                WindowsRdpDiagnosticCategory::Unknown,
                "disconnect code {code}"
            );
        }
    }

    #[test]
    fn ambiguous_server_and_licensing_codes_are_not_forced_into_public_categories() {
        assert_eq!(
            WindowsRdpDisconnectReason::from_native_codes(3, None).category(),
            WindowsRdpDiagnosticCategory::Unknown
        );

        for extended_code in [256, 259, 260, 261, 263, 267] {
            let reason =
                WindowsRdpDisconnectReason::from_native_codes(i32::MIN, Some(extended_code));
            assert_eq!(
                reason.category(),
                WindowsRdpDiagnosticCategory::Unknown,
                "extended disconnect code {extended_code}"
            );
            assert_eq!(reason.disconnect_code(), i32::MIN);
            assert_eq!(reason.extended_code(), Some(extended_code));
        }
    }

    #[test]
    fn extended_disconnect_codes_are_classified_in_their_own_code_space() {
        let cases = [
            (11, WindowsRdpDiagnosticCategory::UserInitiated),
            (768, WindowsRdpDiagnosticCategory::Authentication),
            (264, WindowsRdpDiagnosticCategory::CertificateOrSecurity),
            (266, WindowsRdpDiagnosticCategory::ServerPolicy),
            (262, WindowsRdpDiagnosticCategory::Network),
        ];

        for (extended_code, expected) in cases {
            let reason =
                WindowsRdpDisconnectReason::from_native_codes(i32::MIN, Some(extended_code));
            assert_eq!(
                reason.category(),
                expected,
                "extended disconnect code {extended_code}"
            );
            assert_eq!(reason.disconnect_code(), i32::MIN);
            assert_eq!(reason.extended_code(), Some(extended_code));
        }
    }

    #[test]
    fn known_extended_reason_overrides_primary_and_unknown_extended_reason_falls_back() {
        let extended_override = WindowsRdpDisconnectReason::from_native_codes(2308, Some(768));
        assert_eq!(
            extended_override.category(),
            WindowsRdpDiagnosticCategory::Authentication
        );
        assert_eq!(extended_override.disconnect_code(), 2308);
        assert_eq!(extended_override.extended_code(), Some(768));

        let primary_fallback = WindowsRdpDisconnectReason::from_native_codes(2308, Some(-55));
        assert_eq!(
            primary_fallback.category(),
            WindowsRdpDiagnosticCategory::Network
        );
        assert_eq!(primary_fallback.disconnect_code(), 2308);
        assert_eq!(primary_fallback.extended_code(), Some(-55));

        let unknown = WindowsRdpDisconnectReason::from_native_codes(i32::MIN, Some(i32::MAX));
        assert_eq!(unknown.category(), WindowsRdpDiagnosticCategory::Unknown);
        assert_eq!(unknown.disconnect_code(), i32::MIN);
        assert_eq!(unknown.extended_code(), Some(i32::MAX));
    }
}
