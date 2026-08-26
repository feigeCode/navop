// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2026 Jarkko Sakkinen

//! ZMODEM receiver state machine.

use crate::api::{Action, Event, FileInfo, Position};
use crate::buffer::Buffer;
use crate::error::Error;
use crate::file::parse_file_size;
use crate::header::{Encoding, Frame, Header, ZACK_HEADER, ZFIN_HEADER, ZNAK_HEADER, ZRPOS_HEADER};
use crate::io::Read;
use crate::session::{ReceiverEvent, ReceiverPhase, SubpacketPhase};
use crate::string::String;
use crate::wire::{
    BufferWriter, HeaderReader, RxCrc, SUBPACKET_MAX_SIZE, SliceReader, SubpacketType,
    WIRE_BUF_SIZE, write_zrinit,
};
use crate::{ZDLE, zdle};
use core::cmp::min;

const RECEIVER_EVENT_QUEUE_CAP: usize = 4;

/// ZMODEM receiver state machine.
// The receiver tracks several independent one-shot conditions (escape
// continuation, overlapped I/O, manual accept, active file) that don't
// collapse into a single enum without losing clarity.
#[allow(clippy::struct_excessive_bools)]
pub struct Receiver {
    state: ReceiverPhase,
    count: u32,
    file_name: String,
    file_size: u32,
    buf: Buffer<SUBPACKET_MAX_SIZE>,
    buf_write_offset: usize,
    data_encoding: Encoding,
    header_reader: HeaderReader,
    subpacket_state: SubpacketPhase,
    subpacket_escape_pending: bool,
    crc: RxCrc,
    outgoing: Buffer<WIRE_BUF_SIZE>,
    outgoing_offset: usize,
    pending_events: [Option<ReceiverEvent>; RECEIVER_EVENT_QUEUE_CAP],
    pending_event_head: usize,
    pending_event_len: usize,
    buffer_len: u16,
    overlapped_io: bool,
    manual_accept: bool,
    zrpos_retries: u8,
    file_active: bool,
}

/// Consecutive corrupt data subpackets, without any forward progress in
/// between, tolerated before the transfer is abandoned. Each one costs a
/// ZRPOS rewind; a link that cannot deliver a single clean subpacket in
/// this many attempts is failing, so surfacing the CRC error is more
/// honest than looping forever. lrzsz's `rz` gives up on a similar
/// garbage/retry threshold.
pub(crate) const MAX_ZRPOS_RETRIES: u8 = 10;

impl Receiver {
    /// Create a new receiver instance.
    ///
    /// Advertises a buffer length of one subpacket (1024 bytes) and no
    /// overlapped I/O: the sender pauses for an acknowledgement after
    /// each buffer's worth of data, which suits constrained targets
    /// that cannot drain the wire while persisting file data. See
    /// [`Receiver::with_flow_control`] to lift that pacing.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the outgoing buffer cannot hold the handshake
    pub fn new() -> Result<Self, Error> {
        let buffer_len =
            u16::try_from(SUBPACKET_MAX_SIZE).map_err(|_| Error::UnsupportedFeature)?;
        Self::with_flow_control(buffer_len, false)
    }

