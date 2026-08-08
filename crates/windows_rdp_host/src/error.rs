use std::fmt;

use crate::ffi;

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
}
