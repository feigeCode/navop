//! 基础版本地 Mock HTTP 服务。
//!
//! 这个模块刻意只实现 HTTP/1.1 的最小子集：读取请求行和请求头、匹配
//! [`MockRuleSet`]、生成响应并记录最近请求。规则和日志都放在共享状态中，
//! 因此编辑请求后可以直接替换规则，不需要重启服务。

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

use crate::{
    http::{KeyValue, substitute},
    mock::{MockRequestLike, MockRuleSet, url_decode},
};

const DEFAULT_LOG_CAPACITY: usize = 100;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// 一条 Mock 服务请求日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockRequestLog {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub matched: bool,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
}

/// Mock 服务的共享状态。
///
/// `MockServerState` 可被 UI 持有，用于热更新规则、读取请求日志和清空日志。
#[derive(Debug)]
pub struct MockServerState {
    rules: RwLock<Arc<MockRuleSet>>,
    logs: Mutex<VecDeque<MockRequestLog>>,
    log_capacity: usize,
}

impl MockServerState {
    pub fn new(rules: MockRuleSet) -> Arc<Self> {
        Self::with_log_capacity(rules, DEFAULT_LOG_CAPACITY)
    }

    pub fn with_log_capacity(rules: MockRuleSet, log_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            rules: RwLock::new(Arc::new(rules)),
            logs: Mutex::new(VecDeque::with_capacity(log_capacity)),
            log_capacity: log_capacity.max(1),
        })
    }

    pub fn replace_rules(&self, rules: MockRuleSet) {
        *self.rules.write().expect("mock rules lock poisoned") = Arc::new(rules);
    }

    pub fn rules(&self) -> Arc<MockRuleSet> {
        self.rules.read().expect("mock rules lock poisoned").clone()
    }

    pub fn logs(&self) -> Vec<MockRequestLog> {
        self.logs
            .lock()
            .expect("mock logs lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn clear_logs(&self) {
        self.logs.lock().expect("mock logs lock poisoned").clear();
    }

    fn push_log(&self, log: MockRequestLog) {
        let mut logs = self.logs.lock().expect("mock logs lock poisoned");
        logs.push_back(log);
        while logs.len() > self.log_capacity {
            logs.pop_front();
        }
    }
}

/// 正在运行的本地 Mock HTTP 服务。
pub struct MockServer {
    addr: SocketAddr,
    state: Arc<MockServerState>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MockServer {
    /// 在 loopback 随机端口启动服务。
    pub async fn bind(rules: MockRuleSet) -> Result<Self> {
        Self::bind_on(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0), rules).await
    }

    /// 在指定地址启动服务。使用端口 `0` 可让操作系统分配空闲端口。
    pub async fn bind_on(addr: SocketAddr, rules: MockRuleSet) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind mock server to {addr}"))?;
        let local_addr = listener
            .local_addr()
            .context("read mock server local address")?;
        let state = MockServerState::new(rules);
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            accept_loop(listener, task_state, shutdown_rx).await;
        });

        Ok(Self {
            addr: local_addr,
            state,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn state(&self) -> Arc<MockServerState> {
        self.state.clone()
    }

    pub fn replace_rules(&self, rules: MockRuleSet) {
        self.state.replace_rules(rules);
    }

    pub fn logs(&self) -> Vec<MockRequestLog> {
        self.state.logs()
    }

    pub fn clear_logs(&self) {
        self.state.clear_logs();
    }

    /// 停止 accept loop。已经建立的连接会在当前请求完成后结束。
    pub async fn stop(mut self) {
        self.stop_inner().await;
    }

    async fn stop_inner(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    state: Arc<MockServerState>,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = listener.accept() => {
                let Ok((stream, _peer)) = result else {
                    break;
                };
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve_connection(stream, state).await {
                        tracing::debug!(%error, "mock connection ended");
                    }
                });
            }
        }
    }
}

async fn serve_connection(mut stream: TcpStream, state: Arc<MockServerState>) -> Result<()> {
    let started = Instant::now();
    let request = read_request(&mut stream).await?;
    let Some(request) = request else {
        return Ok(());
    };
    let rules = state.rules();
    let matched = rules.find_match(&request);
    let (status, response_headers, response_body, rule_id, rule_name, delay_ms) =
        if let Some(entry) = matched {
            let body = render_template(&entry.rule.body, &request, entry.rule.enable_templates);
            let headers =
                render_headers(&entry.rule.headers, &request, entry.rule.enable_templates);
            (
                entry.rule.status,
                headers,
                body.into_bytes(),
                Some(entry.request_id.clone()),
                Some(entry.request_name.clone()),
                entry.rule.delay_ms,
            )
        } else {
            (
                404,
                vec![KeyValue::new("Content-Type", "text/plain; charset=utf-8")],
                format!("No mock rule matched {} {}\n", request.method, request.path).into_bytes(),
                None,
                None,
                0,
            )
        };

    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    let response = response_bytes(status, &response_headers, &response_body);
    stream.write_all(&response).await?;
    stream.flush().await?;

    state.push_log(MockRequestLog {
        method: request.method,
        path: request.path,
        status,
        duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        matched: rule_id.is_some(),
        rule_id,
        rule_name,
    });
    Ok(())
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
}

