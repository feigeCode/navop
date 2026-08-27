// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2025 Jarkko Sakkinen

//! ZMODEM protocol header, encoding, and frame definitions.

use crate::buffer::Buffer;
use crate::crc::{crc16_xmodem, crc32_iso_hdlc};
use crate::error::Error;
use crate::io::Write;
use crate::zdle;
use crate::{XON, ZDLE, ZPAD};
use bitflags::bitflags;

/// Buffer size with enough capacity for an escaped header
pub(crate) const HEADER_SIZE: usize = 32;
/// The size of the header payload (frame type + flags).
pub(crate) const HEADER_PAYLOAD_SIZE: usize = 5;

/// Controls which bytes are escaped when serializing binary ZMODEM frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EscapeMode {
    /// Escape the protocol's always-sensitive bytes.
    #[default]
    Standard,
    /// Escape every C0 control byte and its high-bit counterpart, as requested
    /// by the receiver's `ZRINIT.ESCCTL` capability.
    Control,
}

impl EscapeMode {
    const fn escaped(self, value: u8) -> u8 {
        if matches!(self, Self::Control) && value & 0x60 == 0 {
            value ^ 0x40
        } else {
            zdle::ZDLE_TABLE[value as usize]
        }
    }
}

pub(crate) const ZACK_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZACK, [0; 4]);
pub(crate) const ZDATA_HEADER: Header = Header::new(Encoding::ZBIN32, Frame::ZDATA, [0; 4]);
pub(crate) const ZEOF_HEADER: Header = Header::new(Encoding::ZBIN32, Frame::ZEOF, [0; 4]);
pub(crate) const ZFIN_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZFIN, [0; 4]);
pub(crate) const ZNAK_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZNAK, [0; 4]);
pub(crate) const ZRPOS_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZRPOS, [0; 4]);
pub(crate) const ZRQINIT_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZRQINIT, [0; 4]);

/// Data structure for holding a ZMODEM protocol header, which begins a frame,
/// and is followed optionally by a variable number of subpackets.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    encoding: Encoding,
    frame: Frame,
    flags: [u8; 4],
}

impl Header {
    /// Creates a new instance
    #[must_use]
    pub const fn new(encoding: Encoding, frame: Frame, flags: [u8; 4]) -> Self {
        Self {
            encoding,
            frame,
            flags,
        }
    }

    /// Returns `Encoding` of the frame
    #[must_use]
    pub const fn encoding(self) -> Encoding {
        self.encoding
    }

    /// Returns `Frame`, containing the frame type
    #[must_use]
    pub const fn frame(self) -> Frame {
        self.frame
    }

    /// Returns count for the frame types using this field
    #[must_use]
    pub const fn count(self) -> u32 {
        u32::from_le_bytes(self.flags)
    }

    /// Encodes and writes the header to the sink.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the sink is full
    pub(crate) fn write<P>(self, port: &mut P) -> Result<Option<()>, Error>
    where
        P: Write + ?Sized,
    {
        self.write_with_escape(port, EscapeMode::Standard)
    }

    /// Encodes and writes the header using the negotiated escape mode.
    pub(crate) fn write_with_escape<P>(
        self,
        port: &mut P,
        escape_mode: EscapeMode,
    ) -> Result<Option<()>, Error>
    where
        P: Write + ?Sized,
    {
        if write_header_start(port, self.encoding)?.is_none() {
            return Ok(None);
        }

        let mut out: Buffer<HEADER_SIZE> = Buffer::new();
        out.push(self.frame as u8).map_err(|_| Error::OutOfMemory)?;
        out.extend_from_slice(&self.flags)
            .map_err(|_| Error::OutOfMemory)?;

        let mut crc = [0u8; 4];
        let crc_len = make_crc(&out, &mut crc, self.encoding);
        out.extend_from_slice(&crc[..crc_len])
            .map_err(|_| Error::OutOfMemory)?;

        if self.encoding == Encoding::ZHEX {
            let mut hex_buf = [0u8; HEADER_SIZE];
            let len = out.len() * 2;
            let hex = &mut hex_buf.get_mut(..len).ok_or(Error::UnexpectedEof)?;
            hex::encode_to_slice(&out, hex).map_err(|_| Error::OutOfMemory)?;
            if write_slice_escaped(port, hex, escape_mode)?.is_none() {
                return Ok(None);
            }

            if write_header_end_hex(port, self.frame)?.is_none() {
                return Ok(None);
            }
        } else if write_slice_escaped(port, &out, escape_mode)?.is_none() {
            return Ok(None);
        }

        Ok(Some(()))
    }

    /// Returns a new instance with the flags substitude with a count
    /// for the frame types using this field.
    #[must_use]
    pub const fn with_count(self, count: u32) -> Self {
        Header::new(self.encoding, self.frame, count.to_le_bytes())
    }

    /// Returns the serialized size of the header payload (payload + CRC)
    pub(crate) const fn read_size(encoding: Encoding) -> usize {
        match encoding {
            Encoding::ZBIN => HEADER_PAYLOAD_SIZE + 2,
            Encoding::ZBIN32 => HEADER_PAYLOAD_SIZE + 4,
            Encoding::ZHEX => (HEADER_PAYLOAD_SIZE + 2) * 2,
        }
    }
}

/// The ZMODEM protocol frame encoding
#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoding {
    ZBIN = 0x41,
    ZHEX = 0x42,
    ZBIN32 = 0x43,
}

