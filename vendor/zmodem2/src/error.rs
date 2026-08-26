// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2023-2025 Jarkko Sakkinen

/// Top-level error type.
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error {
    #[cfg_attr(feature = "std", error("malformed encoding type: {0:#02x}"))]
    MalformedEncoding(u8),
    #[cfg_attr(feature = "std", error("malformed file size"))]
    MalformedFileSize,
    #[cfg_attr(feature = "std", error("malformed filename"))]
    MalformedFileName,
    #[cfg_attr(feature = "std", error("malformed frame type: {0:#02x}"))]
    MalformedFrame(u8),
    #[cfg_attr(feature = "std", error("malformed header"))]
    MalformedHeader,
    #[cfg_attr(feature = "std", error("malformed packet type: {0:#02x}"))]
    MalformedPacket(u8),
    #[cfg_attr(feature = "std", error("invalid state"))]
    InvalidState,
    #[cfg_attr(feature = "std", error("backpressure"))]
    Backpressure,
    #[cfg_attr(feature = "std", error("out of memory"))]
    OutOfMemory,
    #[cfg_attr(feature = "std", error("unexpected CRC-16"))]
    UnexpectedCrc16,
    #[cfg_attr(feature = "std", error("unexpected CRC-32"))]
    UnexpectedCrc32,
    #[cfg_attr(feature = "std", error("unexpected EOF"))]
    UnexpectedEof,
    #[cfg_attr(feature = "std", error("unsupported feature"))]
    UnsupportedFeature,
}
