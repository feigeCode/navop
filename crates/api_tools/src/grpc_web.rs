use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, Builder, HttpClient, HttpRequestExt, Method, RedirectPolicy};

use crate::http::{HttpResponse, KeyValue, PreparedRequest, RequestMethod};

const DATA_FRAME: u8 = 0x00;
const COMPRESSED_FLAG: u8 = 0x01;
const TRAILER_FRAME: u8 = 0x80;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DecodedGrpcWebResponse {
    pub payload: Vec<u8>,
    pub grpc_status: Option<u32>,
    pub grpc_message: Option<String>,
    pub trailers: Vec<(String, String)>,
}

pub(crate) fn prepare_grpc_web_request(
    mut request: PreparedRequest,
    timeout_secs: u64,
) -> Result<PreparedRequest> {
    request.method = RequestMethod::Post;
    request.url = normalize_grpc_web_url(&request.url)?;
    set_grpc_content_type(&mut request.headers);
    push_header_if_missing(&mut request.headers, "Accept", "application/grpc-web+json");
    push_header_if_missing(&mut request.headers, "X-Grpc-Web", "1");
    push_header_if_missing(&mut request.headers, "X-User-Agent", "navop-grpc-web/0.1");
    push_header_if_missing(
        &mut request.headers,
        "Grpc-Timeout",
        &format!("{}S", timeout_secs.max(1)),
    );
    request.body = frame_request(&request.body);
    Ok(request)
}

pub(crate) async fn execute(
    client: &dyn HttpClient,
    request: PreparedRequest,
    timeout_secs: u64,
) -> HttpResponse {
    let started = Instant::now();
    let mut builder = Builder::new()
        .uri(&request.url)
        .method(Method::POST)
        .follow_redirects(RedirectPolicy::FollowAll);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    let request = match builder.body(AsyncBody::from(request.body)) {
        Ok(request) => request,
        Err(error) => {
            return transport_error(
                format!("build gRPC-Web request: {error}"),
                started.elapsed().as_millis() as u64,
            );
        }
    };

    let timeout = async {
        smol::Timer::after(Duration::from_secs(timeout_secs.max(1))).await;
        Err(anyhow!("request timed out after {timeout_secs}s"))
    };
    let exchange = async {
        let response = client.send(request).await?;
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| KeyValue::new(name.as_str(), value.to_str().unwrap_or("<binary>")))
            .collect::<Vec<_>>();
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes)
            .await
            .context("read gRPC-Web response body")?;
        Ok::<_, anyhow::Error>((status, headers, bytes))
    };

    let (status, headers, bytes) = match smol::future::or(timeout, exchange).await {
        Ok(result) => result,
        Err(error) => {
            return transport_error(error.to_string(), started.elapsed().as_millis() as u64);
        }
    };
    let time_ms = started.elapsed().as_millis() as u64;
    let size = bytes.len() as u64;
    let (header_grpc_status, header_grpc_message) = match grpc_headers(&headers) {
        Ok(headers) => headers,
        Err(error) => {
            let raw_body = String::from_utf8_lossy(&bytes).into_owned();
            return HttpResponse {
                status: status.as_u16(),
                status_text: status.canonical_reason().unwrap_or("").into(),
                time_ms,
                size,
                headers,
                raw_body: raw_body.clone(),
                body: raw_body,
                is_json: false,
                streaming: false,
                error: Some(format!("invalid gRPC-Web response headers: {error}")),
            };
        }
    };
    let content_type = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("content-type"))
        .map(|header| header.value.as_str());
    let is_grpc_web_content_type = content_type.is_none_or(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|value| value.trim().starts_with("application/grpc-web"))
    });

    if !status.is_success() || !is_grpc_web_content_type {
        let raw_body = String::from_utf8_lossy(&bytes).into_owned();
        let parsed_json = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
        let body = parsed_json
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_else(|| raw_body.clone());
        let error = grpc_error(header_grpc_status, header_grpc_message.as_deref()).or_else(|| {
            if !status.is_success() {
                Some(format!(
                    "HTTP {} {}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                ))
            } else {
                Some(format!(
                    "expected application/grpc-web response, got {}",
                    content_type.unwrap_or("<missing content-type>")
                ))
            }
        });
        return HttpResponse {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").into(),
            time_ms,
            size,
            headers,
            raw_body,
            body,
            is_json: parsed_json.is_some(),
            streaming: false,
            error,
        };
    }

    match decode_response(&bytes) {
        Ok(decoded) => {
            let raw_body = String::from_utf8_lossy(&decoded.payload).into_owned();
            let parsed_json = serde_json::from_slice::<serde_json::Value>(&decoded.payload).ok();
            let is_json = parsed_json.is_some();
            let body = parsed_json
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| raw_body.clone());
            let grpc_status = decoded.grpc_status.or(header_grpc_status);
            let grpc_message = decoded.grpc_message.or(header_grpc_message);
            let error = grpc_error(grpc_status, grpc_message.as_deref());

            HttpResponse {
                status: status.as_u16(),
                status_text: status.canonical_reason().unwrap_or("").into(),
                time_ms,
                size,
                headers,
                raw_body,
                body,
                is_json,
                streaming: false,
                error,
            }
        }
        Err(error) => {
            let raw_body = hex_body(&bytes);
            HttpResponse {
                status: status.as_u16(),
                status_text: status.canonical_reason().unwrap_or("").into(),
                time_ms,
                size,
                headers,
                raw_body: raw_body.clone(),
                body: raw_body,
                is_json: false,
                streaming: false,
                error: Some(format!("decode gRPC-Web response: {error}")),
            }
        }
    }
}

