use std::path::Path;

use x11_forwarding::{MagicCookie, ServerEndpoint};

use crate::X11ComponentError;

const AUTH_NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";

pub fn encode_xauth(
    endpoint: &ServerEndpoint,
    cookie: &MagicCookie,
) -> Result<Vec<u8>, X11ComponentError> {
    let (family, address, display) = authority_address(endpoint)?;
    let mut output = Vec::with_capacity(64);
    push_u16(&mut output, family);
    for field in [
        address.as_slice(),
        display.as_bytes(),
        AUTH_NAME,
        cookie.bytes().as_slice(),
    ] {
        let length = u16::try_from(field.len())
            .map_err(|_| X11ComponentError::Config("Xauthority 字段过长".into()))?;
        push_u16(&mut output, length);
        output.extend_from_slice(field);
    }
    Ok(output)
}

pub fn write_xauth(
    path: &Path,
    endpoint: &ServerEndpoint,
    cookie: &MagicCookie,
) -> Result<(), X11ComponentError> {
    let bytes = encode_xauth(endpoint, cookie)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(config_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(config_error)?;
    }
    std::io::Write::write_all(&mut file, &bytes).map_err(config_error)
}

fn authority_address(
    endpoint: &ServerEndpoint,
) -> Result<(u16, Vec<u8>, String), X11ComponentError> {
    match endpoint {
        ServerEndpoint::Inet { host, port } => {
            if host != "127.0.0.1" || *port < 6000 {
                return Err(X11ComponentError::Config(
                    "Xauthority 仅接受回环 TCP".into(),
                ));
            }
            Ok((0, vec![127, 0, 0, 1], (*port - 6000).to_string()))
        }
        ServerEndpoint::Unix(path) => {
            let display = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix('X'))
                .filter(|number| number.chars().all(|ch| ch.is_ascii_digit()))
                .ok_or_else(|| X11ComponentError::Config("Unix socket 缺少显示号".into()))?;
            Ok((256, local_hostname()?, display.into()))
        }
    }
}

#[cfg(unix)]
fn local_hostname() -> Result<Vec<u8>, X11ComponentError> {
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY：buffer 提供有效可写内存，长度与实际容量一致。
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len()) };
    if result != 0 {
        return Err(config_error(std::io::Error::last_os_error()));
    }
    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    if length == 0 {
        return Err(X11ComponentError::Config("本机 hostname 为空".into()));
    }
    Ok(buffer[..length].iter().map(|byte| *byte as u8).collect())
}

#[cfg(not(unix))]
fn local_hostname() -> Result<Vec<u8>, X11ComponentError> {
    Ok(b"localhost".to_vec())
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn config_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Config(error.to_string())
}

#[cfg(test)]
mod tests;
