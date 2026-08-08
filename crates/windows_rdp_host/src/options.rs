/// Options used to allocate the opaque native host lifecycle handle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowsRdpHostOptions {
    generation: u64,
}

impl WindowsRdpHostOptions {
    pub const fn new(generation: u64) -> Self {
        Self { generation }
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// A caller-owned native parent window handle for the Windows-only ActiveX
/// presentation path.
///
/// The integer is never dereferenced by Rust. The caller must keep the
/// underlying window valid on the host owner/UI thread until the host has been
/// successfully closed or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowsRdpParentWindow(usize);

impl WindowsRdpParentWindow {
    /// Wraps a valid, non-null native parent window handle.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid parent window handle owned by the caller, and the
    /// caller must keep that window alive until the created host is destroyed.
    pub const unsafe fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> usize {
        self.0
    }
}
