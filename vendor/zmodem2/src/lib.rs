// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2026 Jarkko Sakkinen

//! ZMODEM file transfer protocol library. [`Sender`] and [`Receiver`] are
//! caller-driven state machines for sending and receiving files with the
//! ZMODEM protocol. The crate is `no_std` compatible and heapless.
//!
//! The caller owns all I/O. The loop is the same for both roles:
//! 1. Create a [`Sender`] or [`Receiver`].
//! 2. Call `poll()` to get the next [`Action`]:
//!    - [`Action::WriteWire`] — write the bytes to the transport, then call
//!      `wire_written(n)`.
//!    - [`Action::ReadFile`] (sender) — read the requested file bytes and
//!      provide them with [`Sender::submit_file`].
//!    - [`Action::WriteFile`] (receiver) — persist the bytes, then call
//!      [`Receiver::file_written`].
//!    - [`Action::Event`] — handle a protocol [`Event`].
//!    - [`Action::Idle`] — feed incoming transport bytes with `submit_wire()`,
//!      or call `timeout()` if none arrive.
//! 3. The sender offers files with [`Sender::start_file`] and ends the session
//!    with [`Sender::finish`].

#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::result_large_err)]
#![cfg_attr(not(feature = "std"), no_std)]

mod api;
mod buffer;
mod crc;
mod error;
mod file;
mod header;
mod io;
mod receiver;
mod sender;
mod session;
mod string;
#[cfg(test)]
mod tests;
mod transmission;
mod wire;
mod zdle;

pub use api::*;
pub use error::*;
pub use transmission::*;

pub(crate) const ZPAD: u8 = b'*';
pub(crate) const ZDLE: u8 = 0x18;
pub(crate) const XON: u8 = 0x11;
