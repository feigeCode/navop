// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2023-2025 Jarkko Sakkinen

//! Internal traits for non-blocking byte I/O over the crate's in-memory
//! slice reader and buffer writer.

use crate::Error;

/// Write operations.
pub(crate) trait Write {
    /// Writes the entire buffer to the sink. Returns `Ok(None)` if no bytes
    /// were written.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the sink is full
    fn write_all(&mut self, buf: &[u8]) -> Result<Option<()>, Error>;

    /// Writes a single byte to the sink. Returns `Ok(None)` if no byte was
    /// written.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the sink is full
    fn write_byte(&mut self, value: u8) -> Result<Option<()>, Error> {
        self.write_all(&[value])
    }
}

/// Read operations.
pub(crate) trait Read {
    /// Reads a single byte from the source. Returns `Ok(None)` if no byte is
    /// available.
    ///
    /// # Errors
    ///
    /// Propagates protocol errors from the source.
    fn read_byte(&mut self) -> Result<Option<u8>, Error>;
}
