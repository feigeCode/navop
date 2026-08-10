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

/// Stable classification for `IMsTscAxEvents::OnWarning`.
///
/// Microsoft's documented warning-code list is intentionally treated as
/// non-exhaustive. Unknown values retain their raw signed code in
/// [`WindowsRdpWarning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpWarningKind {
    BitmapCacheCorrupt,
    Unknown,
}

/// A raw-code-preserving warning diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpWarning {
    kind: WindowsRdpWarningKind,
    code: i32,
}

impl WindowsRdpWarning {
    pub const fn from_native_code(code: i32) -> Self {
        let kind = match code {
            1 => WindowsRdpWarningKind::BitmapCacheCorrupt,
            _ => WindowsRdpWarningKind::Unknown,
        };

        Self { kind, code }
    }

    pub const fn kind(self) -> WindowsRdpWarningKind {
        self.kind
    }

    pub const fn code(self) -> i32 {
        self.code
    }
}

/// Stable classification for `IMsTscAxEvents::OnFatalError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpFatalErrorKind {
    /// The documented native value `0`, distinct from an unrecognized code.
    UnknownError,
    Internal,
    OutOfMemory,
    WindowCreation,
    InvalidState,
    ConnectionUnrecoverable,
    WinsockInitialization,
    Unknown,
}

/// A raw-code-preserving fatal ActiveX diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpFatalError {
    kind: WindowsRdpFatalErrorKind,
    code: i32,
}

impl WindowsRdpFatalError {
    pub const fn from_native_code(code: i32) -> Self {
        let kind = match code {
            0 => WindowsRdpFatalErrorKind::UnknownError,
            1 | 4 | 6 => WindowsRdpFatalErrorKind::Internal,
            2 => WindowsRdpFatalErrorKind::OutOfMemory,
            3 => WindowsRdpFatalErrorKind::WindowCreation,
            5 => WindowsRdpFatalErrorKind::InvalidState,
            7 => WindowsRdpFatalErrorKind::ConnectionUnrecoverable,
            100 => WindowsRdpFatalErrorKind::WinsockInitialization,
            _ => WindowsRdpFatalErrorKind::Unknown,
        };

        Self { kind, code }
    }

    pub const fn kind(self) -> WindowsRdpFatalErrorKind {
        self.kind
    }

    pub const fn code(self) -> i32 {
        self.code
    }
}

/// Stable classification for `IMsTscAxEvents::OnLogonError`.
///
/// The native code list is not exhaustive and includes multiple independent
/// spaces, including signed NTSTATUS values and session-arbitration results.
/// Classification therefore remains conservative and always preserves the
/// original signed code in [`WindowsRdpLogonError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpLogonErrorKind {
    BadCredentials,
    PasswordChangeRequired,
    Other,
    Warning,
    AccessDenied,
    AccountRestriction,
    SessionArbitration,
    Unknown,
}

/// A raw-code-preserving logon diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpLogonError {
    kind: WindowsRdpLogonErrorKind,
    code: i32,
}

impl WindowsRdpLogonError {
    pub const fn from_native_code(code: i32) -> Self {
        let kind = match code {
            // LOGON_FAILED_BAD_PASSWORD / STATUS_LOGON_FAILURE.
            0 | -1_073_741_715 => WindowsRdpLogonErrorKind::BadCredentials,
            // LOGON_FAILED_UPDATE_PASSWORD / STATUS_PASSWORD_MUST_CHANGE.
            1 | -1_073_741_276 => WindowsRdpLogonErrorKind::PasswordChangeRequired,
            2 => WindowsRdpLogonErrorKind::Other,
            3 => WindowsRdpLogonErrorKind::Warning,
            -1 => WindowsRdpLogonErrorKind::AccessDenied,
            // STATUS_ACCOUNT_RESTRICTION.
            -1_073_741_714 => WindowsRdpLogonErrorKind::AccountRestriction,
            // Documented ARBITRATION_CODE_* values.
            -7..=-2 => WindowsRdpLogonErrorKind::SessionArbitration,
            _ => WindowsRdpLogonErrorKind::Unknown,
        };

        Self { kind, code }
    }

    pub const fn kind(self) -> WindowsRdpLogonErrorKind {
        self.kind
    }

    pub const fn code(self) -> i32 {
        self.code
    }
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

/// Raw signed HRESULT returned by a synchronous native RDP operation.
///
/// The value is preserved exactly as reported by the Windows API. The facade
/// intentionally does not expose native error text, endpoint data, or sensitive values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsRdpHresultKind {
    ClassNotRegistered,
    NoInterface,
    InvalidArgument,
    OutOfMemory,
    AccessDenied,
    WrongThread,
    ComNotInitialized,
    CallRejected,
    ServerCallRetryLater,
    Disconnected,
    Timeout,
    Cancelled,
    Unknown,
}

