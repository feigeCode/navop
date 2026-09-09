//! Remoting 连接管理:TCP 通道、opaque 配对、超时、NameServer 轮询与 SSH 隧道。
//!
//! - 每个目标地址(Nameserver/Broker)一条长连接:独立读写任务;
//! - 请求通过 `opaque` 与响应配对(`DashMap<opaque, oneshot>` + 读循环分发);
//! - 请求超时默认 3s(可配);NameServer 多地址轮询故障切换;
//! - 断线自动重连:通道失效即从缓存剔除,下次调用重建连接;
//! - SSH 隧道:每次建连时按 `connection_tunnel::resolve_connection_target`
//!   展开为本地端口前置(隧道随通道存活)。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::time::Duration;

use connection_tunnel::{TunnelGuard, resolve_connection_target};
use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, warn};

use crate::acl::AclCredentials;
use crate::protocol::{RemotingCommand, decode_frame, ensure_success, frame_body_length};
use crate::types::{RocketmqError, RocketmqParams};

/// 单通道写队列容量
const WRITER_CHANNEL_CAPACITY: usize = 128;

/// 写任务命令:发送帧或主动关闭连接
enum WriterCommand {
    /// 发送一帧(已编码的完整字节)
    Frame(Vec<u8>),
    /// 主动关闭连接(优雅 shutdown 写半)
    Close,
}

/// Remoting 客户端:管理 NameServer 与 Broker 通道
pub struct RemotingClient {
    params: Arc<RocketmqParams>,
    acl: Option<AclCredentials>,
    /// 目标地址(规范化 host:port)→ 活动通道
    channels: Mutex<HashMap<String, Arc<RemotingChannel>>>,
    /// NameServer 轮询游标
    namesrv_cursor: AtomicUsize,
}

