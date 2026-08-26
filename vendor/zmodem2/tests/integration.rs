// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2026 Jarkko Sakkinen

#![cfg(has_lrzsz)]

use nix::fcntl::{self, OFlag};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Result, Seek, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use zmodem2::{Action, Event, FileInfo, Position};

const FILE_COUNT: usize = 10;
const FILE_SIZE: usize = 50 * 1024;
const RATE_BPS: u32 = 115200;

const NAME_PREFIX: &[&str] = &[
    "Laser",
    "Neon",
    "Chrome",
    "Cosmic",
    "Turbo",
    "Starlight",
    "Future",
];
const NAME_POSTFIX: &[&str] = &[
    "Rider", "Funk", "Dream", "Grid", "System", "Dancer", "Midnight",
];
const EXTENSIONS: &[&str] = &["dat", "BIN", "log", "TMP", "txt"];

struct MockPort<R: Read, W: Write> {
    r: R,
    w: W,
    bits_per_second: u32,
    next_byte_due: Instant,
}

impl<R: Read, W: Write> MockPort<R, W> {
    pub fn new(r: R, w: W, bits_per_second: u32) -> Self {
        MockPort {
            r,
            w,
            bits_per_second,
            next_byte_due: Instant::now(),
        }
    }

    fn throttle(&mut self, bytes_transferred: usize) {
        if self.bits_per_second == 0 {
            return;
        }
        let bits_transferred = (bytes_transferred * 10) as f64;
        let duration_needed =
            Duration::from_secs_f64(bits_transferred / f64::from(self.bits_per_second));
        let now = Instant::now();
        if self.next_byte_due > now {
            sleep(self.next_byte_due - now);
        }
        self.next_byte_due = max(now, self.next_byte_due) + duration_needed;
    }
}

impl<R: Read, W: Write> Read for MockPort<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let bytes_read = self.r.read(buf)?;
        if bytes_read > 0 {
            self.throttle(bytes_read);
        }
        Ok(bytes_read)
    }
}

impl<R: Read, W: Write> Write for MockPort<R, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let bytes_written = self.w.write(buf)?;
        if bytes_written > 0 {
            self.throttle(bytes_written);
        }
        Ok(bytes_written)
    }

    fn flush(&mut self) -> Result<()> {
        self.w.flush()
    }
}

/// Creates a temporary file with a predictable, patterned content.
fn create_test_file(path: &Path, size_bytes: usize) {
    let mut file = File::create(path).unwrap();
    let mut buffer = [0u8; 1024];
    for (i, byte) in buffer.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }

    let mut bytes_written = 0;
    while bytes_written < size_bytes {
        let to_write = min(buffer.len(), size_bytes - bytes_written);
        file.write_all(&buffer[..to_write]).unwrap();
        bytes_written += to_write;
    }
}

/// Asserts that two files have the same size and content.
fn assert_files_equal(path1: &Path, path2: &Path) {
    let meta1 = path1.metadata().unwrap();
    let meta2 = path2.metadata().unwrap();
    assert_eq!(meta1.len(), meta2.len(), "File sizes do not match");

    let mut f1 = BufReader::new(File::open(path1).unwrap());
    let mut f2 = BufReader::new(File::open(path2).unwrap());

    loop {
        let buf1 = f1.fill_buf().unwrap();
        let buf2 = f2.fill_buf().unwrap();

        if buf1.is_empty() && buf2.is_empty() {
            break;
        }

        assert_eq!(buf1, buf2, "File contents do not match");

        let len1 = buf1.len();
        let len2 = buf2.len();
        f1.consume(len1);
        f2.consume(len2);
    }
}

/// Manages a set of temporary files for a test run.
struct TestFiles {
    #[allow(dead_code)]
    dir: TempDir,
    paths: Vec<PathBuf>,
}

impl TestFiles {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zmodem_test_src_")
            .tempdir()
            .unwrap();
        let mut paths = Vec::new();
        for i in 0..FILE_COUNT {
            let prefix = NAME_PREFIX[i % NAME_PREFIX.len()];
            let postfix = NAME_POSTFIX[i % NAME_POSTFIX.len()];
            let ext = EXTENSIONS[i % EXTENSIONS.len()];
            let filename = format!("{prefix}{postfix}_{i}.{ext}");
            let path = dir.path().join(filename);

            create_test_file(&path, FILE_SIZE);
            paths.push(path);
        }
        Self { dir, paths }
    }
}

/// Sets the O_NONBLOCK flag on a raw file descriptor.
fn set_nonblocking(fd: RawFd) {
    let flags = fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFL).unwrap();
    let mut nonblocking_flags = OFlag::from_bits_truncate(flags);
    nonblocking_flags.insert(OFlag::O_NONBLOCK);
    fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFL(nonblocking_flags)).unwrap();
}

