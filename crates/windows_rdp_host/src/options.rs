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
