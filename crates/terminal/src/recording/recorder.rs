use super::asciicast::{
    RecordingFileError, RecordingFileLimit, RecordingFileLimits, RecordingHeader,
    RecordingMetadata, encode_event, encode_header,
};
use super::{RecordingEvent, RecordingEventKind};
use crate::TerminalSize;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_FLUSH_EVERY_EVENTS: u64 = 32;
const WRITE_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct RecordingFileConfig {
    pub limits: RecordingFileLimits,
    /// Flushes and synchronizes the partial file after this many events.
    pub flush_every_events: u64,
}

impl Default for RecordingFileConfig {
    fn default() -> Self {
        Self {
            limits: RecordingFileLimits::default(),
            flush_every_events: DEFAULT_FLUSH_EVERY_EVENTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingFileState {
    Open,
    Published,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingFileTransition {
    Changed,
    Unchanged,
}

pub struct RecordingFileWriter {
    config: RecordingFileConfig,
    metadata: RecordingMetadata,
    final_path: PathBuf,
    partial_path: PathBuf,
    writer: Option<BufWriter<File>>,
    state: RecordingFileState,
    bytes_written: u64,
    event_count: u64,
    decoded_payload_bytes: u64,
    events_since_flush: u64,
    last_elapsed: Option<Duration>,
}

impl RecordingFileWriter {
    pub fn create(
        final_path: impl AsRef<Path>,
        metadata: RecordingMetadata,
        initial_size: TerminalSize,
        config: RecordingFileConfig,
    ) -> Result<Self, RecordingFileError> {
        validate_config(&config)?;
        let final_path = final_path.as_ref().to_path_buf();
        let partial_path = partial_recording_path(&final_path)?;
        if final_path.exists() {
            return Err(RecordingFileError::FinalPathExists(final_path));
        }
        if let Some(parent) = final_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| RecordingFileError::io("create parent directory", error))?;
        }

        let header = RecordingHeader::new(metadata.clone(), initial_size);
        let mut header_line = encode_header(&header)?;
        header_line.push(b'\n');
        if header_line.len() > config.limits.max_header_bytes {
            return Err(RecordingFileError::LimitReached(
                RecordingFileLimit::HeaderBytes,
            ));
        }
        let header_bytes = u64::try_from(header_line.len())
            .map_err(|_| RecordingFileError::LimitReached(RecordingFileLimit::HeaderBytes))?;
        if header_bytes > config.limits.max_file_bytes {
            return Err(RecordingFileError::LimitReached(
                RecordingFileLimit::FileBytes,
            ));
        }

        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial_path)
            .map_err(|error| RecordingFileError::io("create partial file", error))?;
        let mut writer = BufWriter::with_capacity(WRITE_BUFFER_BYTES, file);
        writer
            .write_all(&header_line)
            .map_err(|error| RecordingFileError::io("write header", error))?;
        writer
            .flush()
            .map_err(|error| RecordingFileError::io("flush header", error))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| RecordingFileError::io("sync header", error))?;

        Ok(Self {
            config,
            metadata,
            final_path,
            partial_path,
            writer: Some(writer),
            state: RecordingFileState::Open,
            bytes_written: header_bytes,
            event_count: 0,
            decoded_payload_bytes: 0,
            events_since_flush: 0,
            last_elapsed: None,
        })
    }

