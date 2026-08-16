use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, Builder, HttpClient, HttpRequestExt, Method, RedirectPolicy};

use crate::http::{HttpResponse, KeyValue, PreparedRequest, RequestMethod};
use crate::sse_parser::{SseEvent, SseParser};

const READ_POLL_INTERVAL_MS: u64 = 100;
const READ_CHUNK_SIZE: usize = 4096;

enum SendOutcome {
    Response(gpui::http_client::Response<AsyncBody>),
    Stopped,
}

#[derive(Debug, Clone, Default)]
pub struct SseProgress {
    pub response: HttpResponse,
    pub done: bool,
}

pub fn prepare_sse_request(mut request: PreparedRequest) -> PreparedRequest {
    if !request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
    {
        request
            .headers
            .push(("Accept".into(), "text/event-stream".into()));
    }
    request
}

pub async fn execute(
    client: &dyn HttpClient,
    request: PreparedRequest,
    timeout_secs: u64,
    stop: Arc<AtomicBool>,
    progress: Arc<Mutex<SseProgress>>,
) -> HttpResponse {
    let started = Instant::now();
    let request = prepare_sse_request(request);
    let response = match send(client, request, timeout_secs, &stop).await {
        Ok(SendOutcome::Response(response)) => response,
        Ok(SendOutcome::Stopped) => {
            return finish_stopped(started, &progress);
        }
        Err(error) => {
            return finish_with_error(error.to_string(), started, &progress);
        }
    };
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| KeyValue::new(name.as_str(), value.to_str().unwrap_or("<binary>")))
        .collect::<Vec<_>>();
    let mut result = HttpResponse {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").into(),
        headers,
        streaming: true,
        ..Default::default()
    };
    publish(&progress, &result, false);

    let mut parser = SseParser::default();
    let mut body = response.into_body();
    let mut chunk = [0_u8; READ_CHUNK_SIZE];
    loop {
        match read_chunk(&mut body, &mut chunk, &stop).await {
            Ok(Some(0)) => {
                append_events(&mut result, parser.finish());
                break;
            }
            Ok(Some(count)) => {
                result.size += count as u64;
                append_events(&mut result, parser.push(&chunk[..count]));
                result.time_ms = started.elapsed().as_millis() as u64;
                publish(&progress, &result, false);
            }
            Ok(None) => {
                append_events(&mut result, parser.finish());
                result.body.push_str("\n[stopped]\n");
                result.raw_body = result.body.clone();
                break;
            }
            Err(error) => {
                result.error = Some(error.to_string());
                break;
            }
        }
    }
    result.streaming = false;
    result.time_ms = started.elapsed().as_millis() as u64;
    publish(&progress, &result, true);
    result
}

async fn send(
    client: &dyn HttpClient,
    request: PreparedRequest,
    timeout_secs: u64,
    stop: &AtomicBool,
) -> anyhow::Result<SendOutcome> {
    let mut builder = Builder::new()
        .uri(&request.url)
        .method(http_method(request.method))
        .follow_redirects(RedirectPolicy::FollowAll);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    let request = builder.body(AsyncBody::from(request.body))?;
    smol::future::or(wait_for_stop_or_timeout(stop, timeout_secs), async {
        client.send(request).await.map(SendOutcome::Response)
    })
    .await
}

async fn wait_for_stop_or_timeout(
    stop: &AtomicBool,
    timeout_secs: u64,
) -> anyhow::Result<SendOutcome> {
    let timeout = Duration::from_secs(timeout_secs.max(1));
    let started = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(SendOutcome::Stopped);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!("request timed out after {timeout_secs}s"));
        }
        let interval = Duration::from_millis(READ_POLL_INTERVAL_MS);
        smol::Timer::after(remaining.min(interval)).await;
    }
}

fn http_method(method: RequestMethod) -> Method {
    match method {
        RequestMethod::Get => Method::GET,
        RequestMethod::Post => Method::POST,
        RequestMethod::Put => Method::PUT,
        RequestMethod::Delete => Method::DELETE,
        RequestMethod::Patch => Method::PATCH,
        RequestMethod::Head => Method::HEAD,
        RequestMethod::Options => Method::OPTIONS,
        RequestMethod::Trace => Method::TRACE,
    }
}

async fn read_chunk(
    body: &mut AsyncBody,
    chunk: &mut [u8],
    stop: &AtomicBool,
) -> std::io::Result<Option<usize>> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let result = smol::future::or(
            async {
                smol::Timer::after(Duration::from_millis(READ_POLL_INTERVAL_MS)).await;
                None
            },
            async { Some(body.read(chunk).await) },
        )
        .await;
        if let Some(result) = result {
            return result.map(Some);
        }
    }
}

fn append_events(response: &mut HttpResponse, events: Vec<SseEvent>) {
    for event in events {
        if let Some(id) = event.id {
            response.body.push_str(&format!("id: {id}\n"));
        }
        response.body.push_str(&format!("event: {}\n", event.event));
        for line in event.data.split('\n') {
            response.body.push_str(&format!("data: {line}\n"));
        }
        response.body.push('\n');
    }
    response.raw_body = response.body.clone();
}

fn publish(progress: &Mutex<SseProgress>, response: &HttpResponse, done: bool) {
    if let Ok(mut progress) = progress.lock() {
        progress.response = response.clone();
        progress.done = done;
    }
}

fn finish_with_error(
    error: String,
    started: Instant,
    progress: &Mutex<SseProgress>,
) -> HttpResponse {
    let response = HttpResponse {
        status_text: "Error".into(),
        time_ms: started.elapsed().as_millis() as u64,
        error: Some(error),
        ..Default::default()
    };
    publish(progress, &response, true);
    response
}

fn finish_stopped(started: Instant, progress: &Mutex<SseProgress>) -> HttpResponse {
    let response = HttpResponse {
        body: "\n[stopped]\n".into(),
        raw_body: "\n[stopped]\n".into(),
        time_ms: started.elapsed().as_millis() as u64,
        ..Default::default()
    };
    publish(progress, &response, true);
    response
}