impl RemotingClient {
    /// 创建客户端(不建立连接,首次调用时按需连接)
    pub fn new(params: Arc<RocketmqParams>) -> Result<Self, RocketmqError> {
        let addrs = params.normalized_namesrv_addrs();
        if addrs.is_empty() {
            return Err(RocketmqError::Config(
                "RocketMQ NameServer 地址列表为空".into(),
            ));
        }
        let acl = if params.acl_enabled() {
            match AclCredentials::new(
                params.access_key.as_deref().unwrap_or_default(),
                params.secret_key.as_deref().unwrap_or_default(),
                None,
            ) {
                Ok(credentials) => Some(credentials),
                Err(message) => {
                    warn!("RocketMQ ACL 凭据非法, 已忽略: {message}");
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            params,
            acl,
            channels: Mutex::new(HashMap::new()),
            namesrv_cursor: AtomicUsize::new(0),
        })
    }

    /// 连接配置引用
    pub fn params(&self) -> &RocketmqParams {
        &self.params
    }

    /// ACL 是否启用
    pub fn acl_enabled(&self) -> bool {
        self.acl.is_some()
    }

    /// 向 NameServer 发起请求(多地址轮询故障切换)
    pub async fn invoke_namesrv(
        &self,
        mut command: RemotingCommand,
    ) -> Result<RemotingCommand, RocketmqError> {
        let addrs = self.params.normalized_namesrv_addrs();
        let start = self.namesrv_cursor.fetch_add(1, Ordering::Relaxed);
        let mut last_error = None;
        for offset in 0..addrs.len() {
            let addr = &addrs[(start + offset) % addrs.len()];
            match self.invoke_addr(addr, &mut command).await {
                Ok(response) => {
                    ensure_success(&response)?;
                    return Ok(response);
                }
                // 连接级失败(连接断开/超时):切换下一个 NameServer
                Err(error @ RocketmqError::Connection(_))
                | Err(error @ RocketmqError::Timeout(_)) => {
                    warn!("NameServer {addr} 调用失败, 切换下一个: {error}");
                    last_error = Some(error);
                    self.invalidate(addr).await;
                }
                // 业务/协议级错误直接返回
                Err(error) => return Err(error),
            }
        }
        Err(last_error
            .unwrap_or_else(|| RocketmqError::Connection("全部 NameServer 地址均不可用".into())))
    }

    /// 向 Broker 发起请求(addr 为 `host:port`;4.x VIP 通道不启用)
    pub async fn invoke_broker(
        &self,
        addr: &str,
        mut command: RemotingCommand,
    ) -> Result<RemotingCommand, RocketmqError> {
        let response = self.invoke_addr(addr, &mut command).await?;
        ensure_success(&response)?;
        Ok(response)
    }

    /// 与 [`Self::invoke_broker`] 相同,但跳过响应码校验(调用方自行处理特殊码)
    pub async fn invoke_broker_raw(
        &self,
        addr: &str,
        mut command: RemotingCommand,
    ) -> Result<RemotingCommand, RocketmqError> {
        self.invoke_addr(addr, &mut command).await
    }

    /// 关闭全部通道(断开连接)
    pub async fn close(&self) {
        let mut channels = self.channels.lock().await;
        for (_, channel) in channels.drain() {
            channel.shutdown();
        }
    }

    /// 使目标地址的缓存通道失效(下次调用重建)
    async fn invalidate(&self, addr: &str) {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.remove(addr) {
            debug!("RocketMQ 通道失效: {addr}");
            channel.shutdown();
        }
    }

    /// 取缓存通道或新建连接
    async fn get_or_connect(&self, addr: &str) -> Result<Arc<RemotingChannel>, RocketmqError> {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.get(addr) {
            if channel.is_active() {
                return Ok(channel.clone());
            }
            // 已失效:剔除后重建
            let channel = channel.clone();
            channels.remove(addr);
            channel.shutdown();
        }
        let channel = Arc::new(RemotingChannel::connect(addr, &self.params).await?);
        channels.insert(addr.to_string(), channel.clone());
        Ok(channel)
    }

    /// 对目标地址执行一次调用(含 ACL 签名、opaque 配对与超时)。
    ///
    /// 公开给需要精确控制目标(如逐台 NameServer 删除路由)的调用方。
    pub async fn invoke_addr(
        &self,
        addr: &str,
        command: &mut RemotingCommand,
    ) -> Result<RemotingCommand, RocketmqError> {
        let normalized = crate::types::normalize_addr(addr, crate::types::DEFAULT_NAMESRV_PORT);
        let channel = self.get_or_connect(&normalized).await?;
        let result = channel.invoke(command, &self.acl).await;
        if let Err(error) = &result {
            // 连接级失败:剔除通道,触发下次重连
            if matches!(
                error,
                RocketmqError::Connection(_) | RocketmqError::Timeout(_)
            ) {
                self.invalidate(&normalized).await;
            }
        }
        result
    }
}

/// 单地址 Remoting 通道
pub struct RemotingChannel {
    /// 目标地址(未展开隧道的原始地址)
    addr: String,
    /// 写队列
    writer: mpsc::Sender<WriterCommand>,
    /// opaque → 响应等待者
    pending: Arc<DashMap<i32, oneshot::Sender<RemotingCommand>>>,
    /// 通道是否存活(读循环退出即失效)
    active: Arc<AtomicBool>,
    /// opaque 生成器
    opaque: AtomicI32,
    /// 单请求超时
    request_timeout: Duration,
    /// SSH 隧道守卫(启用隧道时随通道存活,Drop 时关闭)
    _tunnel: Option<TunnelGuard>,
}

impl RemotingChannel {
    /// 建立连接(含 SSH 隧道展开)并启动读写任务
    async fn connect(addr: &str, params: &RocketmqParams) -> Result<Self, RocketmqError> {
        let (host, port) = split_addr(addr)?;
        let target = resolve_connection_target(&host, port, params.ssh_tunnel.as_ref())
            .await
            .map_err(|error| {
                RocketmqError::Connection(format!("RocketMQ 连接 {addr} 隧道建立失败: {error}"))
            })?;

        let connect_timeout = Duration::from_secs(params.connect_timeout.max(1));
        let stream = tokio::time::timeout(
            connect_timeout,
            TcpStream::connect((target.host.as_str(), target.port)),
        )
        .await
        .map_err(|_| {
            RocketmqError::Timeout(format!(
                "RocketMQ 连接 {addr} 超时({}s)",
                params.connect_timeout
            ))
        })?
        .map_err(|error| {
            RocketmqError::Connection(format!("RocketMQ 连接 {addr} 失败: {error}"))
        })?;
        // 禁用 Nagle,降低小指令延迟
        let _ = stream.set_nodelay(true);

        let (read_half, write_half) = stream.into_split();
        let (writer_tx, writer_rx) = mpsc::channel(WRITER_CHANNEL_CAPACITY);
        let pending: Arc<DashMap<i32, oneshot::Sender<RemotingCommand>>> = Arc::new(DashMap::new());
        let active = Arc::new(AtomicBool::new(true));

        // 写任务:顺序写出帧
        tokio::spawn(writer_loop(
            write_half,
            writer_rx,
            active.clone(),
            addr.to_string(),
        ));
        // 读任务:解析帧并按 opaque 分发响应
        tokio::spawn(reader_loop(
            read_half,
            pending.clone(),
            active.clone(),
            addr.to_string(),
        ));

        Ok(Self {
            addr: addr.to_string(),
            writer: writer_tx,
            pending,
            active,
            opaque: AtomicI32::new(1),
            request_timeout: Duration::from_millis(params.request_timeout.max(100)),
            _tunnel: target.tunnel,
        })
    }

