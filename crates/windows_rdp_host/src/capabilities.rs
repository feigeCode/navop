/// Capabilities exposed by the currently loaded native host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsRdpHostCapabilities {
    available: bool,
}

impl WindowsRdpHostCapabilities {
    pub(crate) const fn new(available: bool) -> Self {
        Self { available }
    }

    /// Returns whether the versioned Windows native boundary is available.
    ///
    /// This does not claim that an ActiveX control or an RDP connection has
    /// been created. Those runtime capabilities are introduced by later
    /// implementation stages.
    pub const fn is_available(self) -> bool {
        self.available
    }
}