/// Helper to set up a non-blocking `sz` process.
fn setup_sz(test_files: &TestFiles) -> (Child, MockPort<ChildStdout, ChildStdin>) {
    let mut sz_process = Command::new(env!("ZMODEM_SZ_BIN"))
        .args(&test_files.paths)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();

    let stdin = sz_process.stdin.take().unwrap();
    let stdout = sz_process.stdout.take().unwrap();

    set_nonblocking(stdin.as_raw_fd());
    set_nonblocking(stdout.as_raw_fd());

    let port = MockPort::new(stdout, stdin, RATE_BPS);
    (sz_process, port)
}

/// Helper to set up a non-blocking `rz` process.
fn setup_rz(dest_dir: &TempDir) -> (Child, MockPort<ChildStdout, ChildStdin>) {
    let mut rz_process: Child = Command::new(env!("ZMODEM_RZ_BIN"))
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .current_dir(dest_dir.path())
        .spawn()
        .unwrap();

    let stdin = rz_process.stdin.take().unwrap();
    let stdout = rz_process.stdout.take().unwrap();

    set_nonblocking(stdin.as_raw_fd());
    set_nonblocking(stdout.as_raw_fd());

    let port = MockPort::new(stdout, stdin, RATE_BPS);
    (rz_process, port)
}

struct MockBlockingFile {
    file: File,
    counter: usize,
}

impl Write for MockBlockingFile {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.counter += 1;
        if self.counter % 10 == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.file.flush()
    }
}

