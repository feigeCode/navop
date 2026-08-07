//! Versioned Rust/C++ boundary for the Windows native Remote Desktop host.
//!
//! This slice owns an opaque lifecycle handle plus an internal, owned-copy
//! callback queue. It intentionally does not create COM objects, ActiveX
//! controls, native windows, event sinks, or RDP connections.

mod capabilities;
mod error;
mod event;
mod ffi;
mod handle;
mod options;

pub use capabilities::WindowsRdpHostCapabilities;
pub use error::WindowsRdpHostError;
pub use handle::WindowsRdpHost;
pub use options::WindowsRdpHostOptions;