impl TryFrom<u8> for Encoding {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x41 => Ok(Encoding::ZBIN),
            0x42 => Ok(Encoding::ZHEX),
            0x43 => Ok(Encoding::ZBIN32),
            _ => Err(Error::MalformedEncoding(value)),
        }
    }
}

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
/// Frame types
pub enum Frame {
    /// Request receive init
    ZRQINIT = 0,
    /// Receiver capabilities and packet size
    ZRINIT = 1,
    /// Send init sequence (optional)
    ZSINIT = 2,
    /// ACK to above
    ZACK = 3,
    /// File name from sender
    ZFILE = 4,
    /// To sender: skip this file
    ZSKIP = 5,
    /// Last packet was garbled
    ZNAK = 6,
    /// Abort batch transfers
    ZABORT = 7,
    /// Finish session
    ZFIN = 8,
    /// Resume data trans at this position
    ZRPOS = 9,
    /// Data packet(s) follow
    ZDATA = 10,
    /// End of file
    ZEOF = 11,
    /// Fatal Read or Write error Detected
    ZFERR = 12,
    /// Request for file CRC and response
    ZCRC = 13,
    /// Receiver's Challenge
    ZCHALLENGE = 14,
    /// Request is complete
    ZCOMPL = 15,
    /// Other end canned session with CAN*5
    ZCAN = 16,
    /// Request for free bytes on filesystem
    ZFREECNT = 17,
    /// Command from sending program
    ZCOMMAND = 18,
    /// Output to standard error, data follows
    ZSTDERR = 19,
}

impl TryFrom<u8> for Frame {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Frame::ZRQINIT),
            1 => Ok(Frame::ZRINIT),
            2 => Ok(Frame::ZSINIT),
            3 => Ok(Frame::ZACK),
            4 => Ok(Frame::ZFILE),
            5 => Ok(Frame::ZSKIP),
            6 => Ok(Frame::ZNAK),
            7 => Ok(Frame::ZABORT),
            8 => Ok(Frame::ZFIN),
            9 => Ok(Frame::ZRPOS),
            10 => Ok(Frame::ZDATA),
            11 => Ok(Frame::ZEOF),
            12 => Ok(Frame::ZFERR),
            13 => Ok(Frame::ZCRC),
            14 => Ok(Frame::ZCHALLENGE),
            15 => Ok(Frame::ZCOMPL),
            16 => Ok(Frame::ZCAN),
            17 => Ok(Frame::ZFREECNT),
            18 => Ok(Frame::ZCOMMAND),
            19 => Ok(Frame::ZSTDERR),
            _ => Err(Error::MalformedFrame(value)),
        }
    }
}

bitflags! {
    /// `ZRINIT` flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Zrinit: u8 {
        /// Can send and receive in full-duplex
        const CANFDX = 0x01;
        /// Can receive data in parallel with disk I/O
        const CANOVIO = 0x02;
        /// Can send a break signal
        const CANBRK = 0x04;
        /// Can decrypt
        const CANCRY = 0x08;
        /// Can uncompress
        const CANLZW = 0x10;
        /// Can use 32-bit frame check
        const CANFC32 = 0x20;
        /// Expects control character to be escaped
        const ESCCTL = 0x40;
        /// Expects 8th bit to be escaped
        const ESC8 = 0x80;
    }
}

fn write_header_start<P>(port: &mut P, encoding: Encoding) -> Result<Option<()>, Error>
where
    P: Write + ?Sized,
{
    if port.write_byte(ZPAD)?.is_none() {
        return Ok(None);
    }
    if encoding == Encoding::ZHEX && port.write_byte(ZPAD)?.is_none() {
        return Ok(None);
    }
    if port.write_byte(ZDLE)?.is_none() {
        return Ok(None);
    }
    port.write_byte(encoding as u8)
}

fn write_header_end_hex<P>(port: &mut P, frame: Frame) -> Result<Option<()>, Error>
where
    P: Write + ?Sized,
{
    if port.write_byte(b'\r')?.is_none() {
        return Ok(None);
    }
    if port.write_byte(b'\n')?.is_none() {
        return Ok(None);
    }
    if frame != Frame::ZACK && frame != Frame::ZFIN && port.write_byte(XON)?.is_none() {
        return Ok(None);
    }
    Ok(Some(()))
}

fn make_crc(data: &[u8], out: &mut [u8], encoding: Encoding) -> usize {
    if encoding == Encoding::ZBIN32 {
        let crc = crc32_iso_hdlc(data).to_le_bytes();
        out[..4].copy_from_slice(&crc[..4]);
        4
    } else {
        let crc = crc16_xmodem(data).to_be_bytes();
        out[..2].copy_from_slice(&crc[..2]);
        2
    }
}

pub(crate) fn write_slice_escaped<P>(
    port: &mut P,
    buf: &[u8],
    escape_mode: EscapeMode,
) -> Result<Option<()>, Error>
where
    P: Write + ?Sized,
{
    for value in buf {
        if write_byte_escaped(port, *value, escape_mode)?.is_none() {
            return Ok(None);
        }
    }

    Ok(Some(()))
}

pub(crate) fn write_byte_escaped<P>(
    port: &mut P,
    value: u8,
    escape_mode: EscapeMode,
) -> Result<Option<()>, Error>
where
    P: Write + ?Sized,
{
    let escaped = escape_mode.escaped(value);
    if escaped != value && port.write_byte(ZDLE)?.is_none() {
        return Ok(None);
    }
    port.write_byte(escaped)
}
