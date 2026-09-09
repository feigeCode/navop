//! Read-only Elasticsearch universal resource provider.
//!
//! The provider owns only transport translation. Elasticsearch permissions
//! remain host-authoritative: connection endpoints are checked by the host
//! before `resource/open`, and API keys are resolved through the reverse Host
//! API after the extension manifest's `secrets:read:*` permission is checked.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use extension_protocol::{
    blob::{BlobCloseParams, BlobReadParams, BlobReadResult, should_stream_blob},
    conn::SecretRef,
    envelope::{Request, RequestId, Response, RpcMessage},
    error::{ProtocolError, error_codes},
    event_stream::{
        DEFAULT_EVENT_MAX_EVENTS, EventCloseParams, EventOpenParams, EventOpenResult,
        EventReadParams, EventReadResult, MAX_EVENT_MAX_EVENTS,
    },
    framing::{recv_msg_async, send_msg_async},
    host::ResolveSecretParams,
    job::{
        JobCancelParams, JobCloseParams, JobResultParams, JobResultResult, JobStartParams,
        JobStartResult, JobState, JobStatusParams, JobStatusResult, ProgressPercent,
    },
    lifecycle::InitResult,
    method,
    resource::{
        ResourceCloseParams, ResourceInvokeParams, ResourceInvokeResult, ResourceOpenParams,
        ResourceOpenResult, ResourcePingParams,
    },
    result_ref::ResultRef,
};
use futures::StreamExt;
use interprocess::local_socket::{
    GenericNamespaced, ToNsName,
    tokio::{Stream, prelude::*},
};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;
use url::Url;
use uuid::Uuid;

const SOCKET_ENV_VAR: &str = "ONETCLI_EXT_SOCKET";
const PROVIDER_VERSION: &str = env!("CARGO_PKG_VERSION");
const RESOURCE_TYPE: &str = "elasticsearch";
const RESOURCE_ID: &str = "elasticsearch-resource";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_BLOB_BYTES: usize = 32 * 1024 * 1024;
const MAX_JOBS: usize = 64;
const MAX_EVENT_STREAMS: usize = 64;

type ProviderResult = Result<Value, Box<ProtocolError>>;

#[derive(Clone, Debug)]
struct ElasticsearchResource {
    base_url: String,
    api_key: Vec<u8>,
}

struct ProviderState {
    resource: Option<ElasticsearchResource>,
    blobs: HashMap<String, ProviderBlob>,
    jobs: HashMap<String, ProviderJob>,
    event_streams: HashMap<String, ProviderEventStream>,
    next_reverse_request_id: AtomicI64,
}

#[derive(Debug)]
struct ProviderBlob {
    data: Vec<u8>,
    offset: usize,
    closed: bool,
}

#[derive(Debug)]
struct ProviderJob {
    state: JobState,
    progress_percent: Option<ProgressPercent>,
    message: Option<String>,
    result: Option<JobResultResult>,
    completion: Option<mpsc::Receiver<Result<Value, Box<ProtocolError>>>>,
    cancellation: Option<oneshot::Sender<()>>,
}

#[derive(Debug)]
struct ProviderEventStream {
    kind: String,
    buffer: VecDeque<Value>,
    capacity: usize,
    dropped_count: u64,
    closed: bool,
}

struct IpcParts<R, W>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    reader: R,
    writer: W,
}

impl ProviderState {
    fn new() -> Self {
        Self {
            resource: None,
            blobs: HashMap::new(),
            jobs: HashMap::new(),
            event_streams: HashMap::new(),
            next_reverse_request_id: AtomicI64::new(1),
        }
    }

    fn total_blob_bytes(&self) -> usize {
        self.blobs.values().map(|blob| blob.data.len()).sum()
    }