pub(crate) fn frame_request(payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).expect("gRPC-Web payload exceeds 4 GiB");
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.push(DATA_FRAME);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(payload);
    framed
}

pub(crate) fn decode_response(bytes: &[u8]) -> Result<DecodedGrpcWebResponse> {
    if bytes.is_empty() {
        return Ok(DecodedGrpcWebResponse::default());
    }
    if bytes.len() < 5 {
        return Ok(DecodedGrpcWebResponse {
            payload: bytes.to_vec(),
            ..Default::default()
        });
    }

    let mut decoded = DecodedGrpcWebResponse::default();
    let mut offset = 0;
    let mut saw_trailer = false;
    while offset < bytes.len() {
        if bytes.len() - offset < 5 {
            bail!(
                "truncated frame header at byte {offset}: {} byte(s) remain",
                bytes.len() - offset
            );
        }
        let flags = bytes[offset];
        let length = u32::from_be_bytes(
            bytes[offset + 1..offset + 5]
                .try_into()
                .expect("frame length slice"),
        ) as usize;
        offset += 5;
        if bytes.len() - offset < length {
            bail!(
                "truncated frame payload at byte {offset}: expected {length} byte(s), got {}",
                bytes.len() - offset
            );
        }
        let frame = &bytes[offset..offset + length];
        offset += length;

        if flags & TRAILER_FRAME != 0 {
            if flags != TRAILER_FRAME {
                bail!("unsupported gRPC-Web trailer flags 0x{flags:02x}");
            }
            if saw_trailer {
                bail!("gRPC-Web trailers may only appear once");
            }
            parse_trailers(frame, &mut decoded)?;
            saw_trailer = true;
            continue;
        }
        if saw_trailer {
            bail!("gRPC-Web trailers must be the final frame");
        }
        if flags & COMPRESSED_FLAG != 0 {
            bail!("compressed gRPC-Web messages are not supported");
        }
        if flags != DATA_FRAME {
            bail!("unsupported gRPC-Web data flags 0x{flags:02x}");
        }
        decoded.payload.extend_from_slice(frame);
    }
    Ok(decoded)
}

fn normalize_grpc_web_url(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("grpc://") {
        return Ok(format!("http://{rest}"));
    }
    if let Some(rest) = trimmed.strip_prefix("grpcs://") {
        return Ok(format!("https://{rest}"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    if let Some((scheme, _)) = trimmed.split_once("://") {
        bail!("unsupported gRPC-Web URL scheme: {scheme}");
    }
    Ok(crate::http::normalize_url_with_default(trimmed, "http"))
}

fn set_grpc_content_type(headers: &mut Vec<(String, String)>) {
    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        if !value
            .to_ascii_lowercase()
            .starts_with("application/grpc-web")
        {
            *value = "application/grpc-web+json".into();
        }
        return;
    }
    headers.push(("Content-Type".into(), "application/grpc-web+json".into()));
}

fn push_header_if_missing(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !headers
        .iter()
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
    {
        headers.push((name.into(), value.into()));
    }
}

fn parse_trailers(frame: &[u8], decoded: &mut DecodedGrpcWebResponse) -> Result<()> {
    let text = std::str::from_utf8(frame).context("gRPC-Web trailers are not UTF-8")?;
    for line in text.split("\r\n").filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("malformed gRPC-Web trailer: {line}"))?;
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("grpc-status") {
            decoded.grpc_status = Some(
                value
                    .parse()
                    .with_context(|| format!("invalid grpc-status trailer: {value}"))?,
            );
        } else if name.eq_ignore_ascii_case("grpc-message") {
            decoded.grpc_message = Some(percent_decode(&value)?);
        }
        decoded.trailers.push((name, value));
    }
    Ok(())
}

fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            bail!("invalid percent-encoding in grpc-message");
        }
        let high = hex_digit(bytes[index + 1])?;
        let low = hex_digit(bytes[index + 2])?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    Ok(String::from_utf8_lossy(&decoded).into_owned())
}

fn grpc_headers(headers: &[KeyValue]) -> Result<(Option<u32>, Option<String>)> {
    let status = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("grpc-status"))
        .map(|header| {
            header
                .value
                .parse()
                .with_context(|| format!("invalid grpc-status header: {}", header.value))
        })
        .transpose()?;
    let message = headers
        .iter()
        .find(|header| header.key.eq_ignore_ascii_case("grpc-message"))
        .map(|header| percent_decode(&header.value))
        .transpose()?;
    Ok((status, message))
}

fn grpc_error(status: Option<u32>, message: Option<&str>) -> Option<String> {
    status.filter(|status| *status != 0).map(|status| {
        match message.filter(|message| !message.is_empty()) {
            Some(message) => format!("gRPC status {status}: {message}"),
            None => format!("gRPC status {status}"),
        }
    })
}

fn hex_digit(digit: u8) -> Result<u8> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => bail!("invalid percent-encoding in grpc-message"),
    }
}

fn hex_body(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(2 + bytes.len() * 2);
    result.push_str("0x");
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn transport_error(error: String, time_ms: u64) -> HttpResponse {
    HttpResponse {
        status: 0,
        status_text: "Error".into(),
        time_ms,
        error: Some(error),
        ..Default::default()
    }
}