    /// Create a receiver advertising explicit flow-control
    /// capabilities in its ZRINIT handshake.
    ///
    /// `buffer_len` is the receiver buffer length the sender must
    /// respect: it will not transmit more than this many bytes without
    /// waiting for an acknowledgement. Zero advertises nonstop I/O.
    /// `overlapped_io` advertises `CANOVIO` (storage is written while
    /// data is being received). Senders such as lrzsz's `sz` require
    /// both (a zero buffer length and `CANOVIO`) before they stream
    /// continuously; anything less inserts one round-trip wait per
    /// buffer of data, which dominates transfer time on links with
    /// real latency.
    ///
    /// Callers that pump [`Receiver::submit_wire`] from a reliable,
    /// flow-controlled transport (TCP, SSH, a pipe) and persist file
    /// data promptly should prefer `with_flow_control(0, true)`; the
    /// conservative [`Receiver::new`] default exists for targets where
    /// wire input can genuinely overrun the consumer.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the outgoing buffer cannot hold the handshake
    pub fn with_flow_control(buffer_len: u16, overlapped_io: bool) -> Result<Self, Error> {
        let mut receiver = Self {
            state: ReceiverPhase::SessionBegin,
            count: 0,
            file_name: String::new(),
            file_size: 0,
            buf: Buffer::<SUBPACKET_MAX_SIZE>::new(),
            buf_write_offset: 0,
            data_encoding: Encoding::ZBIN,
            header_reader: HeaderReader::new(),
            subpacket_state: SubpacketPhase::Idle,
            subpacket_escape_pending: false,
            crc: RxCrc::new(),
            outgoing: Buffer::<WIRE_BUF_SIZE>::new(),
            outgoing_offset: 0,
            pending_events: [None; RECEIVER_EVENT_QUEUE_CAP],
            pending_event_head: 0,
            pending_event_len: 0,
            buffer_len,
            overlapped_io,
            manual_accept: false,
            zrpos_retries: 0,
            file_active: false,
        };
        receiver.queue_zrinit()?;
        Ok(receiver)
    }

    /// Enables or disables manual file acceptance.
    ///
    /// In the default automatic mode every announced file is accepted
    /// from offset zero as soon as its ZFILE metadata parses. In
    /// manual mode the receiver instead pauses after emitting
    /// [`Event::FileStarted`] and waits for the caller to decide:
    /// [`Receiver::accept_file_at`] requests the file from a given
    /// offset (resuming an existing partial download), and
    /// [`Receiver::skip_file`] declines it (ZSKIP). While a decision
    /// is pending, [`Receiver::poll`] returns [`Action::Idle`] and no
    /// wire bytes are produced.
    pub fn set_manual_file_accept(&mut self, manual: bool) {
        self.manual_accept = manual;
    }

    /// Accepts the file announced by the pending [`Event::FileStarted`]
    /// and asks the sender to start at `offset` (ZRPOS).
    ///
    /// Zero requests the whole file; a nonzero offset resumes a
    /// partial transfer, and the caller is responsible for appending
    /// the incoming data to its existing `offset` bytes.
    ///
    /// # Errors
    ///
    /// * [`InvalidState`](crate::Error::InvalidState) when no file is awaiting acceptance
    /// * [`Backpressure`](crate::Error::Backpressure) when outgoing bytes are still
    ///   pending (drain and retry; the acceptance state is unchanged)
    pub fn accept_file_at(&mut self, offset: u32) -> Result<(), Error> {
        if self.state != ReceiverPhase::FileAcceptPending {
            return Err(Error::InvalidState);
        }
        self.queue_zrpos(offset)?;
        self.count = offset;
        self.state = ReceiverPhase::FileBegin;
        // The sender's response may be ZDATA (more bytes) or, when the
        // resume offset already sits at EOF, an immediate ZEOF with no
        // data frame. Mark the file active so the ZEOF handler accepts
        // that completion in FileBegin (see handle_header).
        self.file_active = true;
        Ok(())
    }

    /// Declines the file announced by the pending [`Event::FileStarted`]
    /// (ZSKIP); the sender moves on to its next file or finishes.
    ///
    /// # Errors
    ///
    /// * [`InvalidState`](crate::Error::InvalidState) when no file is awaiting acceptance
    /// * [`Backpressure`](crate::Error::Backpressure) when outgoing bytes are still
    ///   pending (drain and retry; the acceptance state is unchanged)
    pub fn skip_file(&mut self) -> Result<(), Error> {
        if self.state != ReceiverPhase::FileAcceptPending {
            return Err(Error::InvalidState);
        }
        self.queue_header(Header::new(Encoding::ZHEX, Frame::ZSKIP, [0; 4]))?;
        self.state = ReceiverPhase::FileBegin;
        self.file_active = false;
        Ok(())
    }

