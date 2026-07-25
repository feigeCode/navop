use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MAX_PATH_COUNT: usize = 256;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;
#[cfg(target_os = "windows")]
const CONNECTION_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(target_os = "windows")]
const CONNECT_RETRY_COUNT: usize = 20;
#[cfg(target_os = "windows")]
const CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupRequest {
    paths: Vec<PathBuf>,
}

impl StartupRequest {
    pub(crate) fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }
}

#[cfg(target_os = "windows")]
pub(crate) enum SingleInstanceOutcome {
    Primary,
    Forwarded,
}

fn instance_name_for_config_dir(config_dir: &Path) -> String {
    let path = config_dir.to_string_lossy();
    #[cfg(target_os = "windows")]
    let path = path.to_lowercase();

    let digest = Sha256::digest(path.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("navop-single-instance-{suffix}")
}

fn encode_request(request: &StartupRequest) -> io::Result<Vec<u8>> {
    if request.paths.len() > MAX_PATH_COUNT {
        return Err(invalid_data("too many startup paths"));
    }

    let mut payload = Vec::new();
    append_u32(&mut payload, request.paths.len())?;
    for path in &request.paths {
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

fn decode_request(payload: &[u8]) -> io::Result<StartupRequest> {
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

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(target_os = "windows")]
pub(crate) fn claim_or_forward(
    config_dir: &Path,
    request: StartupRequest,
    on_request: impl Fn(StartupRequest) + Send + 'static,
) -> io::Result<SingleInstanceOutcome> {
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};

    let instance_name = instance_name_for_config_dir(config_dir);
    let listener_name = instance_name.clone().to_ns_name::<GenericNamespaced>()?;
    match ListenerOptions::new().name(listener_name).create_sync() {
        Ok(listener) => {
            std::thread::Builder::new()
                .name("navop-single-instance".to_string())
                .spawn(move || {
                    for connection in listener.incoming() {
                        match connection {
                            Ok(mut connection) => {
                                if let Err(error) = receive_request(&mut connection, &on_request) {
                                    tracing::warn!(
                                        %error,
                                        "failed to receive Windows single-instance request"
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    %error,
                                    "failed to accept Windows single-instance connection"
                                );
                            }
                        }
                    }
                })?;
            Ok(SingleInstanceOutcome::Primary)
        }
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            forward_request::<Stream>(&instance_name, &request)?;
            Ok(SingleInstanceOutcome::Forwarded)
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "windows")]
fn receive_request(
    connection: &mut interprocess::local_socket::Stream,
    on_request: &impl Fn(StartupRequest),
) -> io::Result<()> {
    use interprocess::local_socket::prelude::*;
    use std::io::{Read as _, Write as _};

    connection.set_recv_timeout(Some(CONNECTION_IO_TIMEOUT))?;
    connection.set_send_timeout(Some(CONNECTION_IO_TIMEOUT))?;
    let mut payload_length = [0; size_of::<u32>()];
    connection.read_exact(&mut payload_length)?;
    let payload_length = u32::from_le_bytes(payload_length) as usize;
    if payload_length > MAX_PAYLOAD_BYTES {
        return Err(invalid_data("startup request is too large"));
    }

    let mut payload = vec![0; payload_length];
    connection.read_exact(&mut payload)?;
    on_request(decode_request(&payload)?);
    connection.write_all(&[1])?;
    connection.flush()
}

#[cfg(target_os = "windows")]
fn forward_request<S>(instance_name: &str, request: &StartupRequest) -> io::Result<()>
where
    S: interprocess::local_socket::traits::Stream,
{
    use interprocess::local_socket::{GenericNamespaced, prelude::*};

    let payload = encode_request(request)?;
    let payload_length =
        u32::try_from(payload.len()).map_err(|_| invalid_data("startup request is too large"))?;
    let mut last_error = None;

    for _ in 0..CONNECT_RETRY_COUNT {
        let name = instance_name.to_ns_name::<GenericNamespaced>()?;
        match S::connect(name) {
            Ok(mut connection) => {
                connection.set_recv_timeout(Some(CONNECTION_IO_TIMEOUT))?;
                connection.set_send_timeout(Some(CONNECTION_IO_TIMEOUT))?;
                connection.write_all(&payload_length.to_le_bytes())?;
                connection.write_all(&payload)?;
                connection.flush()?;

                let mut acknowledgement = [0];
                connection.read_exact(&mut acknowledgement)?;
                return if acknowledgement == [1] {
                    Ok(())
                } else {
                    Err(invalid_data(
                        "primary instance rejected the startup request",
                    ))
                };
            }
            Err(error) => {
                last_error = Some(error);
                std::thread::sleep(CONNECT_RETRY_DELAY);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "primary instance did not accept the startup request",
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_is_stable_and_scoped_to_config_directory() {
        let first = instance_name_for_config_dir(Path::new("C:/Users/alice/AppData/Navop"));
        let repeated = instance_name_for_config_dir(Path::new("C:/Users/alice/AppData/Navop"));
        let portable = instance_name_for_config_dir(Path::new("D:/Portable/Navop/config"));

        assert_eq!(first, repeated);
        assert_ne!(first, portable);
        assert!(first.starts_with("navop-single-instance-"));
        assert!(!first.contains("alice"));
    }

    #[test]
    fn startup_request_round_trips_empty_and_unicode_paths() {
        for paths in [
            Vec::new(),
            vec![
                PathBuf::from(r"C:\工作区\连接.navop"),
                PathBuf::from(r"D:\projects\demo.onetcli"),
            ],
        ] {
            let request = StartupRequest::new(paths);
            let encoded = encode_request(&request).expect("request should encode");
            let decoded = decode_request(&encoded).expect("request should decode");

            assert_eq!(request, decoded);
        }
    }

    #[test]
    fn decoder_rejects_truncated_and_oversized_payloads() {
        assert!(decode_request(&[]).is_err());
        assert!(decode_request(&[1, 0, 0]).is_err());

        let mut oversized_count = Vec::new();
        oversized_count.extend_from_slice(&((MAX_PATH_COUNT as u32) + 1).to_le_bytes());
        assert!(decode_request(&oversized_count).is_err());

        let oversized_payload = vec![0; MAX_PAYLOAD_BYTES + 1];
        assert!(decode_request(&oversized_payload).is_err());
    }
}