impl MockRequestLike for ParsedRequest {
    fn method(&self) -> &str {
        &self.method
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn query(&self) -> &HashMap<String, String> {
        &self.query
    }

    fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<Option<ParsedRequest>> {
    let mut buffer = Vec::with_capacity(4096);
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err(anyhow!("mock request exceeds {MAX_REQUEST_BYTES} bytes"));
        }
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };

    let header_text =
        std::str::from_utf8(&buffer[..header_end]).context("mock request is not UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().context("mock request line is missing")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .context("mock method is missing")?
        .to_uppercase();
    let target = request_parts
        .next()
        .context("mock request target is missing")?;
    let mut headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(key.trim().to_string(), value.trim().to_string());
    }

    let target = if let Some(scheme) = target.find("://") {
        let remainder = &target[scheme + 3..];
        remainder
            .find('/')
            .map(|index| &remainder[index..])
            .unwrap_or("/")
    } else {
        target
    };
    let (path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let query = raw_query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (!key.is_empty()).then(|| (url_decode(key).to_lowercase(), url_decode(value)))
        })
        .collect();

    Ok(Some(ParsedRequest {
        method,
        path: path.to_string(),
        query,
        headers,
    }))
}

fn render_headers(headers: &[KeyValue], request: &ParsedRequest, enabled: bool) -> Vec<KeyValue> {
    headers
        .iter()
        .filter(|header| header.enabled && !header.key.trim().is_empty())
        .map(|header| {
            let mut header = header.clone();
            if enabled {
                header.value = render_template(&header.value, request, true);
            }
            header
        })
        .collect()
}

fn render_template(input: &str, request: &ParsedRequest, enabled: bool) -> String {
    if !enabled {
        return input.to_string();
    }
    let mut vars = BTreeMap::new();
    vars.insert("mock.request.path".to_string(), request.path.clone());
    vars.insert("mock.request.method".to_string(), request.method.clone());
    for (key, value) in &request.query {
        vars.insert(format!("mock.request.query.{key}"), value.clone());
    }
    for (key, value) in &request.headers {
        vars.insert(format!("mock.request.header.{key}"), value.clone());
    }
    substitute(input, &vars)
}

fn response_bytes(status: u16, headers: &[KeyValue], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {} {}\r\n", status, status_reason(status));
    let mut has_content_type = false;
    for header in headers {
        if header.key.eq_ignore_ascii_case("content-length")
            || header.key.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        has_content_type |= header.key.eq_ignore_ascii_case("content-type");
        response.push_str(&header.key);
        response.push_str(": ");
        response.push_str(&header.value);
        response.push_str("\r\n");
    }
    if !has_content_type {
        response.push_str("Content-Type: application/json\r\n");
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Mock Response",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::RequestMethod,
        mock::{MockRule, PathPattern},
        request_store::StoredRequest,
    };

    fn request(path: &str, body: &str) -> StoredRequest {
        let mut request = StoredRequest::new("test", RequestMethod::Get);
        request.url = path.to_string();
        request.mock = Some(MockRule {
            match_path: PathPattern::Exact(path.to_string()),
            body: body.to_string(),
            enable_templates: true,
            ..Default::default()
        });
        request
    }

    #[tokio::test]
    async fn serves_matching_rule_and_records_log() {
        let rules =
            MockRuleSet::from_requests(&[request("/hello", r#"{"path":"{{mock.request.path}}"}"#)]);
        let server = MockServer::bind(rules).await.unwrap();
        let (status, body) = raw_get(server.addr(), "/hello").await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"{"path":"/hello"}"#);
        assert_eq!(server.logs().len(), 1);
        assert_eq!(server.logs()[0].rule_name.as_deref(), Some("test"));
    }

    #[tokio::test]
    async fn replaces_rules_without_restarting_server() {
        let first = request("/dynamic", r#"{"value":1}"#);
        let second = request("/dynamic", r#"{"value":2}"#);
        let server = MockServer::bind(MockRuleSet::from_requests(&[first]))
            .await
            .unwrap();
        let (_, body) = raw_get(server.addr(), "/dynamic").await;
        assert_eq!(body, r#"{"value":1}"#);

        server.replace_rules(MockRuleSet::from_requests(&[second]));
        let (_, body) = raw_get(server.addr(), "/dynamic").await;
        assert_eq!(body, r#"{"value":2}"#);
    }

    #[tokio::test]
    async fn returns_404_for_missing_rule() {
        let server = MockServer::bind(MockRuleSet::default()).await.unwrap();
        let (status, body) = raw_get(server.addr(), "/missing").await;
        assert_eq!(status, 404);
        assert!(body.contains("No mock rule matched"));
        assert!(!server.logs()[0].matched);
    }

    async fn raw_get(addr: SocketAddr, path: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (
            status,
            String::from_utf8(bytes[header_end + 4..].to_vec()).unwrap(),
        )
    }
}
