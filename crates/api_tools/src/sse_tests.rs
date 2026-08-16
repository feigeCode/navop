use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::{BoxFuture, FutureExt};
use futures::io::AsyncRead;
use gpui::http_client::{AsyncBody, HttpClient, Response, Url, http};

use crate::http::{PreparedRequest, RequestMethod};
use crate::sse::{self, SseProgress, prepare_sse_request};

#[test]
fn sse_requests_add_accept_header_without_overwriting_user_value() {
    let request = PreparedRequest {
        method: RequestMethod::Get,
        url: "https://example.test/events".into(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    let prepared = prepare_sse_request(request);
    assert!(
        prepared
            .headers
            .contains(&("Accept".into(), "text/event-stream".into()))
    );

    let custom = PreparedRequest {
        headers: vec![("accept".into(), "application/x-ndjson".into())],
        ..prepared
    };
    let prepared = prepare_sse_request(custom);
    assert_eq!(
        prepared
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("accept"))
            .count(),
        1
    );
}

#[test]
fn stopping_sse_cancels_connection_establishment() {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_timer = stop.clone();
    let progress = Arc::new(Mutex::new(SseProgress::default()));
    let client = PendingHttpClient;
    let request = test_request();

    let result = smol::block_on(smol::future::or(
        async { Some(sse::execute(&client, request, 1, stop, progress).await) },
        async {
            smol::Timer::after(Duration::from_millis(20)).await;
            stop_for_timer.store(true, Ordering::SeqCst);
            smol::Timer::after(Duration::from_millis(120)).await;
            None
        },
    ));

    assert!(result.is_some(), "stop must interrupt a pending handshake");
}

#[test]
fn stopping_sse_flushes_the_pending_event_before_marking_it_stopped() {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_timer = stop.clone();
    let progress = Arc::new(Mutex::new(SseProgress::default()));
    let client = StreamingHttpClient;
    let request = test_request();

    let response = smol::block_on(async {
        let execute = sse::execute(&client, request, 1, stop, progress);
        let stop_later = async move {
            smol::Timer::after(Duration::from_millis(20)).await;
            stop_for_timer.store(true, Ordering::SeqCst);
        };
        futures::future::join(execute, stop_later).await.0
    });

    assert!(response.body.contains("data: tail"));
}

fn test_request() -> PreparedRequest {
    PreparedRequest {
        method: RequestMethod::Get,
        url: "https://example.test/events".into(),
        headers: Vec::new(),
        body: Vec::new(),
    }
}

struct PendingHttpClient;

impl HttpClient for PendingHttpClient {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn send(
        &self,
        _request: http::Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        futures::future::pending().boxed()
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}

struct StreamingHttpClient;

impl HttpClient for StreamingHttpClient {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn send(
        &self,
        _request: http::Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let response = Response::builder()
            .status(200)
            .body(AsyncBody::from_reader(ChunkThenPending::new(b"data: tail")))
            .expect("response builder");
        async move { Ok(response) }.boxed()
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}

struct ChunkThenPending {
    chunk: Option<Vec<u8>>,
}

impl ChunkThenPending {
    fn new(chunk: &[u8]) -> Self {
        Self {
            chunk: Some(chunk.to_vec()),
        }
    }
}

impl AsyncRead for ChunkThenPending {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let Some(chunk) = self.chunk.take() else {
            return Poll::Pending;
        };
        let length = chunk.len().min(buffer.len());
        buffer[..length].copy_from_slice(&chunk[..length]);
        Poll::Ready(Ok(length))
    }
}