const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000E_u32 as i32;
const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
const RPC_E_WRONG_THREAD: i32 = 0x8001_010E_u32 as i32;
const CO_E_NOTINITIALIZED: i32 = 0x8004_01F0_u32 as i32;
const RPC_E_CALL_REJECTED: i32 = 0x8001_0001_u32 as i32;
const RPC_E_SERVERCALL_RETRYLATER: i32 = 0x8001_010A_u32 as i32;
const RPC_E_DISCONNECTED: i32 = 0x8001_0108_u32 as i32;
const HRESULT_FROM_WIN32_ERROR_TIMEOUT: i32 = 0x8007_05B4_u32 as i32;
const HRESULT_FROM_WIN32_ERROR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpHresult {
    code: i32,
}

impl WindowsRdpHresult {
    pub const fn from_code(code: i32) -> Self {
        Self { code }
    }

    pub const fn code(self) -> i32 {
        self.code
    }

    pub const fn kind(self) -> WindowsRdpHresultKind {
        match self.code {
            REGDB_E_CLASSNOTREG => WindowsRdpHresultKind::ClassNotRegistered,
            E_NOINTERFACE => WindowsRdpHresultKind::NoInterface,
            E_INVALIDARG => WindowsRdpHresultKind::InvalidArgument,
            E_OUTOFMEMORY => WindowsRdpHresultKind::OutOfMemory,
            E_ACCESSDENIED => WindowsRdpHresultKind::AccessDenied,
            RPC_E_WRONG_THREAD => WindowsRdpHresultKind::WrongThread,
            CO_E_NOTINITIALIZED => WindowsRdpHresultKind::ComNotInitialized,
            RPC_E_CALL_REJECTED => WindowsRdpHresultKind::CallRejected,
            RPC_E_SERVERCALL_RETRYLATER => WindowsRdpHresultKind::ServerCallRetryLater,
            RPC_E_DISCONNECTED => WindowsRdpHresultKind::Disconnected,
            HRESULT_FROM_WIN32_ERROR_TIMEOUT => WindowsRdpHresultKind::Timeout,
            HRESULT_FROM_WIN32_ERROR_CANCELLED => WindowsRdpHresultKind::Cancelled,
            _ => WindowsRdpHresultKind::Unknown,
        }
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
    NativeHresult {
        result: i32,
        hresult: WindowsRdpHresult,
    },
}

impl WindowsRdpHostError {
    /// Returns the stable native result code when this error carries an HRESULT.
    pub const fn native_result(self) -> Option<i32> {
        match self {
            Self::NativeHresult { result, .. } => Some(result),
            _ => None,
        }
    }

    /// Returns the raw signed HRESULT when one was supplied by the native host.
    pub const fn hresult(self) -> Option<WindowsRdpHresult> {
        match self {
            Self::NativeHresult { hresult, .. } => Some(hresult),
            _ => None,
        }
    }
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
            Self::NativeHresult { result, hresult } => write!(
                formatter,
                "Windows RDP host result {result} with HRESULT {:#010X}",
                hresult.code() as u32
            ),
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
    fn hresult_error_preserves_raw_signed_code() {
        let error = WindowsRdpHostError::NativeHresult {
            result: ffi::RESULT_INTERNAL_ERROR,
            hresult: WindowsRdpHresult::from_code(i32::MIN),
        };

        assert_eq!(error.native_result(), Some(ffi::RESULT_INTERNAL_ERROR));
        assert_eq!(error.hresult().map(WindowsRdpHresult::code), Some(i32::MIN));
        assert_eq!(
            error.to_string(),
            "Windows RDP host result 4 with HRESULT 0x80000000"
        );
    }

    #[test]
    fn documented_hresult_codes_map_to_stable_kinds_and_preserve_raw_values() {
        let cases = [
            (
                0x8004_0154_u32 as i32,
                WindowsRdpHresultKind::ClassNotRegistered,
            ),
            (0x8000_4002_u32 as i32, WindowsRdpHresultKind::NoInterface),
            (
                0x8007_0057_u32 as i32,
                WindowsRdpHresultKind::InvalidArgument,
            ),
            (0x8007_000E_u32 as i32, WindowsRdpHresultKind::OutOfMemory),
            (0x8007_0005_u32 as i32, WindowsRdpHresultKind::AccessDenied),
            (0x8001_010E_u32 as i32, WindowsRdpHresultKind::WrongThread),
            (
                0x8004_01F0_u32 as i32,
                WindowsRdpHresultKind::ComNotInitialized,
            ),
            (0x8001_0001_u32 as i32, WindowsRdpHresultKind::CallRejected),
            (
                0x8001_010A_u32 as i32,
                WindowsRdpHresultKind::ServerCallRetryLater,
            ),
            (0x8001_0108_u32 as i32, WindowsRdpHresultKind::Disconnected),
            (0x8007_05B4_u32 as i32, WindowsRdpHresultKind::Timeout),
            (0x8007_04C7_u32 as i32, WindowsRdpHresultKind::Cancelled),
        ];

        for (code, expected) in cases {
            let hresult = WindowsRdpHresult::from_code(code);
            assert_eq!(hresult.kind(), expected, "HRESULT {:#010X}", code as u32);
            assert_eq!(hresult.code(), code);
        }
    }