    /// Submits incoming wire data into the state machine.
    ///
    /// Returns the number of bytes consumed.
    ///
    /// # Errors
    ///
    /// * [`UnexpectedCrc16`](crate::Error::UnexpectedCrc16) or
    ///   [`UnexpectedCrc32`](crate::Error::UnexpectedCrc32) when corrupted data has been detected
    pub fn submit_wire(&mut self, input: &[u8]) -> Result<usize, Error> {
        let mut reader = SliceReader::new(input);

        loop {
            if self.blocked() {
                break;
            }

            let before = reader.consumed();

            if matches!(
                self.state,
                ReceiverPhase::FileReadingSubpacket
                    | ReceiverPhase::FileReadingMetadata
                    | ReceiverPhase::SinitReadingData
            ) {
                match self.process_subpacket(&mut reader) {
                    Ok(Some(())) => {
                        if self.blocked() {
                            break;
                        }
                        if reader.consumed() == before {
                            break;
                        }
                        continue;
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e),
                }
            }

            let header = match self.header_reader.read(&mut reader) {
                Ok(Some(header)) => header,
                Ok(None) => break,
                Err(e) => {
                    let _ = self.queue_nak();
                    return Err(e);
                }
            };

            self.handle_header(header)?;

            if self.pending_events_full() {
                break;
            }

            if reader.consumed() == before || reader.consumed() == input.len() {
                break;
            }
        }

        Ok(reader.consumed())
    }

    /// Returns pending outgoing bytes.
    fn drain_outgoing(&self) -> &[u8] {
        &self.outgoing[self.outgoing_offset..]
    }

    /// Reports that `n` outgoing bytes from the last [`Action::WriteWire`] were
    /// written to the transport.
    pub fn wire_written(&mut self, n: usize) {
        let remaining = self.outgoing.len().saturating_sub(self.outgoing_offset);
        let n = min(n, remaining);
        self.outgoing_offset += n;
        if self.outgoing_offset >= self.outgoing.len() {
            self.outgoing.clear();
            self.outgoing_offset = 0;
        }
    }

    /// Returns pending file data bytes.
    fn drain_file(&self) -> &[u8] {
        match self.subpacket_state {
            SubpacketPhase::Writing(_) => &self.buf[self.buf_write_offset..],
            _ => &[],
        }
    }

    /// Reports that `n` file bytes from the last [`Action::WriteFile`] were
    /// persisted to storage.
    ///
    /// # Errors
    ///
    /// * [`Backpressure`](crate::Error::Backpressure) when outgoing bytes are still pending
    pub fn file_written(&mut self, n: usize) -> Result<(), Error> {
        let SubpacketPhase::Writing(packet) = self.subpacket_state else {
            return Ok(());
        };

        let remaining = self.buf.len().saturating_sub(self.buf_write_offset);
        let n = min(n, remaining);
        self.buf_write_offset = self
            .buf_write_offset
            .checked_add(n)
            .ok_or(Error::OutOfMemory)?;

        if self.buf_write_offset < self.buf.len() {
            return Ok(());
        }

        self.finish_subpacket(packet)
    }

    /// Signals that the protocol response timeout expired.
    ///
    /// While waiting for a session or file to begin, this re-queues the
    /// `ZRINIT` handshake.
    ///
    /// # Errors
    ///
    /// * [`Backpressure`](crate::Error::Backpressure) when outgoing bytes are still pending
    pub fn timeout(&mut self) -> Result<(), Error> {
        if matches!(
            self.state,
            ReceiverPhase::SessionBegin | ReceiverPhase::FileBegin
        ) && !self.outgoing()
        {
            self.queue_zrinit()?;
        }
        Ok(())
    }

    /// Aborts the current session.
    ///
    /// # Errors
    ///
    /// * [`OutOfMemory`](crate::Error::OutOfMemory) when the event queue is full
    pub fn abort(&mut self) -> Result<(), Error> {
        self.state = ReceiverPhase::SessionEnd;
        self.push_event(ReceiverEvent::Aborted)
    }