    /// 通道是否可用
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst) && !self.writer.is_closed()
    }

    /// 关闭通道:通知写任务优雅断开连接,读循环随之退出
    pub fn shutdown(&self) {
        self.active.store(false, Ordering::SeqCst);
        self.pending.clear();
        let _ = self.writer.try_send(WriterCommand::Close);
    }

    /// 发起调用:签名 → 注册等待者 → 编码发送 → 限时等待响应
    async fn invoke(
        &self,
        command: &mut RemotingCommand,
        acl: &Option<AclCredentials>,
    ) -> Result<RemotingCommand, RocketmqError> {
        if !self.is_active() {
            return Err(RocketmqError::Connection(format!(
                "RocketMQ 通道已断开: {}",
                self.addr
            )));
        }
        command.opaque = self.opaque.fetch_add(1, Ordering::Relaxed);
        if let Some(credentials) = acl {
            credentials.sign_request(command);
        }
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.insert(command.opaque, response_tx);

        // 发送失败:清理等待者并标记断连
        if let Err(error) = self
            .writer
            .send(WriterCommand::Frame(command.encode_frame()?))
            .await
        {
            self.pending.remove(&command.opaque);
            self.active.store(false, Ordering::SeqCst);
            return Err(RocketmqError::Connection(format!(
                "RocketMQ 发送失败(通道 {}): {error}",
                self.addr
            )));
        }

        // 限时等待响应;超时清理等待者
        match tokio::time::timeout(self.request_timeout, response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(RocketmqError::Connection(format!(
                "RocketMQ 通道断开, 请求无响应(通道 {})",
                self.addr
            ))),
            Err(_) => {
                self.pending.remove(&command.opaque);
                Err(RocketmqError::Timeout(format!(
                    "RocketMQ 请求超时(code={}, opaque={}, 通道 {})",
                    command.code, command.opaque, self.addr
                )))
            }
        }
    }
}

/// 写任务:顺序发送编码帧;收到 Close 命令时优雅关闭连接
async fn writer_loop(
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    mut writer_rx: mpsc::Receiver<WriterCommand>,
    active: Arc<AtomicBool>,
    addr: String,
) {
    while let Some(command) = writer_rx.recv().await {
        match command {
            WriterCommand::Frame(frame) => {
                if let Err(error) = write_half.write_all(&frame).await {
                    debug!("RocketMQ 通道 {addr} 写入失败: {error}");
                    break;
                }
            }
            WriterCommand::Close => {
                debug!("RocketMQ 通道 {addr} 主动关闭");
                let _ = write_half.shutdown().await;
                break;
            }
        }
    }
    active.store(false, Ordering::SeqCst);
}

