use super::asciicast::{
    RecordingFileError, RecordingFileLimit, RecordingFileLimits, RecordingHeader, decode_event,
    decode_header,
};
use super::{RecordingEvent, RecordingEventKind};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingCompleteness {
    Complete,
    Partial { discarded_bytes: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedRecording {
    pub header: RecordingHeader,
    pub events: Vec<RecordingEvent>,
    pub completeness: RecordingCompleteness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingRecovery {
    pub recording: ParsedRecording,
    pub valid_bytes: u64,
    pub discarded_bytes: u64,
}

pub fn read_recording(
    path: impl AsRef<Path>,
    limits: RecordingFileLimits,
) -> Result<ParsedRecording, RecordingFileError> {
    Ok(read_recording_inner(path.as_ref(), limits, false)?.recording)
}

pub fn recover_partial_recording(
    path: impl AsRef<Path>,
    limits: RecordingFileLimits,
) -> Result<RecordingRecovery, RecordingFileError> {
    let path = path.as_ref();
    if !path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".partial"))
    {
        return Err(RecordingFileError::InvalidPartialPath(path.to_path_buf()));
    }
    let recovery = read_recording_inner(path, limits, true)?;
    let current_bytes = fs::metadata(path)
        .map_err(|error| RecordingFileError::io("stat partial recording", error))?
        .len();
    let expected_bytes = recovery
        .valid_bytes
        .checked_add(recovery.discarded_bytes)
        .ok_or(RecordingFileError::FileChangedDuringRecovery)?;
    if current_bytes != expected_bytes {
        return Err(RecordingFileError::FileChangedDuringRecovery);
    }
    if recovery.discarded_bytes > 0 {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| RecordingFileError::io("open partial for recovery", error))?;
        file.set_len(recovery.valid_bytes)
            .map_err(|error| RecordingFileError::io("truncate partial recording", error))?;
        file.sync_all()
            .map_err(|error| RecordingFileError::io("sync recovered recording", error))?;
    }
    Ok(recovery)
}

fn read_recording_inner(
    path: &Path,
    limits: RecordingFileLimits,
    allow_partial_tail: bool,
) -> Result<RecordingRecovery, RecordingFileError> {
    let file_bytes = fs::metadata(path)
        .map_err(|error| RecordingFileError::io("stat recording", error))?
        .len();
    if file_bytes > limits.max_file_bytes {
        return Err(RecordingFileError::LimitReached(
            RecordingFileLimit::FileBytes,
        ));
    }
    let file =
        fs::File::open(path).map_err(|error| RecordingFileError::io("open recording", error))?;
    let mut reader = BufReader::new(file);

    let Some(header_line) = read_bounded_line(
        &mut reader,
        limits.max_header_bytes,
        RecordingFileLimit::HeaderBytes,
    )?
    else {
        return Err(RecordingFileError::InvalidHeader(
            "recording is empty".to_string(),
        ));
    };
    if !header_line.terminated {
        return Err(RecordingFileError::InvalidHeader(
            "header is truncated".to_string(),
        ));
    }
    let header = decode_header(strip_newline(&header_line.bytes))?;
    let mut valid_bytes = header_line.consumed;
    let mut discarded_bytes = 0;
    let mut events = Vec::new();
    let mut event_count = 0_u64;
    let mut decoded_payload_bytes = 0_u64;
    let mut last_elapsed = None;
    let mut line_number = 2_u64;

    while let Some(line) = read_bounded_line(
        &mut reader,
        limits.max_serialized_event_bytes,
        RecordingFileLimit::EventBytes,
    )? {
        if !line.terminated {
            if allow_partial_tail {
                discarded_bytes = file_bytes.saturating_sub(valid_bytes);
                break;
            }
            return Err(RecordingFileError::invalid_event(
                line_number,
                "event line is truncated",
            ));
        }
        if event_count >= limits.max_events {
            return Err(RecordingFileError::LimitReached(
                RecordingFileLimit::EventCount,
            ));
        }
        let event = decode_event(strip_newline(&line.bytes), line_number)?;
        if last_elapsed.is_some_and(|elapsed| event.elapsed < elapsed) {
            return Err(RecordingFileError::invalid_event(
                line_number,
                "timestamp moved backwards",
            ));
        }
        if matches!(event.kind, RecordingEventKind::Input(_)) && !header.navop.capture_input {
            return Err(RecordingFileError::invalid_event(
                line_number,
                "input event is present while capture_input is disabled",
            ));
        }
        let payload_bytes = u64::try_from(event.kind.payload_len()).map_err(|_| {
            RecordingFileError::LimitReached(RecordingFileLimit::DecodedPayloadBytes)
        })?;
        let next_decoded_payload_bytes = decoded_payload_bytes.checked_add(payload_bytes).ok_or(
            RecordingFileError::LimitReached(RecordingFileLimit::DecodedPayloadBytes),
        )?;
        if next_decoded_payload_bytes > limits.max_decoded_payload_bytes {
            return Err(RecordingFileError::LimitReached(
                RecordingFileLimit::DecodedPayloadBytes,
            ));
        }

        valid_bytes =
            valid_bytes
                .checked_add(line.consumed)
                .ok_or(RecordingFileError::LimitReached(
                    RecordingFileLimit::FileBytes,
                ))?;
        if valid_bytes > limits.max_file_bytes {
            return Err(RecordingFileError::LimitReached(
                RecordingFileLimit::FileBytes,
            ));
        }
        last_elapsed = Some(event.elapsed);
        decoded_payload_bytes = next_decoded_payload_bytes;
        event_count += 1;
        events.push(event);
        line_number += 1;
    }

    let completeness = if allow_partial_tail {
        RecordingCompleteness::Partial { discarded_bytes }
    } else {
        RecordingCompleteness::Complete
    };
    Ok(RecordingRecovery {
        recording: ParsedRecording {
            header,
            events,
            completeness,
        },
        valid_bytes,
        discarded_bytes,
    })
}

struct BoundedLine {
    bytes: Vec<u8>,
    consumed: u64,
    terminated: bool,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
    limit: RecordingFileLimit,
) -> Result<Option<BoundedLine>, RecordingFileError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| RecordingFileError::io("read recording", error))?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            let consumed =
                u64::try_from(bytes.len()).map_err(|_| RecordingFileError::LimitReached(limit))?;
            return Ok(Some(BoundedLine {
                bytes,
                consumed,
                terminated: false,
            }));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |newline| newline + 1);
        let next_len = bytes
            .len()
            .checked_add(take)
            .ok_or(RecordingFileError::LimitReached(limit))?;
        if next_len > max_bytes {
            return Err(RecordingFileError::LimitReached(limit));
        }
        let terminated = available.get(take.saturating_sub(1)) == Some(&b'\n');
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if terminated {
            let consumed =
                u64::try_from(bytes.len()).map_err(|_| RecordingFileError::LimitReached(limit))?;
            return Ok(Some(BoundedLine {
                bytes,
                consumed,
                terminated: true,
            }));
        }
    }
}

fn strip_newline(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}
