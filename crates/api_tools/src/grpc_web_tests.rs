use std::sync::{Arc, Mutex};

use futures::AsyncReadExt as _;
use futures::future::{BoxFuture, FutureExt as _};
use gpui::http_client::{AsyncBody, HttpClient, Response, Url, http};

use crate::grpc_web::{decode_response, execute, frame_request, prepare_grpc_web_request};
use crate::http::{PreparedRequest, RequestMethod};

#[test]
fn grpc_web_request_normalizes_scheme_frames_body_and_adds_headers() {
    let request = PreparedRequest {
        method: RequestMethod::Get,
        url: "grpcs://example.test/demo.Service/Call".into(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: br#"{"name":"Navop"}"#.to_vec(),
    };

    let prepared = prepare_grpc_web_request(request, 30).expect("prepare gRPC-Web request");

    assert_eq!(prepared.method, RequestMethod::Post);
    assert_eq!(prepared.url, "https://example.test/demo.Service/Call");
    assert_eq!(prepared.body, frame_request(br#"{"name":"Navop"}"#));
    assert_header(&prepared, "content-type", "application/grpc-web+json");
    assert_header(&prepared, "x-grpc-web", "1");
    assert_header(&prepared, "accept", "application/grpc-web+json");
    assert_header(&prepared, "grpc-timeout", "30S");
}

#[test]
fn grpc_web_request_preserves_custom_grpc_headers_without_duplicates() {
    let request = PreparedRequest {
        method: RequestMethod::Post,
        url: "grpc://example.test/demo.Service/Call".into(),
        headers: vec![
            ("content-type".into(), "application/grpc-web+proto".into()),
            ("X-Grpc-Web".into(), "custom".into()),
            ("x-user-agent".into(), "custom-agent".into()),
            ("grpc-timeout".into(), "250m".into()),
        ],
        body: Vec::new(),
    };

    let prepared = prepare_grpc_web_request(request, 30).expect("prepare gRPC-Web request");

    assert_eq!(prepared.url, "http://example.test/demo.Service/Call");
    assert_header(&prepared, "content-type", "application/grpc-web+proto");
    assert_header(&prepared, "x-grpc-web", "custom");
    assert_header(&prepared, "x-user-agent", "custom-agent");
    assert_header(&prepared, "grpc-timeout", "250m");
    for name in ["content-type", "x-grpc-web", "x-user-agent", "grpc-timeout"] {
        assert_eq!(
            prepared
                .headers
                .iter()
                .filter(|(header, _)| header.eq_ignore_ascii_case(name))
                .count(),
            1
        );
    }
}

#[test]
fn grpc_web_request_rejects_unrelated_url_schemes() {
    let request = PreparedRequest {
        method: RequestMethod::Post,
        url: "ws://example.test/demo.Service/Call".into(),
        headers: Vec::new(),
        body: Vec::new(),
    };

    let error = prepare_grpc_web_request(request, 30).expect_err("scheme should be rejected");
    assert!(
        error
            .to_string()
            .contains("unsupported gRPC-Web URL scheme")
    );
}

#[test]
fn grpc_web_decoder_joins_data_frames_and_parses_trailers() {
    let mut response = frame_request(br#"{"one":1}"#);
    response.extend(frame_request(br#"{"two":2}"#));
    response.extend(trailer_frame(
        b"grpc-status: 7\r\ngrpc-message: permission%20denied\r\n",
    ));

    let decoded = decode_response(&response).expect("decode gRPC-Web response");

    assert_eq!(decoded.payload, br#"{"one":1}{"two":2}"#);
    assert_eq!(decoded.grpc_status, Some(7));
    assert_eq!(decoded.grpc_message.as_deref(), Some("permission denied"));
}

#[test]
fn grpc_web_decoder_reports_compression_and_truncation() {
    let compressed = [vec![0x01, 0, 0, 0, 1], vec![b'x']].concat();
    let compression_error = decode_response(&compressed).expect_err("compression unsupported");
    assert!(compression_error.to_string().contains("compressed"));

    let truncated = [vec![0x00, 0, 0, 0, 4], vec![b'a', b'b']].concat();
    let truncation_error = decode_response(&truncated).expect_err("frame is truncated");
    assert!(
        truncation_error
            .to_string()
            .contains("truncated frame payload")
    );
}

#[test]
fn grpc_web_decoder_requires_trailers_to_be_the_final_frame() {
    let mut data_after_trailers = trailer_frame(b"grpc-status: 0\r\n");
    data_after_trailers.extend(frame_request(b"late"));
    let error = decode_response(&data_after_trailers).expect_err("data after trailers should fail");
    assert!(
        error
            .to_string()
            .contains("trailers must be the final frame")
    );

    let repeated_trailers = [
        trailer_frame(b"grpc-status: 0\r\n"),
        trailer_frame(b"grpc-status: 13\r\n"),
    ]
    .concat();
    let error = decode_response(&repeated_trailers).expect_err("repeated trailers should fail");
    assert!(error.to_string().contains("trailers may only appear once"));
}

#[test]
fn grpc_web_decoder_lossily_decodes_invalid_utf8_grpc_messages() {
    let decoded = decode_response(&trailer_frame(
        b"grpc-status: 13\r\ngrpc-message: invalid%FFmessage\r\n",
    ))
    .expect("decode trailers");

    assert_eq!(decoded.grpc_status, Some(13));
    assert_eq!(
        decoded.grpc_message.as_deref(),
        Some("invalid\u{fffd}message")
    );
}

#[test]
fn grpc_web_execute_sends_framed_post_and_pretty_prints_json_response() {
    let response_body = [
        frame_request(br#"{"ok":true}"#),
        trailer_frame(b"grpc-status: 0\r\n"),
    ]
    .concat();
    let client = RecordingHttpClient::new(
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc-web+json")
            .body(AsyncBody::from(response_body))
            .expect("response"),
    );
    let prepared = prepare_grpc_web_request(
        PreparedRequest {
            method: RequestMethod::Get,
            url: "example.test/demo.Service/Call".into(),
            headers: Vec::new(),
            body: br#"{"name":"Navop"}"#.to_vec(),
        },
        1,
    )
    .expect("prepare");

    let response = smol::block_on(execute(&client, prepared, 1));
    let captured = client.take_request();

    assert_eq!(captured.method, http::Method::POST);
    assert_eq!(captured.uri, "http://example.test/demo.Service/Call");
    assert_eq!(captured.body, frame_request(br#"{"name":"Navop"}"#));
    assert_eq!(
        captured
            .headers
            .get("grpc-timeout")
            .and_then(|value| value.to_str().ok()),
        Some("1S")
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.raw_body, r#"{"ok":true}"#);
    assert_eq!(response.body, "{\n  \"ok\": true\n}");
    assert!(response.is_json);
    assert_eq!(response.error, None);
}

#[test]
fn grpc_web_execute_surfaces_nonzero_grpc_status() {
    let body = [
        frame_request(b"{}"),
        trailer_frame(b"grpc-status: 13\r\ngrpc-message: internal%20error\r\n"),
    ]
    .concat();
    let client = RecordingHttpClient::new(
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc-web+json")
            .body(AsyncBody::from(body))
            .expect("response"),
    );
    let prepared = prepare_grpc_web_request(
        PreparedRequest {
            method: RequestMethod::Post,
            url: "https://example.test/demo.Service/Call".into(),
            headers: Vec::new(),
            body: b"{}".to_vec(),
        },
        1,
    )
    .expect("prepare");

    let response = smol::block_on(execute(&client, prepared, 1));

    assert_eq!(
        response.error.as_deref(),
        Some("gRPC status 13: internal error")
    );
}

#[test]
fn grpc_web_execute_reads_header_only_grpc_errors() {
    let client = RecordingHttpClient::new(
        Response::builder()
            .status(200)
            .header("content-type", "application/grpc-web+json")
            .header("grpc-status", "13")
            .header("grpc-message", "internal%20error")
            .body(AsyncBody::from(Vec::<u8>::new()))
            .expect("response"),
    );
    let prepared = prepare_grpc_web_request(
        PreparedRequest {
            method: RequestMethod::Post,
            url: "https://example.test/demo.Service/Call".into(),
            headers: Vec::new(),
            body: b"{}".to_vec(),
        },
        1,
    )
    .expect("prepare");

    let response = smol::block_on(execute(&client, prepared, 1));

    assert_eq!(response.raw_body, "");
    assert_eq!(
        response.error.as_deref(),
        Some("gRPC status 13: internal error")
    );
}

#[test]
fn grpc_web_execute_preserves_non_grpc_http_error_bodies() {
    let client = RecordingHttpClient::new(
        Response::builder()
            .status(502)
            .header("content-type", "text/html")
            .body(AsyncBody::from("<h1>Bad Gateway</h1>"))
            .expect("response"),
    );
    let prepared = prepare_grpc_web_request(
        PreparedRequest {
            method: RequestMethod::Post,
            url: "https://example.test/demo.Service/Call".into(),
            headers: Vec::new(),
            body: b"{}".to_vec(),
        },
        1,
    )
    .expect("prepare");

    let response = smol::block_on(execute(&client, prepared, 1));

    assert_eq!(response.raw_body, "<h1>Bad Gateway</h1>");
    assert_eq!(response.body, "<h1>Bad Gateway</h1>");
    assert_eq!(response.error.as_deref(), Some("HTTP 502 Bad Gateway"));
}

#[test]
fn grpc_web_execute_preserves_json_for_wrong_success_content_type() {
    let client = RecordingHttpClient::new(
        Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(AsyncBody::from(br#"{"error":"wrong upstream"}"#.to_vec()))
            .expect("response"),
    );
    let prepared = prepare_grpc_web_request(
        PreparedRequest {
            method: RequestMethod::Post,
            url: "https://example.test/demo.Service/Call".into(),
            headers: Vec::new(),
            body: b"{}".to_vec(),
        },
        1,
    )
    .expect("prepare");

    let response = smol::block_on(execute(&client, prepared, 1));

    assert_eq!(response.raw_body, r#"{"error":"wrong upstream"}"#);
    assert_eq!(response.body, "{\n  \"error\": \"wrong upstream\"\n}");
    assert!(response.is_json);
    assert_eq!(
        response.error.as_deref(),
        Some("expected application/grpc-web response, got application/json")
    );
}

fn trailer_frame(trailers: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + trailers.len());
    frame.push(0x80);
    frame.extend_from_slice(&(trailers.len() as u32).to_be_bytes());
    frame.extend_from_slice(trailers);
    frame
}

fn assert_header(request: &PreparedRequest, name: &str, value: &str) {
    assert!(
        request
            .headers
            .iter()
            .any(|(header, actual)| header.eq_ignore_ascii_case(name) && actual == value),
        "missing {name}: {value:?} in {:?}",
        request.headers
    );
}

#[derive(Debug)]
struct CapturedRequest {
    method: http::Method,
    uri: String,
    headers: http::HeaderMap,
    body: Vec<u8>,
}

struct RecordingHttpClient {
    response: Mutex<Option<Response<AsyncBody>>>,
    captured: Arc<Mutex<Option<CapturedRequest>>>,
}

impl RecordingHttpClient {
    fn new(response: Response<AsyncBody>) -> Self {
        Self {
            response: Mutex::new(Some(response)),
            captured: Arc::new(Mutex::new(None)),
        }
    }

    fn take_request(&self) -> CapturedRequest {
        self.captured
            .lock()
            .expect("captured request lock")
            .take()
            .expect("captured request")
    }
}

impl HttpClient for RecordingHttpClient {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn send(
        &self,
        request: http::Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let response = self
            .response
            .lock()
            .expect("response lock")
            .take()
            .expect("single response");
        let captured = self.captured.clone();
        async move {
            let (parts, mut body) = request.into_parts();
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;
            *captured.lock().expect("captured request lock") = Some(CapturedRequest {
                method: parts.method,
                uri: parts.uri.to_string(),
                headers: parts.headers,
                body: bytes,
            });
            Ok(response)
        }
        .boxed()
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}
