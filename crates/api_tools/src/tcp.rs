//! Raw TCP request preparation, payload decoding, and transport API.

use anyhow::{Context as _, Result, bail};
use url::Url;

use crate::http::{PreparedRequest, normalize_url_with_default};

#[path = "tcp_transport.rs"]
mod transport;

pub use transport::{ConnectionCommand, ConnectionEvent, ConnectionTask, start_connection};

const DEFAULT_TCP_SCHEME: &str = "tcp";

pub fn prepare_tcp_request(mut request: PreparedRequest) -> PreparedRequest {
    request.url = normalize_url_with_default(&request.url, DEFAULT_TCP_SCHEME);
    request
}

pub fn target_from_url(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("TCP address is empty");
    }
    let normalized = normalize_url_with_default(value, DEFAULT_TCP_SCHEME);
    let url = Url::parse(&normalized).context("invalid TCP address")?;
    if url.scheme() != DEFAULT_TCP_SCHEME {
        bail!("unsupported TCP scheme: {}", url.scheme());
    }
    let host = url.host_str().context("TCP host is missing")?;
    let port = url.port().context("TCP port is missing")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.contains(':') {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}

pub fn decode_payload(value: &str) -> Result<Vec<u8>> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Ok(value.as_bytes().to_vec());
    };
    let compact = hex
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if !compact.len().is_multiple_of(2) {
        bail!("hex payload must contain an even number of digits");
    }
    (0..compact.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&compact[index..index + 2], 16)
                .with_context(|| format!("invalid hex payload at byte {}", index / 2))
        })
        .collect()
}