    /// Returns the next action the caller must perform.
    ///
    /// Pending events take priority, followed by outgoing wire bytes, then
    /// received file bytes, and finally [`Action::Idle`] when there is no
    /// immediate work.
    pub fn poll(&mut self) -> Action<'_> {
        if let Some(event) = self.pop_event() {
            return Action::Event(match event {
                ReceiverEvent::FileStart => Event::FileStarted(FileInfo::new(
                    &self.file_name,
                    Some(Position::new(self.file_size)),
                )),
                ReceiverEvent::FileComplete => Event::FileCompleted,
                ReceiverEvent::SessionComplete => Event::SessionCompleted,
                ReceiverEvent::Aborted => Event::Aborted,
            });
        }

        if self.outgoing() {
            return Action::WriteWire(self.drain_outgoing());
        }

        if !self.drain_file().is_empty() {
            return Action::WriteFile(self.drain_file());
        }

        Action::Idle
    }

    fn outgoing(&self) -> bool {
        self.outgoing_offset < self.outgoing.len()
    }

    /// Returns `true` when no further wire input can be consumed until the
    /// caller drains outgoing bytes, persists file data, or pops events.
    fn blocked(&self) -> bool {
        self.outgoing() || !self.drain_file().is_empty() || self.pending_events_full()
    }

    fn pending_events_full(&self) -> bool {
        self.pending_event_len >= RECEIVER_EVENT_QUEUE_CAP
    }

    fn push_event(&mut self, event: ReceiverEvent) -> Result<(), Error> {
        if self.pending_events_full() {
            return Err(Error::OutOfMemory);
        }
        let index = (self.pending_event_head + self.pending_event_len) % RECEIVER_EVENT_QUEUE_CAP;
        self.pending_events[index] = Some(event);
        self.pending_event_len += 1;
        Ok(())
    }

    fn pop_event(&mut self) -> Option<ReceiverEvent> {
        if self.pending_event_len == 0 {
            return None;
        }
        let event = self.pending_events[self.pending_event_head].take();
        self.pending_event_head = (self.pending_event_head + 1) % RECEIVER_EVENT_QUEUE_CAP;
        self.pending_event_len -= 1;
        event
    }

    fn queue_writer(&mut self) -> Result<BufferWriter<'_, WIRE_BUF_SIZE>, Error> {
        if self.outgoing() {
            return Err(Error::Backpressure);
        }
        Ok(BufferWriter::new(&mut self.outgoing))
    }

    fn queue_header(&mut self, header: Header) -> Result<(), Error> {
        let mut writer = self.queue_writer()?;
        if header.write(&mut writer)?.is_none() {
            return Err(Error::OutOfMemory);
        }
        Ok(())
    }

    fn queue_zrinit(&mut self) -> Result<(), Error> {
        let (buffer_len, overlapped_io) = (self.buffer_len, self.overlapped_io);
        let mut writer = self.queue_writer()?;
        if write_zrinit(&mut writer, buffer_len, overlapped_io)?.is_none() {
            return Err(Error::OutOfMemory);
        }
        Ok(())
    }

    fn queue_zrpos(&mut self, count: u32) -> Result<(), Error> {
        self.queue_header(ZRPOS_HEADER.with_count(count))
    }

    fn queue_zack(&mut self) -> Result<(), Error> {
        self.queue_header(ZACK_HEADER.with_count(self.count))
    }

    fn queue_zfin(&mut self) -> Result<(), Error> {
        self.queue_header(ZFIN_HEADER)
    }

    fn queue_nak(&mut self) -> Result<(), Error> {
        self.queue_header(ZNAK_HEADER)
    }

    fn handle_header(&mut self, header: Header) -> Result<(), Error> {
        match header.frame() {
            Frame::ZRQINIT | Frame::ZDATA if self.state == ReceiverPhase::SessionBegin => {
                self.queue_zrinit()?;
            }
            Frame::ZSINIT if self.state == ReceiverPhase::SessionBegin => {
                // ZSINIT (e.g. lrzsz's `sz -e`) is followed by a data
                // subpacket with the attn string; read it so it is not
                // misparsed as headers, then acknowledge (see the
                // SinitReadingData completion). lrzsz sends the header
                // as ZHEX: there is no hex data encoding on the wire,
                // the subpacket that follows is binary with CRC16, and
                // the hex line trailer before it must be skipped.
                self.data_encoding = match header.encoding() {
                    Encoding::ZHEX => Encoding::ZBIN,
                    other => other,
                };
                self.state = ReceiverPhase::SinitReadingData;
                self.subpacket_state = if header.encoding() == Encoding::ZHEX {
                    SubpacketPhase::SkipTrailer
                } else {
                    SubpacketPhase::Reading
                };
                self.subpacket_escape_pending = false;
                self.crc.reset();
                self.buf.clear();
                self.buf_write_offset = 0;
            }
            Frame::ZFILE
                if matches!(
                    self.state,
                    ReceiverPhase::SessionBegin | ReceiverPhase::FileBegin
                ) =>
            {
                self.data_encoding = header.encoding();
                self.state = ReceiverPhase::FileReadingMetadata;
                self.subpacket_state = SubpacketPhase::Reading;
                self.subpacket_escape_pending = false;
                self.crc.reset();
                self.buf.clear();
                self.buf_write_offset = 0;
            }
            Frame::ZDATA
                if matches!(
                    self.state,
                    ReceiverPhase::FileBegin | ReceiverPhase::FileWaitingSubpacket
                ) =>
            {
                if header.count() != self.count {
                    self.queue_zrpos(self.count)?;
                    return Ok(());
                }
                self.data_encoding = header.encoding();
                self.state = ReceiverPhase::FileReadingSubpacket;
                self.subpacket_state = SubpacketPhase::Reading;
                self.subpacket_escape_pending = false;
                self.crc.reset();
                self.buf.clear();
                self.buf_write_offset = 0;
            }
            Frame::ZEOF
                if (self.state == ReceiverPhase::FileWaitingSubpacket
                    || (self.state == ReceiverPhase::FileBegin && self.file_active))
                    && header.count() == self.count =>
            {
                // FileBegin is reached both while a file is active (right
                // after accept_file_at / auto-accept, before any ZDATA,
                // where a resume at EOF or an empty file yields an
                // immediate ZEOF) and between files (after a prior
                // completion). The file_active guard keeps a stray or
                // resent ZEOF in the between-files state from emitting a
                // second FileComplete.
                self.queue_zrinit()?;
                self.state = ReceiverPhase::FileBegin;
                self.file_active = false;
                self.push_event(ReceiverEvent::FileComplete)?;
            }
            Frame::ZABORT | Frame::ZCAN => {
                self.state = ReceiverPhase::SessionEnd;
                self.push_event(ReceiverEvent::Aborted)?;
            }
            Frame::ZFIN
                if matches!(
                    self.state,
                    ReceiverPhase::FileWaitingSubpacket | ReceiverPhase::FileBegin
                ) =>
            {
                self.queue_zfin()?;
                self.state = ReceiverPhase::SessionEnd;
                self.push_event(ReceiverEvent::SessionComplete)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Parses the file info buffer after a ZFILE subpacket is received.
    fn parse_zfile_buf(&mut self) -> Result<(), Error> {
        let payload = &self.buf;
        let mut fields = payload.split(|&b| b == b'\0');

        let file_name_bytes = fields.next().ok_or(Error::MalformedFileName)?;
        if file_name_bytes.is_empty() {
            return Err(Error::MalformedFileName);
        }

        self.file_name.clear();
        self.file_name
            .extend_from_slice(file_name_bytes)
            .map_err(|_| Error::OutOfMemory)?;

        if let Some(size_str_bytes) = fields.next() {
            let size_field_bytes = size_str_bytes
                .split(|&b| b == b' ')
                .next()
                .unwrap_or_default();

            self.file_size = parse_file_size(size_field_bytes)?;
        } else {
            self.file_size = 0;
        }

        self.count = 0;
        Ok(())
    }

    /// Handles the byte after a `ZDLE`: either a subpacket terminator
    /// or an escaped data byte.
    fn receive_subpacket_followup_byte(&mut self, byte: u8) -> Result<Option<()>, Error> {
        if let Ok(packet) = SubpacketType::try_from(byte) {
            self.crc.update(packet as u8, self.data_encoding);
            self.subpacket_state = SubpacketPhase::Crc(packet);
        } else {
            let unescaped = zdle::UNZDLE_TABLE[byte as usize];
            self.buf.push(unescaped).map_err(|_| Error::OutOfMemory)?;
            self.crc.update(unescaped, self.data_encoding);
        }
        Ok(Some(()))
    }

    /// Handles a plain (non-escape-continuation) subpacket byte.
    fn receive_subpacket_plain_byte<P>(
        &mut self,
        port: &mut P,
        byte: u8,
    ) -> Result<Option<()>, Error>
    where
        P: Read + ?Sized,
    {
        if byte == ZDLE {
            let Some(next) = port.read_byte()? else {
                self.subpacket_escape_pending = true;
                return Ok(None);
            };
            return self.receive_subpacket_followup_byte(next);
        }
        self.buf.push(byte).map_err(|_| Error::OutOfMemory)?;
        self.crc.update(byte, self.data_encoding);
        Ok(Some(()))
    }

    /// Handles reading a single byte for the `SubpacketPhase::Reading` state.
    fn receive_subpacket_data_byte<P>(&mut self, port: &mut P) -> Result<Option<()>, Error>
    where
        P: Read + ?Sized,
    {
        if self.subpacket_escape_pending {
            let Some(byte) = port.read_byte()? else {
                return Ok(None);
            };
            self.subpacket_escape_pending = false;
            return self.receive_subpacket_followup_byte(byte);
        }

        let Some(byte) = port.read_byte()? else {
            return Ok(None);
        };
        self.receive_subpacket_plain_byte(port, byte)
    }

    fn process_subpacket<P>(&mut self, port: &mut P) -> Result<Option<()>, Error>
    where
        P: Read + ?Sized,
    {
        match self.subpacket_state {
            SubpacketPhase::SkipTrailer => {
                // A hex header ends with a CR LF (possibly with the
                // high bit set) XON line trailer that sits between the
                // header and its data subpacket; those bytes are
                // framing, not payload. The first payload byte flips
                // to `Reading` and is processed in place.
                loop {
                    let Some(byte) = port.read_byte()? else {
                        return Ok(None);
                    };
                    if matches!(byte, 0x0d | 0x0a | 0x8d | 0x8a | 0x11 | 0x13 | 0x91 | 0x93) {
                        continue;
                    }
                    self.subpacket_state = SubpacketPhase::Reading;
                    return self.receive_subpacket_plain_byte(port, byte);
                }
            }
            SubpacketPhase::Reading => self.receive_subpacket_data_byte(port),
            SubpacketPhase::Crc(packet) => {
                match self.crc.process(port, self.data_encoding) {
                    Ok(Some(())) => {}
                    Ok(None) => return Ok(None),
                    // A corrupt DATA subpacket is recoverable: ZMODEM's
                    // whole reason for existing over X/YMODEM is that the
                    // receiver asks the sender to retransmit from the last
                    // good offset instead of aborting. Metadata (ZFILE)
                    // and ZSINIT CRC failures stay fatal: there is no
                    // meaningful offset to rewind a header to.
                    Err(e @ (Error::UnexpectedCrc16 | Error::UnexpectedCrc32))
                        if self.state == ReceiverPhase::FileReadingSubpacket =>
                    {
                        return self.recover_corrupt_subpacket(e);
                    }
                    Err(e) => return Err(e),
                }

                if self.state == ReceiverPhase::FileReadingMetadata {
                    self.parse_zfile_buf()?;
                    self.buf.clear();
                    self.buf_write_offset = 0;
                    self.crc.reset();
                    self.subpacket_escape_pending = false;

                    if self.manual_accept {
                        // Hold the ZRPOS until the caller decides via
                        // accept_file_at() / skip_file().
                        self.state = ReceiverPhase::FileAcceptPending;
                    } else {
                        self.queue_zrpos(0)?;
                        self.state = ReceiverPhase::FileBegin;
                        // Same as accept_file_at: an empty file makes the
                        // sender answer ZRPOS(0) with an immediate ZEOF(0)
                        // and no data frame, which must complete from
                        // FileBegin.
                        self.file_active = true;
                    }
                    self.subpacket_state = SubpacketPhase::Idle;
                    self.push_event(ReceiverEvent::FileStart)?;
                } else if self.state == ReceiverPhase::SinitReadingData {
                    // ZSINIT's payload (the attn string) carries nothing
                    // we act on, but the sender blocks until the frame
                    // is acknowledged.
                    self.buf.clear();
                    self.buf_write_offset = 0;
                    self.crc.reset();
                    self.subpacket_escape_pending = false;

                    self.queue_header(ZACK_HEADER)?;

                    self.state = ReceiverPhase::SessionBegin;
                    self.subpacket_state = SubpacketPhase::Idle;
                } else {
                    self.subpacket_state = SubpacketPhase::Writing(packet);
                    self.buf_write_offset = 0;
                    if self.buf.is_empty() {
                        self.finish_subpacket(packet)?;
                    }
                }
                Ok(Some(()))
            }
            SubpacketPhase::Writing(_) => Ok(Some(())),
            SubpacketPhase::Idle => Err(Error::InvalidState),
        }
    }

    fn finish_subpacket(&mut self, packet: SubpacketType) -> Result<(), Error> {
        let len = u32::try_from(self.buf.len()).map_err(|_| Error::OutOfMemory)?;
        // The running offset is a u32 (ZMODEM positions are 32-bit). A
        // sender that keeps streaming past 4 GiB, whether buggy or
        // hostile, must not be able to wrap it back to a low offset and
        // desynchronise the transfer: refuse instead.
        self.count = self.count.checked_add(len).ok_or(Error::OutOfMemory)?;
        self.buf.clear();
        self.buf_write_offset = 0;
        self.crc.reset();
        // A clean subpacket landed: the offset advanced, so the corrupt
        // streak (if any) is broken and the retry budget is replenished.
        self.zrpos_retries = 0;

        match packet {
            SubpacketType::ZCRCW => {
                self.queue_zack()?;
                self.state = ReceiverPhase::FileWaitingSubpacket;
                self.subpacket_state = SubpacketPhase::Idle;
                self.subpacket_escape_pending = false;
            }
            SubpacketType::ZCRCQ => {
                self.queue_zack()?;
                self.subpacket_state = SubpacketPhase::Reading;
                self.subpacket_escape_pending = false;
            }
            SubpacketType::ZCRCG => {
                self.subpacket_state = SubpacketPhase::Reading;
                self.subpacket_escape_pending = false;
            }
            SubpacketType::ZCRCE => {
                self.state = ReceiverPhase::FileWaitingSubpacket;
                self.subpacket_state = SubpacketPhase::Idle;
                self.subpacket_escape_pending = false;
            }
        }
        Ok(())
    }

    /// Recovers from a corrupt data subpacket by asking the sender to
    /// rewind. The buffered (bad) bytes are dropped and `count` is left
    /// at the last acknowledged offset, so nothing corrupt is persisted
    /// and no data is skipped; a ZRPOS(count) tells the sender to
    /// retransmit from there. The header reader is put into resync mode
    /// because a streaming sender keeps emitting the tail of the aborted
    /// window before it honours the ZRPOS, and that tail must be skipped
    /// rather than mistaken for framing.
    ///
    /// `err` is returned unchanged once the retry budget is spent, so a
    /// hopelessly noisy link fails with the same CRC error it would have
    /// before, just after trying to recover.
    fn recover_corrupt_subpacket(&mut self, err: Error) -> Result<Option<()>, Error> {
        self.zrpos_retries = self.zrpos_retries.saturating_add(1);
        if self.zrpos_retries > MAX_ZRPOS_RETRIES {
            return Err(err);
        }
        self.buf.clear();
        self.buf_write_offset = 0;
        self.crc.reset();
        self.subpacket_escape_pending = false;
        self.queue_zrpos(self.count)?;
        self.state = ReceiverPhase::FileWaitingSubpacket;
        self.subpacket_state = SubpacketPhase::Idle;
        self.header_reader.enter_resync();
        Ok(Some(()))
    }
}
