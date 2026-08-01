use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalIngressBudget {
    pub(super) max_pending_bytes: u32,
    pub(super) max_pending_chunks: usize,
    pub(super) max_pending_controls: usize,
}

impl TerminalIngressBudget {
    pub fn new(
        max_pending_bytes: u64,
        max_pending_chunks: usize,
        max_pending_controls: usize,
    ) -> Result<Self, TerminalIngressBudgetError> {
        if max_pending_bytes == 0 {
            return Err(TerminalIngressBudgetError::ZeroPendingBytes);
        }
        if max_pending_bytes > u64::from(u32::MAX) {
            return Err(TerminalIngressBudgetError::PendingBytesTooLarge {
                requested: max_pending_bytes,
                maximum: u32::MAX,
            });
        }
        if max_pending_chunks == 0 {
            return Err(TerminalIngressBudgetError::ZeroPendingChunks);
        }
        if max_pending_controls == 0 {
            return Err(TerminalIngressBudgetError::ZeroPendingControls);
        }
        Ok(Self {
            max_pending_bytes: max_pending_bytes as u32,
            max_pending_chunks,
            max_pending_controls,
        })
    }

    pub fn max_pending_bytes(self) -> usize {
        self.max_pending_bytes as usize
    }

    pub fn max_pending_chunks(self) -> usize {
        self.max_pending_chunks
    }

    pub fn max_pending_controls(self) -> usize {
        self.max_pending_controls
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalIngressBudgetError {
    ZeroPendingBytes,
    ZeroPendingChunks,
    ZeroPendingControls,
    PendingBytesTooLarge { requested: u64, maximum: u32 },
}

impl fmt::Display for TerminalIngressBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPendingBytes => formatter.write_str("pending byte budget must be non-zero"),
            Self::ZeroPendingChunks => formatter.write_str("pending chunk budget must be non-zero"),
            Self::ZeroPendingControls => {
                formatter.write_str("pending control budget must be non-zero")
            }
            Self::PendingBytesTooLarge { requested, maximum } => write!(
                formatter,
                "pending byte budget {requested} exceeds semaphore maximum {maximum}"
            ),
        }
    }
}

impl std::error::Error for TerminalIngressBudgetError {}

#[derive(PartialEq, Eq)]
pub enum TerminalDataSendError {
    Empty(Vec<u8>),
    Oversized { data: Vec<u8>, max_bytes: usize },
    Closed(Vec<u8>),
}

impl fmt::Debug for TerminalDataSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(_) => formatter.write_str("Empty(<redacted>)"),
            Self::Oversized { data, max_bytes } => formatter
                .debug_struct("Oversized")
                .field("bytes", &data.len())
                .field("max_bytes", max_bytes)
                .finish(),
            Self::Closed(data) => formatter
                .debug_tuple("Closed")
                .field(&format_args!("<redacted:{} bytes>", data.len()))
                .finish(),
        }
    }
}

impl fmt::Display for TerminalDataSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(_) => formatter.write_str("terminal ingress data must not be empty"),
            Self::Oversized { data, max_bytes } => write!(
                formatter,
                "terminal ingress chunk of {} bytes exceeds budget {max_bytes}",
                data.len()
            ),
            Self::Closed(_) => formatter.write_str("terminal ingress receiver is closed"),
        }
    }
}

impl std::error::Error for TerminalDataSendError {}

#[derive(PartialEq, Eq)]
pub enum TerminalControlSendError<C> {
    Closed(C),
}

impl<C> fmt::Debug for TerminalControlSendError<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Closed(<redacted control>)")
    }
}

impl<C> fmt::Display for TerminalControlSendError<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal ingress receiver is closed")
    }
}

impl<C> std::error::Error for TerminalControlSendError<C> {}

#[derive(PartialEq, Eq)]
pub enum TerminalIngressItem<C> {
    Data(Vec<u8>),
    Control(C),
}

impl<C> fmt::Debug for TerminalIngressItem<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(data) => formatter
                .debug_struct("Data")
                .field("bytes", &data.len())
                .finish(),
            Self::Control(_) => formatter.write_str("Control(<redacted>)"),
        }
    }
}

/// A data payload whose byte budget remains reserved until it is dropped.
///
/// The parser should hold this guard for the entire synchronous consumption
/// of the payload. This keeps the queue's byte budget aligned with the real
/// parser boundary instead of only the channel receive boundary.
pub struct TerminalIngressDataGuard {
    data: Vec<u8>,
    reservation: super::ByteReservation,
}

impl TerminalIngressDataGuard {
    pub(super) fn new(data: Vec<u8>, reservation: super::ByteReservation) -> Self {
        Self { data, reservation }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn into_vec(self) -> Vec<u8> {
        let Self { data, reservation } = self;
        drop(reservation);
        data
    }
}

impl fmt::Debug for TerminalIngressDataGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalIngressDataGuard")
            .field("bytes", &self.data.len())
            .finish()
    }
}

pub enum ReservedTerminalIngressItem<C> {
    Data(TerminalIngressDataGuard),
    Control(C),
}

impl<C> fmt::Debug for ReservedTerminalIngressItem<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(data) => formatter.debug_tuple("Data").field(data).finish(),
            Self::Control(_) => formatter.write_str("Control(<redacted>)"),
        }
    }
}
