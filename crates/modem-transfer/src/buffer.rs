// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2023-2025 Jarkko Sakkinen

//! A simple stack-allocated buffer with a fixed capacity for staging data coming
//! from the serial link.

use core::fmt;
use core::ops::{Deref, DerefMut};

/// An error indicating a buffer's capacity was exceeded.
#[derive(Debug, PartialEq)]
pub struct CapacityError;

/// A buffer type for incoming and outgoing and other flyaway data.
pub struct Buffer<const CAP: usize> {
    bytes: [u8; CAP],
    len: usize,
}

impl<const CAP: usize> Default for Buffer<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> Buffer<CAP> {
    /// Creates a new instance.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [0; CAP],
            len: 0,
        }
    }

    /// Returns the number of bytes stored in the buffer.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns the capacity of the buffer in bytes.
    #[must_use]
    #[allow(clippy::unused_self)]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    /// Returns `true` if the buffer is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resets buffer length back to zero.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Appends a byte to the buffer.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)`, if the capacity would be exceeded.
    pub fn push(&mut self, value: u8) -> Result<(), CapacityError> {
        if self.len < CAP {
            self.bytes[self.len] = value;
            self.len += 1;
            Ok(())
        } else {
            Err(CapacityError)
        }
    }

    /// Copies bytes from a slice to the end of the buffer.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)`, if the capacity would be exceeded.
    pub fn extend_from_slice(&mut self, slice: &[u8]) -> Result<(), CapacityError> {
        if self.len + slice.len() > CAP {
            return Err(CapacityError);
        }
        let end = self.len + slice.len();
        self.bytes[self.len..end].copy_from_slice(slice);
        self.len = end;
        Ok(())
    }
}

impl<const CAP: usize> fmt::Write for Buffer<CAP> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.extend_from_slice(s.as_bytes()).map_err(|_| fmt::Error)
    }
}

impl<const CAP: usize> Deref for Buffer<CAP> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes[0..self.len]
    }
}

impl<const CAP: usize> DerefMut for Buffer<CAP> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bytes[0..self.len]
    }
}

impl<const CAP: usize> AsRef<[u8]> for Buffer<CAP> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[0..self.len]
    }
}

impl<const CAP: usize, T: ?Sized> PartialEq<T> for Buffer<CAP>
where
    T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<const CAP: usize> Eq for Buffer<CAP> {}