    fn store_blob(&mut self, data: Vec<u8>) -> Option<String> {
        if data.len() > MAX_TOTAL_BLOB_BYTES
            || self.total_blob_bytes().saturating_add(data.len()) > MAX_TOTAL_BLOB_BYTES
        {
            return None;
        }
        let blob_id = format!("es-blob-{}", Uuid::new_v4());
        self.blobs.insert(
            blob_id.clone(),
            ProviderBlob {
                data,
                offset: 0,
                closed: false,
            },
        );
        Some(blob_id)
    }

    fn blob_result(&mut self, value: Value) -> Result<ResourceInvokeResult, Box<ProtocolError>> {
        let data = serde_json::to_vec(&value)
            .map_err(|error| boxed_invalid_params(format!("failed to encode result: {error}")))?;
        if !should_stream_blob(data.len() as u64) {
            return Ok(ResourceInvokeResult {
                result: ResultRef::Inline { value },
            });
        }
        let Some(blob_id) = self.store_blob(data) else {
            return Err(boxed_error(
                extension_protocol::error::error_codes::DATA_VALUE_OUT_OF_RANGE,
                "Elasticsearch result exceeds the provider blob budget",
            ));
        };
        Ok(ResourceInvokeResult {
            result: ResultRef::Blob {
                id: blob_id.clone(),
            },
        })
    }

    fn read_blob(&mut self, params: BlobReadParams) -> Result<BlobReadResult, Box<ProtocolError>> {
        let max_bytes = params.effective_max_bytes() as usize;
        let blob = self.blobs.get_mut(&params.blob_id).ok_or_else(|| {
            boxed_error(
                extension_protocol::error::error_codes::RESOURCE_CLOSED,
                "blob is closed or unknown",
            )
        })?;
        if blob.closed {
            return Err(boxed_error(
                extension_protocol::error::error_codes::RESOURCE_CLOSED,
                "blob is closed",
            ));
        }
        let start = blob.offset.min(blob.data.len());
        let end = start.saturating_add(max_bytes).min(blob.data.len());
        let bytes_read = end.saturating_sub(start) as u32;
        let done = end == blob.data.len() && bytes_read > 0;
        blob.offset = end;
        Ok(BlobReadResult {
            data: BASE64.encode(&blob.data[start..end]),
            bytes_read,
            done,
        })
    }

    fn close_blob(&mut self, params: BlobCloseParams) {
        self.blobs.remove(&params.blob_id);
    }

