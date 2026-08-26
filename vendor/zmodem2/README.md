# zmodem2

`zmodem2` provides a ZMODEM file transmission protocol implementation driven by
the caller, who owns all I/O. It is based on prior work of Aleksei Arbuzov in
the [`zmodem`](https://github.com/lexxvir/zmodem) crate.

This crate is fully `no_std` compatible and *heapless*.

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

## Development

* Git: https://codeberg.org/jarkko/zmodem2.git
* Commits follow the
  [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
  specification.
* New commits must include a `Signed-off-by` trailer.
* Versioning scheme uses [Semantic Versioning](https://semver.org/).
