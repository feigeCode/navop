//! Versioned Rust/C++ boundary for the Windows native Remote Desktop host.
//!
//! This slice owns an opaque lifecycle handle, an internal owned-copy callback
//! queue, a synchronous zeroizing credential transport, and the first Windows
//! native presentation slice: a borrowed parent window with a hidden,
//! zero-sized ActiveX child.

mod capabilities;
mod credential;
mod diagnostic;
mod error;
mod event;
mod ffi;
mod handle;
mod lifecycle;
#[cfg(all(test, windows_rdp_host_native))]
mod native_tests;
mod options;

pub use capabilities::WindowsRdpHostCapabilities;
pub use credential::WindowsRdpCredentialBundle;
pub use diagnostic::{
    WindowsRdpDiagnosticContext, WindowsRdpDiagnosticSnapshot, WindowsRdpRedactedValue,
    WindowsRdpUsernameRedaction,
};
pub use error::WindowsRdpHostError;
pub use error::WindowsRdpHresult;
pub use error::WindowsRdpHresultKind;
pub use error::{
    WindowsRdpDiagnosticCategory, WindowsRdpDisconnectReason, WindowsRdpFatalError,
    WindowsRdpFatalErrorKind, WindowsRdpLogonError, WindowsRdpLogonErrorKind, WindowsRdpWarning,
    WindowsRdpWarningKind,
};
pub use event::{WindowsRdpEvent, WindowsRdpRawEvent};
pub use handle::{WindowsRdpConnectionState, WindowsRdpHost, WindowsRdpRequestCloseStatus};
pub use lifecycle::WindowsRdpHostLifecycle;
pub use options::{
    WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS, WindowsRdpColorDepth, WindowsRdpConnectionOptions,
    WindowsRdpHostOptions, WindowsRdpParentWindow,
};
