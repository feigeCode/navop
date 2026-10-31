//! 启动请求的线上格式：`u32 路径条数` + 每条 `u32 字节长度 + 路径字节`（小端）。
//!
//! Windows 侧路径用 UTF-16LE（能无损表达任意 `OsString`），Unix 侧用 UTF-8。
//! 两边都做条数/单条/总量上限校验，解码失败一律丢弃整条请求。

use std::io;
use std::path::{Path, PathBuf};

use super::{MAX_PATH_BYTES, MAX_PATH_COUNT, MAX_PAYLOAD_BYTES, StartupRequest};

pub(super) fn encode_request(request: &StartupRequest) -> io::Result<Vec<u8>> {
    if request.paths().len() > MAX_PATH_COUNT {
        return Err(invalid_data("too many startup paths"));
    }

    let mut payload = Vec::new();
    append_u32(&mut payload, request.paths().len())?;
    for path in request.paths() {
        let bytes = encode_path(path);
        if bytes.len() > MAX_PATH_BYTES {
            return Err(invalid_data("startup path is too long"));
        }
        append_u32(&mut payload, bytes.len())?;
        payload.extend_from_slice(&bytes);
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(invalid_data("startup request is too large"));
        }
    }
    Ok(payload)
}

pub(super) fn decode_request(payload: &[u8]) -> io::Result<StartupRequest> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid_data("startup request is too large"));
    }

    let mut cursor = 0;
    let path_count = read_u32(payload, &mut cursor)?;
    if path_count > MAX_PATH_COUNT {
        return Err(invalid_data("too many startup paths"));
    }

    let mut paths = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let path_length = read_u32(payload, &mut cursor)?;
        if path_length > MAX_PATH_BYTES {
            return Err(invalid_data("startup path is too long"));
        }
        let path_end = cursor
            .checked_add(path_length)
            .ok_or_else(|| invalid_data("startup path length overflow"))?;
        let path_bytes = payload
            .get(cursor..path_end)
            .ok_or_else(|| invalid_data("truncated startup path"))?;
        paths.push(decode_path(path_bytes)?);
        cursor = path_end;
    }
    if cursor != payload.len() {
        return Err(invalid_data("startup request contains trailing data"));
    }

    Ok(StartupRequest::new(paths))
}

#[cfg(target_os = "windows")]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn encode_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(target_os = "windows")]
fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    if !bytes.len().is_multiple_of(size_of::<u16>()) {
        return Err(invalid_data("startup path has truncated UTF-16 data"));
    }
    let wide = bytes
        .chunks_exact(size_of::<u16>())
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(not(target_os = "windows"))]
fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    let path =
        std::str::from_utf8(bytes).map_err(|_| invalid_data("startup path is not valid UTF-8"))?;
    Ok(PathBuf::from(path))
}

fn append_u32(buffer: &mut Vec<u8>, value: usize) -> io::Result<()> {
    let value = u32::try_from(value).map_err(|_| invalid_data("length exceeds wire format"))?;
    buffer.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_u32(payload: &[u8], cursor: &mut usize) -> io::Result<usize> {
    let end = cursor
        .checked_add(size_of::<u32>())
        .ok_or_else(|| invalid_data("startup request length overflow"))?;
    let bytes: [u8; 4] = payload
        .get(*cursor..end)
        .ok_or_else(|| invalid_data("truncated startup request"))?
        .try_into()
        .expect("u32 payload slice must have four bytes");
    *cursor = end;
    Ok(u32::from_le_bytes(bytes) as usize)
}

pub(super) fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
