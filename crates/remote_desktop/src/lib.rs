pub mod backend;
pub mod capabilities;
pub mod config;
pub mod framebuffer;
pub mod helper_protocol;
pub mod input;
pub mod output;
pub mod runtime;

pub mod backends;

pub use backend::{RemoteDesktopBackend, create_backend};
pub use capabilities::{RemoteDesktopCapabilities, ResizeSupport};
pub use config::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopSize};
pub use input::{RemoteDesktopInput, RemoteKey, RemoteMouseButton, RemoteNamedKey};
pub use output::RemoteDesktopOutput;
pub use runtime::RemoteDesktopRuntime;