#[test]
#[cfg(has_lrzsz)]
fn test_batch_from_sz() {
    let test_files = TestFiles::new();
    let dest_dir = tempfile::Builder::new()
        .prefix("zmodem_test_dest_")
        .tempdir()
        .unwrap();

    let (mut sz_process, mut port) = setup_sz(&test_files);
    let mut receiver = zmodem2::Receiver::new().unwrap();
    let mut open_files: HashMap<Vec<u8>, MockBlockingFile> = HashMap::new();
    let mut sink = std::io::sink();
    let mut current_file_name_bytes: Vec<u8> = Vec::new();
    let mut wire_buf = [0u8; 4096];
    let mut input_buf: Vec<u8> = Vec::new();
    let mut input_offset: usize = 0;
    let mut session_done = false;

    loop {
        let mut progressed = false;
        let mut idle = false;

        match port.read(&mut wire_buf) {
            Ok(0) => {}
            Ok(n) => {
                input_buf.extend_from_slice(&wire_buf[..n]);
                progressed = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("wire read failed: {e}"),
        }

        match receiver.poll() {
            Action::WriteWire(bytes) => match port.write(bytes) {
                Ok(0) => {}
                Ok(n) => {
                    receiver.wire_written(n);
                    progressed = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("wire write failed: {e}"),
            },
            Action::WriteFile(bytes) => {
                let file_writer: &mut dyn Write = open_files
                    .get_mut(&current_file_name_bytes)
                    .map(|f| f as &mut dyn Write)
                    .unwrap_or(&mut sink);

                match file_writer.write(bytes) {
                    Ok(0) => {}
                    Ok(n) => {
                        receiver.file_written(n).unwrap();
                        progressed = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => panic!("file write failed: {e}"),
                }
            }
            Action::Event(event) => {
                progressed = true;
                match event {
                    Event::FileStarted(info) => {
                        if info.name != current_file_name_bytes.as_slice() {
                            let filename_str = std::str::from_utf8(info.name).unwrap();
                            let filename = Path::new(filename_str)
                                .file_name()
                                .unwrap()
                                .to_str()
                                .unwrap();
                            let file_path = dest_dir.path().join(filename);
                            let file = File::create(file_path).unwrap();
                            open_files
                                .insert(info.name.to_vec(), MockBlockingFile { file, counter: 0 });
                            current_file_name_bytes = info.name.to_vec();
                        }
                    }
                    Event::FileCompleted => {}
                    Event::SessionCompleted => session_done = true,
                    Event::Aborted => panic!("receiver aborted"),
                    _ => {}
                }
            }
            Action::ReadFile { .. } => unreachable!("receiver never reads files"),
            Action::Idle => {
                idle = true;
                if input_offset < input_buf.len() {
                    let consumed = receiver.submit_wire(&input_buf[input_offset..]).unwrap();
                    if consumed > 0 {
                        input_offset += consumed;
                        progressed = true;
                        if input_offset == input_buf.len() {
                            input_buf.clear();
                            input_offset = 0;
                        } else if input_offset > 4096 {
                            input_buf.drain(..input_offset);
                            input_offset = 0;
                        }
                    }
                }
            }
            _ => {}
        }

        if session_done && idle {
            break;
        }

        if !progressed {
            sleep(Duration::from_millis(5));
        }
    }

    sz_process.wait().unwrap();
    for path in &test_files.paths {
        let filename = path.file_name().unwrap();
        let received_path = dest_dir.path().join(filename);
        assert!(
            received_path.exists(),
            "File '{}' was not received",
            received_path.display()
        );
        assert_files_equal(path, &received_path);
    }
}

#[test]
#[cfg(has_lrzsz)]
fn test_batch_to_rz() {
    let test_files = TestFiles::new();
    let dest_dir = tempfile::Builder::new()
        .prefix("zmodem_test_dest_")
        .tempdir()
        .unwrap();

    let (mut rz_process, mut port) = setup_rz(&dest_dir);

    let mut open_files: HashMap<String, File> = HashMap::new();
    for path in &test_files.paths {
        let filename = path.file_name().unwrap().to_str().unwrap().to_string();
        let file = File::open(path).unwrap();
        open_files.insert(filename, file);
    }

    let mut file_iter = test_files.paths.iter();

    let first_path = file_iter.next().expect("No test files found");
    let first_filename = first_path.file_name().unwrap().to_str().unwrap();
    let first_size = first_path.metadata().unwrap().len() as u32;
    let mut sender = zmodem2::Sender::new().unwrap();
    sender
        .start_file(FileInfo::new(
            first_filename.as_bytes(),
            Some(Position::new(first_size)),
        ))
        .unwrap();
    let mut current_filename = first_filename.to_string();
    let mut wire_buf = [0u8; 4096];
    let mut input_buf: Vec<u8> = Vec::new();
    let mut input_offset: usize = 0;
    let mut file_buf = [0u8; 1024];
    let mut session_done = false;

    loop {
        let mut progressed = false;
        let mut idle = false;

        match port.read(&mut wire_buf) {
            Ok(0) => {}
            Ok(n) => {
                input_buf.extend_from_slice(&wire_buf[..n]);
                progressed = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("wire read failed: {e}"),
        }

        match sender.poll() {
            Action::WriteWire(bytes) => match port.write(bytes) {
                Ok(0) => {}
                Ok(n) => {
                    sender.wire_written(n);
                    progressed = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("wire write failed: {e}"),
            },
            Action::ReadFile { offset, max_len } => {
                let file = open_files
                    .get_mut(&current_filename)
                    .expect("File not found in map");
                file.seek(std::io::SeekFrom::Start(u64::from(offset.get())))
                    .unwrap();
                let n = file.read(&mut file_buf[..max_len]).unwrap();
                sender.submit_file(&file_buf[..n]).unwrap();
                progressed = true;
            }
            Action::Event(event) => {
                progressed = true;
                match event {
                    Event::FileCompleted => {
                        if let Some(next_path) = file_iter.next() {
                            let next_filename = next_path.file_name().unwrap().to_str().unwrap();
                            let next_size = next_path.metadata().unwrap().len() as u32;
                            sender
                                .start_file(FileInfo::new(
                                    next_filename.as_bytes(),
                                    Some(Position::new(next_size)),
                                ))
                                .unwrap();
                            current_filename = next_filename.to_string();
                        } else {
                            sender.finish().unwrap();
                        }
                    }
                    Event::SessionCompleted => session_done = true,
                    Event::FileStarted(_) => {}
                    Event::Aborted => panic!("sender aborted"),
                    _ => {}
                }
            }
            Action::WriteFile(_) => unreachable!("sender never writes files"),
            Action::Idle => {
                idle = true;
                if input_offset < input_buf.len() {
                    let consumed = sender.submit_wire(&input_buf[input_offset..]).unwrap();
                    if consumed > 0 {
                        input_offset += consumed;
                        progressed = true;
                        if input_offset == input_buf.len() {
                            input_buf.clear();
                            input_offset = 0;
                        } else if input_offset > 4096 {
                            input_buf.drain(..input_offset);
                            input_offset = 0;
                        }
                    }
                }
            }
            _ => {}
        }

        if session_done && idle {
            break;
        }

        if !progressed {
            sleep(Duration::from_millis(5));
        }
    }

    rz_process.wait().unwrap();
    for path in &test_files.paths {
        let filename = path.file_name().unwrap();
        let received_path = dest_dir.path().join(filename);
        assert!(
            received_path.exists(),
            "File '{}' was not sent",
            received_path.display()
        );
        assert_files_equal(path, &received_path);
    }
}