/// 读任务:循环解析帧并按 opaque 分发
async fn reader_loop(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    pending: Arc<DashMap<i32, oneshot::Sender<RemotingCommand>>>,
    active: Arc<AtomicBool>,
    addr: String,
) {
    loop {
        // 4B 总长度
        let mut length_bytes = [0u8; 4];
        if let Err(error) = read_half.read_exact(&mut length_bytes).await {
            if error.kind() != std::io::ErrorKind::UnexpectedEof {
                debug!("RocketMQ 通道 {addr} 读取失败: {error}");
            }
            break;
        }
        let total_len = u32::from_be_bytes(length_bytes);
        let body_len = match frame_body_length(total_len) {
            Ok(len) => len,
            Err(error) => {
                warn!("RocketMQ 通道 {addr} 收到非法帧: {error}");
                break;
            }
        };
        // 帧体 = 头部描述 + 头部 + body
        let mut frame_body = vec![0u8; body_len];
        if let Err(error) = read_half.read_exact(&mut frame_body).await {
            debug!("RocketMQ 通道 {addr} 读取帧体失败: {error}");
            break;
        }
        match decode_frame(&frame_body) {
            Ok(command) => {
                if let Some((_, sender)) = pending.remove(&command.opaque) {
                    // 接收端已放弃(超时)时忽略发送失败
                    let _ = sender.send(command);
                } else {
                    debug!(
                        "RocketMQ 通道 {addr} 收到未匹配响应: opaque={}",
                        command.opaque
                    );
                }
            }
            Err(error) => {
                warn!("RocketMQ 通道 {addr} 帧解析失败: {error}");
                break;
            }
        }
    }
    active.store(false, Ordering::SeqCst);
    // 通道断开:清理等待者使其收到 RecvError,等待者据此判定断连
    pending.clear();
}