    fn open_event_stream(
        &mut self,
        params: EventOpenParams,
    ) -> Result<EventOpenResult, Box<ProtocolError>> {
        if params.kind != "elasticsearch/search/events" {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch event stream `{}`", params.kind),
            ));
        }
        if params.conn_id.is_some() {
            return Err(boxed_invalid_params(
                "Elasticsearch uses a single resource-scoped connection",
            ));
        }
        if self.resource.is_none() {
            return Err(resource_error());
        }
        if self.event_streams.len() >= MAX_EVENT_STREAMS {
            return Err(boxed_error(
                error_codes::RESOURCE_BUSY,
                "Elasticsearch event stream limit reached",
            ));
        }
        let stream_id = format!("es-stream-{}", Uuid::new_v4());
        self.event_streams.insert(
            stream_id.clone(),
            ProviderEventStream {
                kind: params.kind,
                buffer: VecDeque::new(),
                capacity: params
                    .capacity
                    .unwrap_or(DEFAULT_EVENT_MAX_EVENTS)
                    .clamp(1, MAX_EVENT_MAX_EVENTS) as usize,
                dropped_count: 0,
                closed: false,
            },
        );
        Ok(EventOpenResult { stream_id })
    }

    fn push_event(&mut self, stream_id: &str, event: Value) {
        let Some(stream) = self.event_streams.get_mut(stream_id) else {
            return;
        };
        if stream.closed {
            return;
        }
        if stream.buffer.len() >= stream.capacity {
            let _ = stream.buffer.pop_front();
            stream.dropped_count = stream.dropped_count.saturating_add(1);
        }
        stream.buffer.push_back(event);
    }

    fn broadcast_event(&mut self, event: Value) {
        let stream_ids = self
            .event_streams
            .iter()
            .filter(|(_, stream)| stream.kind == "elasticsearch/search/events")
            .map(|(stream_id, _)| stream_id.clone())
            .collect::<Vec<_>>();
        for stream_id in stream_ids {
            self.push_event(&stream_id, event.clone());
        }
    }

    fn read_event_stream(&mut self, params: EventReadParams) -> EventReadResult {
        let Some(stream) = self.event_streams.get_mut(&params.stream_id) else {
            return EventReadResult {
                events: Vec::new(),
                closed: true,
                dropped_count: 0,
            };
        };
        let count = params
            .effective_max_events()
            .min(stream.buffer.len() as u32) as usize;
        EventReadResult {
            events: stream.buffer.drain(..count).collect(),
            closed: stream.closed,
            dropped_count: stream.dropped_count,
        }
    }

    fn close_event_stream(&mut self, stream_id: &str) {
        self.event_streams.remove(stream_id);
    }

    fn emit_job_event(&mut self, job_id: &str) {
        let Some(job) = self.jobs.get(job_id) else {
            return;
        };
        let state = job.state;
        let progress_percent = job.progress_percent;
        let message = job.message.clone();
        let result = job.result.clone();
        let event = json!({
            "type": "job/completed",
            "job_id": job_id,
            "state": state,
            "progress_percent": progress_percent.map(u8::from),
            "message": message,
            "result": result,
        });
        self.broadcast_event(event);
    }

    fn start_job(
        &mut self,
        params: JobStartParams,
        resource: ElasticsearchResource,
    ) -> Result<JobStartResult, Box<ProtocolError>> {
        if params.resource_id.as_deref() != Some(RESOURCE_ID) {
            return Err(resource_error());
        }
        if params.method != "elasticsearch/search/async" {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch job method `{}`", params.method),
            ));
        }
        if self.jobs.len() >= MAX_JOBS {
            return Err(boxed_error(
                error_codes::RESOURCE_BUSY,
                "Elasticsearch job limit reached",
            ));
        }
        search_body(&params.params)?;
        let job_id = format!("es-job-{}", Uuid::new_v4());
        let (completion_tx, completion_rx) = mpsc::channel(1);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::select! {
                result = execute(&resource, "elasticsearch/search", &params.params) => {
                    let _ = completion_tx.send(result).await;
                }
                _ = cancel_rx => {}
            }
        });
        self.jobs.insert(
            job_id.clone(),
            ProviderJob {
                state: JobState::Running,
                progress_percent: None,
                message: Some("search running".to_owned()),
                result: None,
                completion: Some(completion_rx),
                cancellation: Some(cancel_tx),
            },
        );
        Ok(JobStartResult {
            job_id,
            state: JobState::Running,
        })
    }

    fn poll_job(&mut self, job_id: &str) -> bool {
        let outcome = {
            let Some(job) = self.jobs.get_mut(job_id) else {
                return false;
            };
            if job.state != JobState::Running {
                return false;
            }
            let Some(mut completion) = job.completion.take() else {
                job.state = JobState::Failed;
                job.message = Some("search worker is unavailable".to_owned());
                return true;
            };
            match completion.try_recv() {
                Ok(Ok(value)) => Some(Ok(value)),
                Ok(Err(error)) => Some(Err(error.message)),
                Err(mpsc::error::TryRecvError::Empty) => {
                    job.completion = Some(completion);
                    None
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Some(Err("search worker terminated".to_owned()))
                }
            }
        };

        let Some(outcome) = outcome else {
            return false;
        };
        match outcome {
            Ok(value) => self.complete_job(job_id, value),
            Err(message) => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.state = JobState::Failed;
                    job.progress_percent = None;
                    job.message = Some(message);
                    job.result = None;
                }
                true
            }
        }
    }

    fn complete_job(&mut self, job_id: &str, value: Value) -> bool {
        let value = normalize_search(value);
        let data = match serde_json::to_vec(&value) {
            Ok(data) => data,
            Err(error) => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.state = JobState::Failed;
                    job.message = Some(error.to_string());
                    job.result = None;
                }
                return true;
            }
        };
        let result_ref = if should_stream_blob(data.len() as u64) {
            match self.store_blob(data) {
                Some(blob_id) => ResultRef::Blob { id: blob_id },
                None => {
                    if let Some(job) = self.jobs.get_mut(job_id) {
                        job.state = JobState::Failed;
                        job.progress_percent = None;
                        job.message = Some(
                            "Elasticsearch job result exceeds the provider blob budget".to_owned(),
                        );
                        job.result = None;
                    }
                    return true;
                }
            }
        } else {
            ResultRef::Inline { value }
        };
        if let Some(job) = self.jobs.get_mut(job_id) {
            job.state = JobState::Succeeded;
            job.progress_percent = Some(ProgressPercent::new(100).expect("valid progress"));
            job.message = Some("search completed".to_owned());
            job.result = Some(JobResultResult { result: result_ref });
        }
        true
    }

    fn job_status(&self, params: JobStatusParams) -> Result<JobStatusResult, Box<ProtocolError>> {
        let job = self
            .jobs
            .get(&params.job_id)
            .ok_or_else(|| boxed_error(error_codes::RESOURCE_CLOSED, "job is closed or unknown"))?;
        Ok(JobStatusResult {
            job_id: params.job_id,
            state: job.state,
            progress_percent: job.progress_percent,
            message: job.message.clone(),
        })
    }

    fn cancel_job(&mut self, params: JobCancelParams) -> Result<bool, Box<ProtocolError>> {
        let Some(job) = self.jobs.get_mut(&params.job_id) else {
            return Err(boxed_error(
                error_codes::RESOURCE_CLOSED,
                "job is closed or unknown",
            ));
        };
        if job.state == JobState::Running {
            job.state = JobState::Cancelled;
            job.progress_percent = None;
            job.message = Some("search cancelled".to_owned());
            job.result = None;
            job.completion = None;
            if let Some(cancel) = job.cancellation.take() {
                let _ = cancel.send(());
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn job_result(&self, params: JobResultParams) -> Result<JobResultResult, Box<ProtocolError>> {
        let job = self
            .jobs
            .get(&params.job_id)
            .ok_or_else(|| boxed_error(error_codes::RESOURCE_CLOSED, "job is closed or unknown"))?;
        match job.state {
            JobState::Succeeded => {}
            JobState::Running => {
                return Err(boxed_error(
                    error_codes::RESOURCE_BUSY,
                    "job result is not ready",
                ));
            }
            JobState::Cancelled => {
                return Err(boxed_error(
                    error_codes::REQUEST_CANCELLED,
                    "job was cancelled",
                ));
            }
            state => {
                return Err(boxed_error(
                    error_codes::INTERNAL_ERROR,
                    format!("job failed with state `{state:?}`"),
                ));
            }
        }
        job.result
            .clone()
            .ok_or_else(|| boxed_error(error_codes::INTERNAL_ERROR, "job result is unavailable"))
    }

    fn close_job(&mut self, params: JobCloseParams) -> Result<(), Box<ProtocolError>> {
        if let Some(mut job) = self.jobs.remove(&params.job_id) {
            if job.state == JobState::Running {
                if let Some(cancel) = job.cancellation.take() {
                    let _ = cancel.send(());
                }
            }
            if let Some(JobResultResult {
                result: ResultRef::Blob { id },
            }) = job.result
            {
                self.blobs.remove(&id);
            }
        }
        Ok(())
    }
}

async fn resolve_secret<R, W>(
    ipc: &mut IpcParts<R, W>,
    secret_ref: &SecretRef,
    next_id: &AtomicI64,
) -> Result<Vec<u8>, Box<ProtocolError>>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let id = RequestId::Number(next_id.fetch_add(1, Ordering::SeqCst));
    let request = Request::new(
        id,
        method::HOST_RESOLVE_SECRET,
        serde_json::to_value(ResolveSecretParams {
            secret_ref: secret_ref.clone(),
        })
        .map_err(|error| boxed_invalid_params(error.to_string()))?,
    );
    send_msg_async(&mut ipc.writer, &RpcMessage::Request(request))
        .await
        .map_err(|error| {
            boxed_error(
                error_codes::INTERNAL_ERROR,
                format!("failed to request secret resolution: {error}"),
            )
        })?;

    let message = recv_msg_async::<_, RpcMessage>(&mut ipc.reader)
        .await
        .map_err(|error| {
            boxed_error(
                error_codes::INTERNAL_ERROR,
                format!("failed to receive secret resolution response: {error}"),
            )
        })?;
    let RpcMessage::Response(response) = message else {
        return Err(boxed_error(
            error_codes::INTERNAL_ERROR,
            "secret resolution returned an invalid RPC response",
        ));
    };
    if let Some(error) = response.error() {
        return Err(Box::new(error.clone()));
    }
    let Some(result_value) = response.result() else {
        return Err(boxed_error(
            error_codes::INTERNAL_ERROR,
            "secret resolution returned neither a result nor an error",
        ));
    };
    let result: extension_protocol::host::ResolveSecretResult =
        serde_json::from_value(result_value.clone())
            .map_err(|error| boxed_invalid_params(error.to_string()))?;
    Ok(result.value)
}

fn parse_open_params(params: Value) -> Result<(String, SecretRef), Box<ProtocolError>> {
    let params: ResourceOpenParams =
        serde_json::from_value(params).map_err(|error| boxed_invalid_params(error.to_string()))?;
    if params.resource_type != RESOURCE_TYPE {
        return Err(boxed_invalid_params(format!(
            "resource type must be `{RESOURCE_TYPE}`"
        )));
    }
    let url = params
        .config
        .get("url")
        .and_then(Value::as_str)
        .and_then(|value| value.trim().parse::<Url>().ok())
        .ok_or_else(|| boxed_invalid_params("a valid `http` or `https` `url` is required"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(boxed_invalid_params(
            "`url` must be an HTTP(S) endpoint without path, query, or fragment",
        ));
    }
    let credential = params
        .config
        .get("credential_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| boxed_invalid_params("`credential_ref` is required"))?;
    Ok((url.to_string(), SecretRef::new(credential)))
}

async fn execute(
    resource: &ElasticsearchResource,
    method_name: &str,
    params: &Value,
) -> ProviderResult {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| boxed_error(error_codes::INTERNAL_ERROR, error.to_string()))?;
    let mut request = match method_name {
        "elasticsearch/cluster/info" => client.get(resource.base_url.as_str()),
        "elasticsearch/index/list" => client
            .get(format!("{}/_cat/indices?format=json", resource.base_url))
            .header("Accept", "application/json"),
        "elasticsearch/index/get" => {
            let index = index_name(params)?;
            client
                .get(format!("{}/{}", resource.base_url, index))
                .header("Accept", "application/json")
        }
        "elasticsearch/search" => client
            .post(format!("{}/_search", resource.base_url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(search_body(params)?),
        _ => {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch method `{method_name}`"),
            ));
        }
    };
    request = request.header(
        "Authorization",
        format!("ApiKey {}", String::from_utf8_lossy(&resource.api_key)),
    );
    let response = request.send().await.map_err(|_error| {
        boxed_error(
            error_codes::IO_CONNECTION_REFUSED,
            "Elasticsearch request failed",
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let _body = bounded_body(response).await?;
        return Err(boxed_error(
            error_codes::IO_CONNECTION_REFUSED,
            format!("Elasticsearch returned HTTP status {status}; response body omitted"),
        ));
    }
    let body = bounded_body(response).await?;
    serde_json::from_slice::<Value>(&body).map_err(|_error| {
        boxed_error(
            error_codes::DATA_INVALID_ENCODING,
            "Elasticsearch returned invalid JSON",
        )
    })
}

async fn validate_connection(resource: &ElasticsearchResource) -> Result<(), Box<ProtocolError>> {
    let value = execute(resource, "elasticsearch/cluster/info", &Value::Null).await?;
    if !value.is_object() {
        return Err(boxed_error(
            error_codes::DATA_INVALID_ENCODING,
            "Elasticsearch returned an invalid cluster information response",
        ));
    }
    Ok(())
}

async fn bounded_body(response: reqwest::Response) -> Result<Vec<u8>, Box<ProtocolError>> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_error| {
            boxed_error(
                error_codes::IO_CONNECTION_REFUSED,
                "Elasticsearch response read failed",
            )
        })?;
        if body.len() + chunk.len() > MAX_HTTP_BODY_BYTES {
            return Err(boxed_error(
                error_codes::DATA_VALUE_OUT_OF_RANGE,
                "Elasticsearch response exceeds the provider limit",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn index_name(params: &Value) -> Result<String, Box<ProtocolError>> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| boxed_invalid_params("index name is required"))?;
    if name.len() > 256 || name.contains('/') || name.contains('?') || name.contains('#') {
        return Err(boxed_invalid_params("invalid index name"));
    }
    Ok(name.to_owned())
}

fn search_body(params: &Value) -> Result<String, Box<ProtocolError>> {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| boxed_invalid_params("non-empty search query is required"))?;
    let body = json!({ "query": { "match": { "_all": query } } });
    serde_json::to_string(&body).map_err(|error| boxed_invalid_params(error.to_string()))
}

fn normalize_indices(value: Value) -> Value {
    let Some(indices) = value.as_array() else {
        return json!({ "indices": value });
    };
    let normalized: Vec<Value> = indices
        .iter()
        .map(|index| {
            json!({
                "name": index.get("index").or_else(|| index.get("name")).cloned().unwrap_or(Value::Null),
                "health": index.get("health").cloned().unwrap_or(Value::Null),
                "docs": index.get("docs.count").cloned().unwrap_or(Value::Null),
                "size_bytes": index.get("store.size").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    json!({ "indices": normalized })
}

fn normalize_index(value: Value) -> Value {
    value
}

fn normalize_search(value: Value) -> Value {
    json!({ "raw": value })
}

async fn handle_request<R, W>(
    ipc: &mut IpcParts<R, W>,
    state: &mut ProviderState,
    request: extension_protocol::Request,
) -> (Response, bool)
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let should_exit = request.method == method::SHUTDOWN;
    let result = match request.method.as_str() {
        method::INIT => serde_json::to_value(
            InitResult::new(PROVIDER_VERSION)
                .with_api("extension", "1.0")
                .with_method(method::RESOURCE_OPEN)
                .with_method(method::RESOURCE_PING)
                .with_method(method::RESOURCE_INVOKE)
                .with_method(method::RESOURCE_CLOSE)
                .with_method(method::BLOB_READ)
                .with_method(method::BLOB_CLOSE)
                .with_method(method::JOB_START)
                .with_method(method::JOB_STATUS)
                .with_method(method::JOB_CANCEL)
                .with_method(method::JOB_RESULT)
                .with_method(method::JOB_CLOSE)
                .with_method(method::EVENT_OPEN)
                .with_method(method::EVENT_READ)
                .with_method(method::EVENT_CLOSE),
        )
        .map_err(|error| boxed_invalid_params(error.to_string())),
        method::RESOURCE_OPEN => {
            match state.resource.is_none() {
                true => {
                    let (url, credential_ref) = match parse_open_params(request.params.clone()) {
                        Ok(value) => value,
                        Err(error) => return (Response::err(request.id, *error), false),
                    };
                    let api_key =
                        match resolve_secret(ipc, &credential_ref, &state.next_reverse_request_id)
                            .await
                        {
                            Ok(value) => value,
                            Err(error) => return (Response::err(request.id, *error), false),
                        };
                    let resource = ElasticsearchResource {
                        base_url: url.trim_end_matches('/').to_owned(),
                        api_key,
                    };
                    if let Err(error) = validate_connection(&resource).await {
                        return (Response::err(request.id, *error), false);
                    }
                    state.resource = Some(resource);
                    serde_json::to_value(ResourceOpenResult {
                        resource_id: RESOURCE_ID.to_owned(),
                        capabilities: vec![
                            "elasticsearch/index/list".to_owned(),
                            "elasticsearch/index/get".to_owned(),
                            "elasticsearch/search".to_owned(),
                        ],
                        metadata: Some(
                            json!({ "mode": "http", "network": true, "operations": "read-only" }),
                        ),
                    })
                    .map_err(|error| boxed_invalid_params(error.to_string()))
                }
                false => Err(boxed_invalid_params(
                    "Elasticsearch resource is already open",
                )),
            }
        }
        method::RESOURCE_PING => {
            let params: ResourcePingParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            if &params.resource_id != RESOURCE_ID || state.resource.is_none() {
                return (Response::err(request.id, *resource_error()), false);
            }
            Ok(Value::Null)
        }
        method::BLOB_OPEN => Err(boxed_error(
            error_codes::METHOD_NOT_FOUND,
            "Elasticsearch results are opened by resource invoke",
        )),
        method::BLOB_READ => {
            let params: BlobReadParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            match state.read_blob(params) {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|error| boxed_invalid_params(error.to_string())),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::BLOB_CLOSE => {
            let params: BlobCloseParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            state.close_blob(params);
            Ok(Value::Null)
        }
        method::JOB_START => {
            let params: JobStartParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            let Some(resource) = state.resource.clone() else {
                return (Response::err(request.id, *resource_error()), false);
            };
            match state.start_job(params, resource) {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|error| boxed_invalid_params(error.to_string())),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::JOB_STATUS => {
            let params: JobStatusParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            if state.poll_job(&params.job_id) {
                state.emit_job_event(&params.job_id);
            }
            match state.job_status(params) {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|error| boxed_invalid_params(error.to_string())),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::JOB_CANCEL => {
            let params: JobCancelParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            let job_id = params.job_id.clone();
            match state.cancel_job(params) {
                Ok(true) => {
                    state.emit_job_event(&job_id);
                    Ok(Value::Null)
                }
                Ok(false) => Ok(Value::Null),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::JOB_RESULT => {
            let params: JobResultParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            if state.poll_job(&params.job_id) {
                state.emit_job_event(&params.job_id);
            }
            match state.job_result(params) {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|error| boxed_invalid_params(error.to_string())),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::JOB_CLOSE => {
            let params: JobCloseParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            match state.close_job(params) {
                Ok(()) => Ok(Value::Null),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::EVENT_OPEN => {
            let params: EventOpenParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            match state.open_event_stream(params) {
                Ok(result) => serde_json::to_value(result)
                    .map_err(|error| boxed_invalid_params(error.to_string())),
                Err(error) => return (Response::err(request.id, *error), false),
            }
        }
        method::EVENT_READ => {
            let params: EventReadParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            serde_json::to_value(state.read_event_stream(params))
                .map_err(|error| boxed_invalid_params(error.to_string()))
        }
        method::EVENT_CLOSE => {
            let params: EventCloseParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            state.close_event_stream(&params.stream_id);
            Ok(Value::Null)
        }
        method::RESOURCE_INVOKE => {
            let params: ResourceInvokeParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            let Some(resource) = state.resource.as_ref() else {
                return (Response::err(request.id, *resource_error()), false);
            };
            let value = match execute(resource, &params.method, &params.params).await {
                Ok(value) => match params.method.as_str() {
                    "elasticsearch/index/list" => normalize_indices(value),
                    "elasticsearch/index/get" => normalize_index(value),
                    "elasticsearch/search" => normalize_search(value),
                    _ => value,
                },
                Err(error) => return (Response::err(request.id, *error), false),
            };
            let result = match state.blob_result(value) {
                Ok(result) => result,
                Err(error) => return (Response::err(request.id, *error), false),
            };
            serde_json::to_value(result).map_err(|error| boxed_invalid_params(error.to_string()))
        }
        method::RESOURCE_CLOSE => {
            let params: ResourceCloseParams = match serde_json::from_value(request.params) {
                Ok(value) => value,
                Err(error) => {
                    return (
                        Response::err(request.id, *boxed_invalid_params(error.to_string())),
                        false,
                    );
                }
            };
            if &params.resource_id != RESOURCE_ID || state.resource.is_none() {
                return (Response::err(request.id, *resource_error()), false);
            }
            state.resource = None;
            state.blobs.clear();
            state.jobs.clear();
            state.event_streams.clear();
            Ok(Value::Null)
        }
        method::SHUTDOWN => {
            state.resource = None;
            state.blobs.clear();
            state.jobs.clear();
            state.event_streams.clear();
            Ok(Value::Null)
        }
        _ => Err(boxed_error(
            error_codes::METHOD_NOT_FOUND,
            format!("unknown method `{}`", request.method),
        )),
    };
    let response = match result {
        Ok(result) => Response::ok(request.id, result),
        Err(error) => Response::err(request.id, *error),
    };
    (response, should_exit)
}

fn boxed_error(
    code: extension_protocol::error::ErrorCode,
    message: impl Into<String>,
) -> Box<ProtocolError> {
    Box::new(ProtocolError::new(code, message))
}

fn boxed_invalid_params(message: impl Into<String>) -> Box<ProtocolError> {
    boxed_error(error_codes::INVALID_PARAMS, message)
}

fn resource_error() -> Box<ProtocolError> {
    Box::new(ProtocolError::new(
        error_codes::RESOURCE_CLOSED,
        "Elasticsearch resource is not open",
    ))
}

async fn run<R, W>(mut reader: R, mut writer: W) -> (R, W)
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut state = ProviderState::new();
    while let Ok(message) = recv_msg_async::<_, RpcMessage>(&mut reader).await {
        let RpcMessage::Request(request) = message else {
            continue;
        };
        let mut ipc = IpcParts { reader, writer };
        let (response, should_exit) = handle_request(&mut ipc, &mut state, request).await;
        reader = ipc.reader;
        writer = ipc.writer;
        if send_msg_async(&mut writer, &RpcMessage::Response(response))
            .await
            .is_err()
        {
            break;
        }
        if should_exit {
            break;
        }
    }
    (reader, writer)
}

#[tokio::main]
async fn main() {
    let socket_name = std::env::var(SOCKET_ENV_VAR).unwrap_or_else(|error| {
        eprintln!("missing {SOCKET_ENV_VAR}: {error}");
        std::process::exit(2);
    });
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .expect("valid host-provided local socket name");
    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, Stream::connect(name)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            eprintln!("failed to connect extension socket: {error}");
            std::process::exit(3);
        }
        Err(_) => {
            eprintln!("timed out connecting extension socket");
            std::process::exit(4);
        }
    };

    let (reader, writer) = tokio::io::split(stream);
    let (reader, mut writer) = run(reader, writer).await;
    let _ = writer.shutdown().await;
    let _ = reader;
}