    #[test]
    fn unknown_hresult_codes_remain_unknown_and_preserve_signed_raw_values() {
        for code in [0, -1, i32::MIN, i32::MAX, 0x8000_4005_u32 as i32] {
            let hresult = WindowsRdpHresult::from_code(code);
            assert_eq!(hresult.kind(), WindowsRdpHresultKind::Unknown);
            assert_eq!(hresult.code(), code);
        }
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

    #[test]
    fn warning_codes_preserve_raw_values_and_only_classify_documented_values() {
        let documented = WindowsRdpWarning::from_native_code(1);
        assert_eq!(documented.kind(), WindowsRdpWarningKind::BitmapCacheCorrupt);
        assert_eq!(documented.code(), 1);

        for code in [0, -1, i32::MIN, i32::MAX] {
            let warning = WindowsRdpWarning::from_native_code(code);
            assert_eq!(warning.kind(), WindowsRdpWarningKind::Unknown);
            assert_eq!(warning.code(), code);
        }
    }

    #[test]
    fn fatal_error_codes_distinguish_documented_unknown_from_unrecognized_values() {
        let cases = [
            (0, WindowsRdpFatalErrorKind::UnknownError),
            (1, WindowsRdpFatalErrorKind::Internal),
            (2, WindowsRdpFatalErrorKind::OutOfMemory),
            (3, WindowsRdpFatalErrorKind::WindowCreation),
            (4, WindowsRdpFatalErrorKind::Internal),
            (5, WindowsRdpFatalErrorKind::InvalidState),
            (6, WindowsRdpFatalErrorKind::Internal),
            (7, WindowsRdpFatalErrorKind::ConnectionUnrecoverable),
            (100, WindowsRdpFatalErrorKind::WinsockInitialization),
        ];

        for (code, expected) in cases {
            let error = WindowsRdpFatalError::from_native_code(code);
            assert_eq!(error.kind(), expected, "fatal error code {code}");
            assert_eq!(error.code(), code);
        }

        for code in [-1, 8, 99, 101, i32::MIN, i32::MAX] {
            let error = WindowsRdpFatalError::from_native_code(code);
            assert_eq!(error.kind(), WindowsRdpFatalErrorKind::Unknown);
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn logon_error_codes_classify_documented_signed_spaces_and_preserve_raw_values() {
        let cases = [
            (0, WindowsRdpLogonErrorKind::BadCredentials),
            (-1_073_741_715, WindowsRdpLogonErrorKind::BadCredentials),
            (1, WindowsRdpLogonErrorKind::PasswordChangeRequired),
            (
                -1_073_741_276,
                WindowsRdpLogonErrorKind::PasswordChangeRequired,
            ),
            (2, WindowsRdpLogonErrorKind::Other),
            (3, WindowsRdpLogonErrorKind::Warning),
            (-1, WindowsRdpLogonErrorKind::AccessDenied),
            (-1_073_741_714, WindowsRdpLogonErrorKind::AccountRestriction),
            (-2, WindowsRdpLogonErrorKind::SessionArbitration),
            (-3, WindowsRdpLogonErrorKind::SessionArbitration),
            (-4, WindowsRdpLogonErrorKind::SessionArbitration),
            (-5, WindowsRdpLogonErrorKind::SessionArbitration),
            (-6, WindowsRdpLogonErrorKind::SessionArbitration),
            (-7, WindowsRdpLogonErrorKind::SessionArbitration),
        ];

        for (code, expected) in cases {
            let error = WindowsRdpLogonError::from_native_code(code);
            assert_eq!(error.kind(), expected, "logon error code {code}");
            assert_eq!(error.code(), code);
        }

        for code in [-8, 4, i32::MIN, i32::MAX] {
            let error = WindowsRdpLogonError::from_native_code(code);
            assert_eq!(error.kind(), WindowsRdpLogonErrorKind::Unknown);
            assert_eq!(error.code(), code);
        }
    }
}
