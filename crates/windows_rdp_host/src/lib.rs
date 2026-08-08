//! Versioned Rust/C++ boundary for the Windows native Remote Desktop host.
//!
//! This slice owns an opaque lifecycle handle, an internal owned-copy callback
//! queue, a synchronous zeroizing credential transport, and the first Windows
//! native presentation slice: a borrowed parent window with a hidden,
//! zero-sized ActiveX child.

mod capabilities;
mod credential;
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
pub use error::WindowsRdpHostError;
pub use handle::{WindowsRdpConnectionState, WindowsRdpHost, WindowsRdpRequestCloseStatus};
pub use lifecycle::WindowsRdpHostLifecycle;
pub use options::{
    WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS, WindowsRdpColorDepth, WindowsRdpConnectionOptions,
    WindowsRdpHostOptions, WindowsRdpParentWindow,
};
