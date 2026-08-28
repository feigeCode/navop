// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2026 Jarkko Sakkinen

//! Session-level events and internal phase names.

use crate::wire::SubpacketType;

/// A request for file data from the sender.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FileRequest {
    pub offset: u32,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SenderEvent {
    FileComplete,
    FileSkipped,
    SessionComplete,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ReceiverEvent {
    FileStart,
    FileComplete,
    Aborted,
}

/// Internal state for reading a subpacket byte-by-byte
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SubpacketPhase {
    Idle,
    /// Consuming the CR LF XON line trailer that separates a hex
    /// header from its data subpacket.
    SkipTrailer,
    Reading,
    Writing(SubpacketType),
    Crc(SubpacketType),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SenderPhase {
    WaitReceiverInit,
    ReadyForFile,
    WaitFilePos,
    NeedFileData,
    WaitFileAck,
    WaitFileDone,
    WaitFinish,
    FinishWriting,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ReceiverPhase {
    SessionBegin,
    SinitReadingData,
    FileBegin,
    FileReadingMetadata,
    FileAcceptPending,
    FileReadingSubpacket,
    FileWaitingSubpacket,
    SessionFinishWriting,
    SessionEnd,
}