    pub fn state(&self) -> &RecordingFileState {
        &self.state
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub fn append(&mut self, event: &RecordingEvent) -> Result<(), RecordingFileError> {
        self.ensure_open()?;
        if matches!(event.kind, RecordingEventKind::Input(_)) && !self.metadata.capture_input {
            return self.fail(RecordingFileError::InputCaptureDisabled);
        }
        if self
            .last_elapsed
            .is_some_and(|last_elapsed| event.elapsed < last_elapsed)
        {
            return self.fail(RecordingFileError::invalid_event(
                self.event_count.saturating_add(2),
                "timestamp moved backwards",
            ));
        }
        if self.event_count >= self.config.limits.max_events {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::EventCount,
            ));
        }
        let payload_bytes = match u64::try_from(event.kind.payload_len()) {
            Ok(payload_bytes) => payload_bytes,
            Err(_) => {
                return self.fail(RecordingFileError::LimitReached(
                    RecordingFileLimit::DecodedPayloadBytes,
                ));
            }
        };
        let Some(next_decoded_payload_bytes) =
            self.decoded_payload_bytes.checked_add(payload_bytes)
        else {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::DecodedPayloadBytes,
            ));
        };
        if next_decoded_payload_bytes > self.config.limits.max_decoded_payload_bytes {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::DecodedPayloadBytes,
            ));
        }

        let mut line = match encode_event(event, self.metadata.capture_input) {
            Ok(line) => line,
            Err(error) => return self.fail(error),
        };
        line.push(b'\n');
        if line.len() > self.config.limits.max_serialized_event_bytes {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::EventBytes,
            ));
        }
        let line_bytes = match u64::try_from(line.len()) {
            Ok(line_bytes) => line_bytes,
            Err(_) => {
                return self.fail(RecordingFileError::LimitReached(
                    RecordingFileLimit::EventBytes,
                ));
            }
        };
        let Some(next_file_bytes) = self.bytes_written.checked_add(line_bytes) else {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::FileBytes,
            ));
        };
        if next_file_bytes > self.config.limits.max_file_bytes {
            return self.fail(RecordingFileError::LimitReached(
                RecordingFileLimit::FileBytes,
            ));
        }

        if let Err(error) = self
            .writer
            .as_mut()
            .expect("open recording has a writer")
            .write_all(&line)
        {
            return self.fail(RecordingFileError::io("write event", error));
        }
        self.bytes_written = next_file_bytes;
        self.decoded_payload_bytes = next_decoded_payload_bytes;
        self.event_count += 1;
        self.events_since_flush += 1;
        self.last_elapsed = Some(event.elapsed);
        if self.events_since_flush >= self.config.flush_every_events {
            self.flush()?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), RecordingFileError> {
        self.ensure_open()?;
        let result = self.flush_open_file();
        if let Err(error) = result {
            return self.fail(error);
        }
        self.events_since_flush = 0;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<RecordingFileTransition, RecordingFileError> {
        match self.state {
            RecordingFileState::Published => return Ok(RecordingFileTransition::Unchanged),
            RecordingFileState::Failed => return Err(RecordingFileError::NotOpen),
            RecordingFileState::Open => {}
        }
        if let Err(error) = self.flush_open_file() {
            return self.fail(error);
        }
        let writer = self.writer.take().expect("open recording has a writer");
        let file = match writer.into_inner() {
            Ok(file) => file,
            Err(error) => {
                return self.fail(RecordingFileError::io(
                    "finish buffered recording",
                    error.into_error(),
                ));
            }
        };
        if let Err(error) = file.sync_all() {
            return self.fail(RecordingFileError::io("sync recording", error));
        }
        drop(file);

        if self.final_path.exists() {
            return self.fail(RecordingFileError::FinalPathExists(self.final_path.clone()));
        }
        if let Err(error) = fs::rename(&self.partial_path, &self.final_path) {
            return self.fail(RecordingFileError::io("publish recording", error));
        }
        self.state = RecordingFileState::Published;
        Ok(RecordingFileTransition::Changed)
    }

    fn ensure_open(&self) -> Result<(), RecordingFileError> {
        if self.state == RecordingFileState::Open && self.writer.is_some() {
            Ok(())
        } else {
            Err(RecordingFileError::NotOpen)
        }
    }

    fn flush_open_file(&mut self) -> Result<(), RecordingFileError> {
        let writer = self.writer.as_mut().ok_or(RecordingFileError::NotOpen)?;
        writer
            .flush()
            .map_err(|error| RecordingFileError::io("flush recording", error))?;
        writer
            .get_ref()
            .sync_data()
            .map_err(|error| RecordingFileError::io("sync recording data", error))
    }

    fn fail<T>(&mut self, error: RecordingFileError) -> Result<T, RecordingFileError> {
        self.state = RecordingFileState::Failed;
        Err(error)
    }
}

pub fn partial_recording_path(final_path: &Path) -> Result<PathBuf, RecordingFileError> {
    let Some(file_name) = final_path.file_name() else {
        return Err(RecordingFileError::InvalidFinalPath(
            final_path.to_path_buf(),
        ));
    };
    let mut partial_name = OsString::from(file_name);
    partial_name.push(".partial");
    Ok(final_path.with_file_name(partial_name))
}

fn validate_config(config: &RecordingFileConfig) -> Result<(), RecordingFileError> {
    if config.flush_every_events == 0 {
        return Err(RecordingFileError::InvalidConfig(
            "flush_every_events must be greater than zero".to_string(),
        ));
    }
    if config.limits.max_header_bytes == 0
        || config.limits.max_serialized_event_bytes == 0
        || config.limits.max_file_bytes == 0
        || config.limits.max_events == 0
        || config.limits.max_decoded_payload_bytes == 0
    {
        return Err(RecordingFileError::InvalidConfig(
            "all recording file limits must be greater than zero".to_string(),
        ));
    }
    Ok(())
}
