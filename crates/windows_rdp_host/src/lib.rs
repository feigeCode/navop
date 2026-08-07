//! Versioned Rust/C++ boundary for the Windows native Remote Desktop host.
//!
//! This slice owns an opaque lifecycle handle, an internal owned-copy callback
//! queue, and a synchronous zeroizing credential transport. It intentionally
//! does not create COM objects, ActiveX controls, native windows, event sinks,
//! or RDP connections.

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
pub use handle::WindowsRdpHost;
pub use lifecycle::WindowsRdpHostLifecycle;
pub use options::WindowsRdpHostOptions;
