//! Versioned Rust/C++ boundary for the Windows native Remote Desktop host.
//!
//! This initial slice owns only an opaque lifecycle handle. It intentionally
//! does not create COM objects, ActiveX controls, native windows, event sinks,
//! or RDP connections.

mod capabilities;
mod error;
mod ffi;
mod handle;
mod options;

pub use capabilities::WindowsRdpHostCapabilities;
pub use error::WindowsRdpHostError;
pub use handle::WindowsRdpHost;
pub use options::WindowsRdpHostOptions;
