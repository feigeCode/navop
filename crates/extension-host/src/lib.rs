//! Minimal extension host runtime for process-based database extensions.
//!
//! This crate intentionally contains only the production IPC runtime core:
//! framed transport, JSON-RPC client, process lifecycle, init negotiation, and
//! a small runtime abstraction. Packaging, update, signature verification, UI,
//! and Component/WASM support are outside the first migration stage.

pub mod client;
pub mod error;
pub mod host_api;
pub mod manifest;
pub mod negotiation;
pub mod process;
pub mod process_session;
pub mod runtime;
pub mod transport;

pub use client::{CancellationToken, JsonRpcClient, JsonRpcClientHandle, RequestOptions};
pub use error::{HostError, HostResult};
pub use host_api::{HostApiHandler, HostApiProvider};
pub use manifest::{
    NativeDriverEntry, NativeDriverManifest, NativeDriverProcessPolicy, NativeDriverProcessScope,
    NativeDriverRegistry, NativeDriverTransport,
};
pub use negotiation::{ExtensionSession, NegotiationConfig};
pub use process::{ProcessHandle, SpawnConfig, SpawnTransport};
pub use process_session::{
    DEFAULT_SESSION_REQUEST_TIMEOUT, DEFAULT_SESSION_SHUTDOWN_GRACE, NotificationReceiver,
    ProcessRpcSession, ProcessRpcSessionConfig,
};
pub use runtime::{
    ComponentExtensionRuntime, ExtensionRuntime, ExtensionRuntimeFactory, ExtensionRuntimeType,
    IpcExtensionRuntime,
};
pub use transport::{FramedTransport, ReadFramed, WriteFramed};

/// Default request timeout in milliseconds.
pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 30_000;

/// Default init handshake timeout in milliseconds.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

/// Default graceful shutdown window in milliseconds.
pub const DEFAULT_SHUTDOWN_GRACE_MS: u32 = 5_000;
