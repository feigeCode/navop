// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2023-2025 Jarkko Sakkinen

use crate::buffer::{Buffer, CapacityError};
use core::{
    cmp::min,
    fmt,
    ops::{Deref, DerefMut},
};

/// The capacity of the fixed-size `String` type.
const STRING_CAP: usize = 256;

/// A stack-allocated, fixed-capacity string.
///
/// This is a newtype wrapper around `Buffer<256>` to provide type safety and
/// string-specific operations.
#[derive(Eq)]
pub struct String(Buffer<STRING_CAP>);

impl Default for String {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for String {
    /// Creates a new `String` from a `&str`, truncating if necessary.
    fn from(s: &str) -> Self {
        let mut string = Self::new();
        let bytes = s.as_bytes();
        let len = min(bytes.len(), STRING_CAP);
        let mut end = len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }

        let truncated_bytes = &bytes[..end];
        string
            .extend_from_slice(truncated_bytes)
            .unwrap_or_default();
        string
    }
}

impl String {
    /// Creates a new, empty string.
    #[must_use]
    pub const fn new() -> Self {
        Self(Buffer::<STRING_CAP>::new())
    }

    /// Resets buffer length back to zero.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Copies bytes from a slice to the end of the buffer.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)`, if the capacity would be exceeded.
    pub fn extend_from_slice(&mut self, slice: &[u8]) -> Result<(), CapacityError> {
        self.0.extend_from_slice(slice)
    }
}

impl Deref for String {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for String {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<[u8]> for String {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<T: ?Sized> PartialEq<T> for String
where
    T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl fmt::Debug for String {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match core::str::from_utf8(self) {
            Ok(s) => f.write_str(s),
            Err(_) => f.debug_list().entries(self.iter()).finish(),
        }
    }
}

impl fmt::Display for String {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(core::str::from_utf8(self).unwrap_or(""))
    }
}
