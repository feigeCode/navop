// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Jarkko Sakkinen

//! Public protocol primitives for the poll/submit API.

/// Byte offset in a ZMODEM file transfer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Position(u32);

impl Position {
    /// Start of a file.
    pub const ZERO: Self = Self(0);

    /// Creates a file position from a byte offset.
    #[must_use]
    pub const fn new(offset: u32) -> Self {
        Self(offset)
    }

    /// Returns the byte offset.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for Position {
    fn from(offset: u32) -> Self {
        Self::new(offset)
    }
}

impl From<Position> for u32 {
    fn from(position: Position) -> Self {
        position.get()
    }
}

/// File metadata advertised by the sender.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo<'a> {
    /// Raw file name bytes as sent on the wire.
    pub name: &'a [u8],
    /// File size when known.
    pub size: Option<Position>,
}

impl<'a> FileInfo<'a> {
    /// Creates file metadata from raw name bytes and an optional size.
    #[must_use]
    pub const fn new(name: &'a [u8], size: Option<Position>) -> Self {
        Self { name, size }
    }
}

/// The next action the caller must perform, returned by
/// [`Sender::poll`](crate::Sender::poll) and
/// [`Receiver::poll`](crate::Receiver::poll).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Action<'a> {
    /// Write these bytes to the transport, then report progress with
    /// `wire_written`.
    WriteWire(&'a [u8]),
    /// Read at most `max_len` file bytes at `offset` and provide them with
    /// [`Sender::submit_file`](crate::Sender::submit_file). Senders only.
    ReadFile { offset: Position, max_len: usize },
    /// Persist these received bytes, then report progress with
    /// [`Receiver::file_written`](crate::Receiver::file_written). Receivers
    /// only.
    WriteFile(&'a [u8]),
    /// A protocol event occurred.
    Event(Event<'a>),
    /// No work is pending. Submit more wire input with `submit_wire`, or wait
    /// for a `timeout`.
    Idle,
}

/// Protocol-level event surfaced through [`Action::Event`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// A new incoming file started, with its advertised metadata.
    FileStarted(FileInfo<'a>),
    /// The current file completed.
    FileCompleted,
    /// The session completed successfully.
    SessionCompleted,
    /// The session was aborted.
    Aborted,
}
