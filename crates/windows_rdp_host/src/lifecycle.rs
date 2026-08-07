/// Observable lifecycle of the safe Windows RDP host facade.
///
/// This state describes Rust-side ownership and callback admission. It does
/// not claim that a future ActiveX control or RDP session has reached the same
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowsRdpHostLifecycle {
    /// The native handle is owned and new callbacks may be admitted.
    Open,
    /// Callback admission is closed and native unregistration or destruction
    /// must be retried.
    Closing,
    /// Callback unregistration and native handle destruction completed.
    Closed,
}
