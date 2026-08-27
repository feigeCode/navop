# modem-transfer

`modem-transfer` is Navop's built-in ZMODEM protocol engine. The caller owns all
I/O and drives the protocol state machine. The Rust library target remains named
`zmodem2` so the existing terminal integration API does not need a broad rename.

This crate is fully `no_std` compatible and *heapless*.

## Provenance

This crate is derived from `zmodem2` 0.7.2 by Jarkko Sakkinen, which is based on
prior work by Aleksei Arbuzov in the
[`zmodem`](https://github.com/lexxvir/zmodem) crate.

Upstream: <https://codeberg.org/jarkko/zmodem2>

The upstream and retained source are licensed under MIT OR Apache-2.0. See
`LICENSE-MIT` and `LICENSE-APACHE`.

Navop maintains interoperability fixes including negotiated ESCCTL escaping,
ZFILE retry behavior, and stock lrzsz integration coverage.

OxideTerm was consulted only as a high-level architectural reference for keeping
the protocol engine in a first-party workspace crate and separating it from the
terminal UI, transport, and filesystem. No OxideTerm source code was copied or
adapted. The protocol implementation in this crate remains Navop's maintained
`zmodem2`-derived engine described above.

## Usage

Create a `Sender` or `Receiver`, then loop on `poll()` and act on the returned
`Action`:

1. `Action::WriteWire(bytes)` — write the bytes to the transport, then call
   `wire_written(n)` with the number of bytes accepted.
2. `Action::ReadFile { offset, max_len }` (sender) — read the requested file
   bytes and provide them with `submit_file(data)`.
3. `Action::WriteFile(bytes)` (receiver) — persist the bytes, then call
   `file_written(n)`.
4. `Action::Event(event)` — handle a protocol `Event` (file/session lifecycle).
5. `Action::Idle` — feed incoming transport bytes with `submit_wire(bytes)`, or
   call `timeout()` if none arrive.

The sender offers files with `start_file(FileInfo)` and ends the session with
`finish()`. Either side can `abort()`.