/// 拆分 `host:port`(兼容 `[IPv6]:port` 字面量)
fn split_addr(addr: &str) -> Result<(String, u16), RocketmqError> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| RocketmqError::Config(format!("地址缺少端口: {addr}")))?;
    let port: u16 = port
        .parse()
        .map_err(|_| RocketmqError::Config(format!("地址端口非法: {addr}")))?;
    Ok((
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_string(),
        port,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::RequestCode;
    use std::collections::HashMap;

    type Responder =
        Box<dyn Fn(RemotingCommand) -> (i32, HashMap<String, String>, Vec<u8>) + Send + Sync>;

    /// 假服务端:读取一帧,按闭包响应(opaque 回填,flag 置响应位)
    async fn spawn_fake_server(
        respond: Responder,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let read = match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                buffer.extend_from_slice(&chunk[..read]);
                // 尽力解析完整帧并响应
                while buffer.len() >= 4 {
                    let total =
                        u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
                    if buffer.len() < 4 + total {
                        break;
                    }
                    let frame: Vec<u8> = buffer.drain(..4 + total).collect();
                    let request = decode_frame(&frame[4..]).unwrap();
                    let (code, ext, body) = respond(request.clone());
                    let mut response = RemotingCommand::create_request(code, ext);
                    response.opaque = request.opaque;
                    response.flag = 1; // RPC_TYPE 响应位
                    let frame = response.with_body(body).encode_frame().unwrap();
                    if socket.write_all(&frame).await.is_err() {
                        return;
                    }
                }
            }
        });
        (addr, handle)
    }

    fn test_params(addr: &str) -> Arc<RocketmqParams> {
        let mut params = RocketmqParams::default();
        params.namesrv_addrs = vec![addr.to_string()];
        params.request_timeout = 800;
        params.connect_timeout = 2;
        Arc::new(params)
    }

    /// 单请求-响应:opaque 配对与 ext 透传
    #[tokio::test]
    async fn namesrv_request_response_pairing() {
        let (addr, server) = spawn_fake_server(Box::new(|request| {
            assert_eq!(request.code, RequestCode::GET_BROKER_CLUSTER_INFO);
            let mut ext = HashMap::new();
            ext.insert("echo".to_string(), "ok".to_string());
            (
                0,
                ext,
                br#"{"brokerAddrTable":{},"clusterAddrTable":{}}"#.to_vec(),
            )
        }))
        .await;
        let client = RemotingClient::new(test_params(&addr.to_string())).unwrap();
        let request =
            RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, HashMap::new());
        let response = client.invoke_namesrv(request).await.expect("调用失败");
        assert_eq!(response.code, 0);
        assert_eq!(response.ext_fields["echo"], "ok");
        client.close().await;
        server.abort();
    }

    /// 并发多请求:乱序响应仍按 opaque 正确配对
    #[tokio::test]
    async fn concurrent_requests_pair_by_opaque() {
        let (addr, server) = spawn_fake_server(Box::new(|request| {
            // 回显请求 marker,并发天然乱序,校验配对正确性
            let mut ext = HashMap::new();
            let marker = request
                .ext_fields
                .get("marker")
                .cloned()
                .unwrap_or_default();
            ext.insert("marker".to_string(), marker);
            (0, ext, Vec::new())
        }))
        .await;
        let client = Arc::new(RemotingClient::new(test_params(&addr.to_string())).unwrap());
        let mut handles = Vec::new();
        for index in 0..4 {
            let client = client.clone();
            handles.push(tokio::spawn(async move {
                let mut ext = HashMap::new();
                ext.insert("marker".to_string(), format!("m{index}"));
                let request =
                    RemotingCommand::create_request(RequestCode::GET_ROUTEINFO_BY_TOPIC, ext);
                let response = client.invoke_namesrv(request).await.unwrap();
                response.ext_fields["marker"].clone()
            }));
        }
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }
        results.sort();
        assert_eq!(results, ["m0", "m1", "m2", "m3"]);
        client.close().await;
        server.abort();
    }

    /// 服务端不响应 → 请求超时
    #[tokio::test]
    async fn request_times_out_when_server_silent() {
        // 独立假服务端:读取后保持沉默
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            loop {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
        let client = RemotingClient::new(test_params(&addr.to_string())).unwrap();
        let request =
            RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, HashMap::new());
        let started = std::time::Instant::now();
        let error = client.invoke_namesrv(request).await.unwrap_err();
        assert!(
            matches!(error, RocketmqError::Timeout(_)),
            "实际: {error:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(3), "应按配置超时");
        client.close().await;
        server.abort();
    }

    /// 服务端返回错误码 → ensure_success 映射协议错误
    #[tokio::test]
    async fn server_error_maps_to_protocol_error() {
        let (addr, server) =
            spawn_fake_server(Box::new(|_| (17, HashMap::new(), Vec::new()))).await; // TOPIC_NOT_EXIST
        let client = RemotingClient::new(test_params(&addr.to_string())).unwrap();
        let request =
            RemotingCommand::create_request(RequestCode::GET_ROUTEINFO_BY_TOPIC, HashMap::new());
        let error = client.invoke_namesrv(request).await.unwrap_err();
        assert!(error.to_string().contains("code=17"), "实际: {error}");
        client.close().await;
        server.abort();
    }

    /// broker 直连调用
    #[tokio::test]
    async fn broker_invoke_works() {
        let (addr, server) = spawn_fake_server(Box::new(|request| {
            assert_eq!(request.code, RequestCode::GET_TOPIC_STATS_INFO);
            (0, HashMap::new(), br#"{"offsetTable":{}}"#.to_vec())
        }))
        .await;
        let client = RemotingClient::new(test_params("127.0.0.1:1")).unwrap();
        let request =
            RemotingCommand::create_request(RequestCode::GET_TOPIC_STATS_INFO, HashMap::new());
        let response = client
            .invoke_broker(&addr.to_string(), request)
            .await
            .expect("broker 调用失败");
        assert_eq!(response.code, 0);
        client.close().await;
        server.abort();
    }

    /// 连接失败 → 连接错误(端口未监听)
    #[tokio::test]
    async fn connect_failure_maps_to_connection_error() {
        // 占用一个空闲端口后立即关闭,保证连接被拒绝
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let client = RemotingClient::new(test_params(&addr.to_string())).unwrap();
        let request =
            RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, HashMap::new());
        let error = client.invoke_namesrv(request).await.unwrap_err();
        assert!(
            matches!(error, RocketmqError::Connection(_)),
            "实际: {error:?}"
        );
        client.close().await;
    }

    /// 地址拆分与规范化断言
    #[test]
    fn split_addr_variants() {
        let (host, port) = split_addr("10.0.0.2:10911").unwrap();
        assert_eq!((host.as_str(), port), ("10.0.0.2", 10911));
        let (host, port) = split_addr("[::1]:9876").unwrap();
        assert_eq!((host.as_str(), port), ("::1", 9876));
        assert!(split_addr("no-port").is_err());
        assert!(split_addr("host:bad").is_err());
    }
}
