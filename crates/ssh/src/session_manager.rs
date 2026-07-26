use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{RusshChannel, RusshClient, ShellIntegrationSetup, SshClient, SshConnectConfig};

/// 缓存命中时探活节流窗口：距离上次成功 ping 小于该值跳过 ping。
const PING_THROTTLE: Duration = Duration::from_secs(5);

#[async_trait]
trait SharedSessionClient: Send + Sync {
    fn is_connected(&self) -> bool;
    async fn ping(&self) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
}

#[async_trait]
trait SharedSessionConnector<C>: Send + Sync {
    async fn connect(&self, config: SshConnectConfig) -> Result<C>;
}

struct SessionState<C> {
    client: Option<Arc<Mutex<C>>>,
    /// 与 `client` 同生命周期的 shell integration 结果缓存。`invalidate`/`disconnect` 会一并清空。
    shell_integration: Option<ShellIntegrationSetup>,
    /// 正在进行的 connect 协程会登记一个 sticky completion token，其他等待者订阅它以避免并发
    /// connect。CancellationToken 的完成信号不会像一次性 Notify wakeup 那样丢失。
    connecting: Option<Arc<ConnectionFlight>>,
    /// 最后一次 ping 探活成功时间。节流用，避免 terminal 每次 write 都触发 ping。
    last_ping: Option<Instant>,
}

impl<C> Default for SessionState<C> {
    fn default() -> Self {
        Self {
            client: None,
            shell_integration: None,
            connecting: None,
            last_ping: None,
        }
    }
}

struct ConnectionFlight {
    completion: CancellationToken,
    abandoned: AtomicBool,
}

impl ConnectionFlight {
    fn new() -> Self {
        Self {
            completion: CancellationToken::new(),
            abandoned: AtomicBool::new(false),
        }
    }

    fn complete(&self) {
        self.completion.cancel();
    }

    fn abandon(&self) {
        self.abandoned.store(true, Ordering::Release);
        self.complete();
    }

    fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::Acquire)
    }
}

/// Ensures a cancelled or panicking connection owner cannot strand the
/// single-flight slot.  State cleanup is performed by the next waiter (or
/// shutdown) because `Drop` cannot acquire the asynchronous state mutex.
struct ConnectionFlightGuard {
    flight: Arc<ConnectionFlight>,
    armed: bool,
}

impl ConnectionFlightGuard {
    fn new(flight: Arc<ConnectionFlight>) -> Self {
        Self {
            flight,
            armed: true,
        }
    }

    fn complete(&mut self) {
        self.flight.complete();
        self.armed = false;
    }
}

impl Drop for ConnectionFlightGuard {
    fn drop(&mut self) {
        if self.armed {
            self.flight.abandon();
        }
    }
}

struct SessionPool<C, K> {
    config: RwLock<SessionConfig>,
    connector: K,
    state: Mutex<SessionState<C>>,
    shutdown: CancellationToken,
    /// Serializes successful client checkout/publication with the synchronous
    /// shutdown gate.  Once `request_shutdown()` returns, no connection can
    /// subsequently be published or checked out successfully.
    shutdown_publication: RwLock<()>,
}

struct SessionConfig {
    value: SshConnectConfig,
    generation: u64,
}

impl<C, K> SessionPool<C, K>
where
    C: SharedSessionClient + 'static,
    K: SharedSessionConnector<C> + 'static,
{
    fn new(config: SshConnectConfig, connector: K) -> Self {
        Self {
            config: RwLock::new(SessionConfig {
                value: config,
                generation: 0,
            }),
            connector,
            state: Mutex::new(SessionState::default()),
            shutdown: CancellationToken::new(),
            shutdown_publication: RwLock::new(()),
        }
    }

    fn config(&self) -> SshConnectConfig {
        self.config_snapshot().0
    }

    fn replace_config(&self, config: SshConnectConfig) {
        let mut current = self.config.write().unwrap_or_else(|err| err.into_inner());
        current.value = config;
        current.generation = current.generation.wrapping_add(1);
    }

    fn config_snapshot(&self) -> (SshConnectConfig, u64) {
        let current = self.config.read().unwrap_or_else(|err| err.into_inner());
        (current.value.clone(), current.generation)
    }

    fn is_current_config_generation(&self, generation: u64) -> bool {
        self.config
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .generation
            == generation
    }

    /// 获取一个"活着"的 client。命中缓存会做节流 ping 验真；失败或未命中走 connect 分支，
    /// 在 connect 期间**不持有 state 锁**，并通过 sticky completion signal 让并发等待者
    /// 共享同一次连接。
    async fn client(&self) -> Result<Arc<Mutex<C>>> {
        loop {
            if self.shutdown.is_cancelled() {
                return Err(anyhow!("SSH session manager is shut down"));
            }

            // Phase 1: 检查缓存与并发连接状态，尽快释放锁。
            let outcome = {
                let mut state = self.state.lock().await;

                if self.shutdown.is_cancelled() {
                    return Err(anyhow!("SSH session manager is shut down"));
                }

                if let Some(client) = state.client.clone() {
                    // client 本身的 is_connected 是本地判定，ping 才是真实探活。
                    let recently_pinged = state
                        .last_ping
                        .map(|t| t.elapsed() < PING_THROTTLE)
                        .unwrap_or(false);
                    Phase1::Inspect {
                        client,
                        recently_pinged,
                    }
                } else if let Some(completion) = state.connecting.clone() {
                    Phase1::Wait(completion)
                } else {
                    let completion = Arc::new(ConnectionFlight::new());
                    state.connecting = Some(completion.clone());
                    let (config, generation) = self.config_snapshot();
                    Phase1::Connect {
                        completion,
                        config: Box::new(config),
                        generation,
                    }
                }
            };

            match outcome {
                Phase1::Inspect {
                    client,
                    recently_pinged,
                } => {
                    // 本地状态检查便宜，先做。
                    let connected = client.lock().await.is_connected();
                    if !connected {
                        self.clear_dead_client(&client).await;
                        continue;
                    }

                    if recently_pinged {
                        if self.can_checkout(&client).await {
                            return Ok(client);
                        }
                        continue;
                    }

                    // ping 带 3s 超时（RusshClient::ping 内实现），不持 state 锁。
                    let ping_ok = client.lock().await.ping().await.is_ok();
                    if ping_ok {
                        if self.refresh_ping_and_checkout(&client).await {
                            return Ok(client);
                        }
                        continue;
                    }

                    self.clear_dead_client(&client).await;
                    // 落到下轮循环重连。
                }
                Phase1::Wait(completion) => {
                    tokio::select! {
                        biased;
                        _ = self.shutdown.cancelled() => {
                            return Err(anyhow!("SSH session manager is shut down"));
                        }
                        _ = completion.completion.cancelled() => {
                            if completion.is_abandoned() {
                                self.clear_connection_flight(&completion).await;
                            }
                        }
                    }
                }
                Phase1::Connect {
                    completion,
                    config,
                    generation,
                } => {
                    let mut flight_guard = ConnectionFlightGuard::new(completion.clone());
                    // connect 期间不持 state 锁，别的调用者可以继续 inspect/wait。
                    let result = tokio::select! {
                        biased;
                        _ = self.shutdown.cancelled() => {
                            self.clear_connection_flight(&completion).await;
                            flight_guard.complete();
                            return Err(anyhow!("SSH session manager is shut down"));
                        }
                        result = self.connector.connect(*config) => result,
                    };
                    let mut state = self.state.lock().await;
                    let publication = self
                        .shutdown_publication
                        .read()
                        .unwrap_or_else(|error| error.into_inner());
                    if self.shutdown.is_cancelled() {
                        state.client = None;
                        state.shell_integration = None;
                        state.last_ping = None;
                        drop(publication);
                        drop(state);
                        if let Ok(mut client) = result {
                            let _ = client.disconnect().await;
                        }
                        self.clear_connection_flight(&completion).await;
                        flight_guard.complete();
                        return Err(anyhow!("SSH session manager is shut down"));
                    }
                    if !self.is_current_config_generation(generation) {
                        drop(publication);
                        drop(state);
                        if let Ok(mut stale_client) = result {
                            let _ = stale_client.disconnect().await;
                        }
                        self.clear_connection_flight(&completion).await;
                        flight_guard.complete();
                        continue;
                    }
                    match result {
                        Ok(new_client) => {
                            let arc = Arc::new(Mutex::new(new_client));
                            state.connecting = None;
                            state.client = Some(arc.clone());
                            state.shell_integration = None;
                            state.last_ping = Some(Instant::now());
                            flight_guard.complete();
                            return Ok(arc);
                        }
                        Err(err) => {
                            // 只清 connecting，等待者重跑一轮循环继续尝试（会再次走 Connect 分支）。
                            state.connecting = None;
                            flight_guard.complete();
                            return Err(err);
                        }
                    }
                }
            }
        }
    }

    async fn clear_connection_flight(&self, expected: &Arc<ConnectionFlight>) {
        let mut state = self.state.lock().await;
        if state
            .connecting
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            state.connecting = None;
        }
    }

    /// Validate a cached checkout while holding the read side of the
    /// publication gate.  A concurrent shutdown therefore linearizes either
    /// before this successful checkout or after it, never in the middle.
    async fn can_checkout(&self, candidate: &Arc<Mutex<C>>) -> bool {
        let state = self.state.lock().await;
        let publication = self
            .shutdown_publication
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let can_checkout = !self.shutdown.is_cancelled()
            && state
                .client
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, candidate));
        drop(publication);
        can_checkout
    }

    async fn refresh_ping_and_checkout(&self, candidate: &Arc<Mutex<C>>) -> bool {
        let mut state = self.state.lock().await;
        let publication = self
            .shutdown_publication
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let can_checkout = !self.shutdown.is_cancelled()
            && state
                .client
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, candidate));
        if can_checkout {
            state.last_ping = Some(Instant::now());
        }
        drop(publication);
        can_checkout
    }

    /// 如果死 client 仍挂在 state 上，清掉它（同时丢弃配套的 integration 缓存 / 探活时间）。
    async fn clear_dead_client(&self, dead: &Arc<Mutex<C>>) {
        let mut state = self.state.lock().await;
        if let Some(current) = &state.client {
            if Arc::ptr_eq(current, dead) {
                state.client = None;
                state.shell_integration = None;
                state.last_ping = None;
            }
        }
    }

    async fn invalidate(&self) {
        let mut state = self.state.lock().await;
        state.client = None;
        state.shell_integration = None;
        state.last_ping = None;
    }

    async fn disconnect(&self) -> Result<()> {
        let client = {
            let mut state = self.state.lock().await;
            state.shell_integration = None;
            state.last_ping = None;
            state.client.take()
        };

        if let Some(client) = client {
            client.lock().await.disconnect().await?;
        }
        Ok(())
    }

    /// Close the reconnect gate synchronously.
    ///
    /// The application service calls this for every published manager before
    /// it starts awaiting transport cleanup, so a retained lease cannot race
    /// the shutdown driver and create a new client.
    fn request_shutdown(&self) {
        let _publication = self
            .shutdown_publication
            .write()
            .unwrap_or_else(|error| error.into_inner());
        self.shutdown.cancel();
    }

    /// Permanently close this manager and wait for both its cached client and
    /// any connection already in flight to finish disconnecting.
    ///
    /// Unlike [`Self::disconnect`], this is terminal: all later calls to
    /// [`Self::client`] fail without invoking the connector.
    async fn shutdown(&self) -> Result<()> {
        self.request_shutdown();

        let (client, connecting) = {
            let mut state = self.state.lock().await;
            state.shell_integration = None;
            state.last_ping = None;
            let connecting = state.connecting.clone();
            (state.client.take(), connecting)
        };

        let disconnect_result = if let Some(client) = client {
            client.lock().await.disconnect().await
        } else {
            Ok(())
        };

        // A connect that observed the pre-shutdown state checks the
        // cancellation/publication gate before publication, disconnects its
        // result when necessary, then completes this sticky signal.
        if let Some(connecting) = connecting {
            connecting.completion.cancelled().await;
            self.clear_connection_flight(&connecting).await;
        }

        disconnect_result
    }

    async fn cached_shell_integration(&self) -> Option<ShellIntegrationSetup> {
        self.state.lock().await.shell_integration.clone()
    }

    /// 缓存一次 shell integration 安装结果。只有 `for_client` 与当前 state 中的 client 指向同一
    /// session 时才生效，防止我们把老 session 的 session_dir 绑到重建后的新 session 上。
    async fn set_shell_integration(
        &self,
        for_client: &Arc<Mutex<C>>,
        setup: ShellIntegrationSetup,
    ) {
        let mut state = self.state.lock().await;
        if let Some(current) = &state.client {
            if Arc::ptr_eq(current, for_client) {
                state.shell_integration = Some(setup);
            }
        }
    }
}

enum Phase1<C> {
    Inspect {
        client: Arc<Mutex<C>>,
        recently_pinged: bool,
    },
    Wait(Arc<ConnectionFlight>),
    Connect {
        completion: Arc<ConnectionFlight>,
        config: Box<SshConnectConfig>,
        generation: u64,
    },
}

#[derive(Clone, Copy, Default)]
struct RusshClientConnector;

#[async_trait]
impl SharedSessionClient for RusshClient {
    fn is_connected(&self) -> bool {
        SshClient::is_connected(self)
    }

    async fn ping(&self) -> Result<()> {
        SshClient::ping(self).await
    }

    async fn disconnect(&mut self) -> Result<()> {
        SshClient::disconnect(self).await
    }
}

#[async_trait]
impl SharedSessionConnector<RusshClient> for RusshClientConnector {
    async fn connect(&self, config: SshConnectConfig) -> Result<RusshClient> {
        RusshClient::connect(config).await
    }
}

#[derive(Clone)]
pub struct SshSessionManager {
    inner: Arc<SessionPool<RusshClient, RusshClientConnector>>,
}

impl SshSessionManager {
    pub fn new(config: SshConnectConfig) -> Self {
        Self {
            inner: Arc::new(SessionPool::new(config, RusshClientConnector)),
        }
    }

    pub fn config(&self) -> SshConnectConfig {
        self.inner.config()
    }

    /// 替换后续连接使用的配置，同时保留 manager 的共享 Arc 身份。
    ///
    /// Terminal、SFTP 与监控面板会继续共享同一个 manager；调用方应在重连前断开旧会话。
    pub fn replace_config(&self, config: SshConnectConfig) {
        self.inner.replace_config(config);
    }

    pub async fn client(&self) -> Result<Arc<Mutex<RusshClient>>> {
        self.inner.client().await
    }

    pub async fn open_channel(&self) -> Result<RusshChannel> {
        let client = self.client().await?;
        let mut guard = client.lock().await;
        guard.open_channel().await
    }

    pub async fn open_raw_channel(&self) -> Result<russh::Channel<russh::client::Msg>> {
        let client = self.client().await?;
        let mut guard = client.lock().await;
        guard.open_raw_channel().await
    }

    pub async fn invalidate(&self) {
        self.inner.invalidate().await;
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.inner.disconnect().await
    }

    /// Permanently stop this manager.
    ///
    /// This is reserved for application/service teardown.  Ordinary health
    /// invalidation and idle eviction use [`Self::disconnect`] so a later
    /// checkout may reconnect.
    pub async fn shutdown(&self) -> Result<()> {
        self.inner.shutdown().await
    }

    pub(crate) fn request_shutdown(&self) {
        self.inner.request_shutdown();
    }

    /// 读取已缓存的 shell integration 结果；缓存会跟随当前 session 生命周期一起失效。
    pub async fn cached_shell_integration(&self) -> Option<ShellIntegrationSetup> {
        self.inner.cached_shell_integration().await
    }

    /// 在当前 session 上记录 shell integration 结果。`for_client` 必须是 `client()` 返回的 Arc，
    /// 否则写入会被静默丢弃（防止串到已重建的新 session）。
    pub async fn set_shell_integration(
        &self,
        for_client: &Arc<Mutex<RusshClient>>,
        setup: ShellIntegrationSetup,
    ) {
        self.inner.set_shell_integration(for_client, setup).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{PING_THROTTLE, SessionPool, SharedSessionClient, SharedSessionConnector};
    use crate::{
        HostKeyVerifier, JumpServerConnectConfig, ProxyConnectConfig, ShellIntegrationSetup,
        SshAuth, SshConnectConfig,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::{Notify, Semaphore};
    use tokio::time::{Duration, sleep};

    #[derive(Default)]
    struct FakeConnector {
        connect_count: AtomicUsize,
        fail_first: AtomicBool,
        connected_hosts: StdMutex<Vec<String>>,
    }

    struct FakeClient {
        connected: AtomicBool,
        disconnect_count: Arc<AtomicUsize>,
        ping_count: Arc<AtomicUsize>,
        ping_fails: AtomicBool,
    }

    struct SlowFakeConnector {
        connect_count: AtomicUsize,
        connect_started: Notify,
        connected_hosts: StdMutex<Vec<String>>,
    }

    struct BlockingFakeConnector {
        connect_count: AtomicUsize,
        connect_started: Notify,
        connect_permit: Semaphore,
        disconnect_count: Arc<AtomicUsize>,
    }

    struct PanicOnceConnector {
        connect_count: AtomicUsize,
        first_connect_started: Semaphore,
        panic_permit: Semaphore,
    }

    struct BlockingDisconnectControls {
        connect_started: Semaphore,
        connect_permit: Semaphore,
        connect_returned: Semaphore,
        disconnect_started: Semaphore,
        disconnect_permit: Semaphore,
        disconnect_count: AtomicUsize,
    }

    struct BlockingDisconnectClient {
        connected: AtomicBool,
        controls: Arc<BlockingDisconnectControls>,
    }

    struct BlockingDisconnectConnector {
        controls: Arc<BlockingDisconnectControls>,
    }

    struct BlockingPingControls {
        ping_started: Semaphore,
        ping_permit: Semaphore,
        disconnect_count: AtomicUsize,
    }

    struct BlockingPingClient {
        connected: AtomicBool,
        controls: Arc<BlockingPingControls>,
    }

    struct BlockingPingConnector {
        connect_count: AtomicUsize,
        controls: Arc<BlockingPingControls>,
    }

    #[async_trait]
    impl SharedSessionClient for FakeClient {
        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        async fn ping(&self) -> Result<()> {
            self.ping_count.fetch_add(1, Ordering::SeqCst);
            if self.ping_fails.load(Ordering::SeqCst) {
                Err(anyhow!("fake ping failure"))
            } else {
                Ok(())
            }
        }

        async fn disconnect(&mut self) -> Result<()> {
            self.connected.store(false, Ordering::SeqCst);
            self.disconnect_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl SharedSessionConnector<FakeClient> for Arc<FakeConnector> {
        async fn connect(&self, config: SshConnectConfig) -> Result<FakeClient> {
            self.connected_hosts
                .lock()
                .expect("connected hosts lock should not be poisoned")
                .push(config.host);
            let n = self.connect_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 && self.fail_first.load(Ordering::SeqCst) {
                return Err(anyhow!("first connect fails"));
            }
            Ok(FakeClient {
                connected: AtomicBool::new(true),
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                ping_count: Arc::new(AtomicUsize::new(0)),
                ping_fails: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl SharedSessionConnector<FakeClient> for Arc<SlowFakeConnector> {
        async fn connect(&self, config: SshConnectConfig) -> Result<FakeClient> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
            self.connected_hosts
                .lock()
                .expect("connected hosts lock should not be poisoned")
                .push(config.host);
            self.connect_started.notify_waiters();
            sleep(Duration::from_millis(50)).await;
            Ok(FakeClient {
                connected: AtomicBool::new(true),
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                ping_count: Arc::new(AtomicUsize::new(0)),
                ping_fails: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl SharedSessionConnector<FakeClient> for Arc<BlockingFakeConnector> {
        async fn connect(&self, _config: SshConnectConfig) -> Result<FakeClient> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
            self.connect_started.notify_waiters();
            self.connect_permit
                .acquire()
                .await
                .expect("test connector semaphore should remain open")
                .forget();
            Ok(FakeClient {
                connected: AtomicBool::new(true),
                disconnect_count: self.disconnect_count.clone(),
                ping_count: Arc::new(AtomicUsize::new(0)),
                ping_fails: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl SharedSessionConnector<FakeClient> for Arc<PanicOnceConnector> {
        async fn connect(&self, _config: SshConnectConfig) -> Result<FakeClient> {
            let attempt = self.connect_count.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                self.first_connect_started.add_permits(1);
                self.panic_permit
                    .acquire()
                    .await
                    .expect("test panic semaphore should remain open")
                    .forget();
                panic!("intentional connector panic");
            }

            Ok(FakeClient {
                connected: AtomicBool::new(true),
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                ping_count: Arc::new(AtomicUsize::new(0)),
                ping_fails: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl SharedSessionClient for BlockingDisconnectClient {
        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        async fn ping(&self) -> Result<()> {
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<()> {
            self.controls.disconnect_started.add_permits(1);
            self.controls
                .disconnect_permit
                .acquire()
                .await
                .expect("test disconnect semaphore should remain open")
                .forget();
            self.connected.store(false, Ordering::SeqCst);
            self.controls
                .disconnect_count
                .fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl SharedSessionConnector<BlockingDisconnectClient> for Arc<BlockingDisconnectConnector> {
        async fn connect(&self, _config: SshConnectConfig) -> Result<BlockingDisconnectClient> {
            self.controls.connect_started.add_permits(1);
            self.controls
                .connect_permit
                .acquire()
                .await
                .expect("test connect semaphore should remain open")
                .forget();
            self.controls.connect_returned.add_permits(1);
            Ok(BlockingDisconnectClient {
                connected: AtomicBool::new(true),
                controls: self.controls.clone(),
            })
        }
    }

    #[async_trait]
    impl SharedSessionClient for BlockingPingClient {
        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        async fn ping(&self) -> Result<()> {
            self.controls.ping_started.add_permits(1);
            self.controls
                .ping_permit
                .acquire()
                .await
                .expect("test ping semaphore should remain open")
                .forget();
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<()> {
            self.connected.store(false, Ordering::SeqCst);
            self.controls
                .disconnect_count
                .fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl SharedSessionConnector<BlockingPingClient> for Arc<BlockingPingConnector> {
        async fn connect(&self, _config: SshConnectConfig) -> Result<BlockingPingClient> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
            Ok(BlockingPingClient {
                connected: AtomicBool::new(true),
                controls: self.controls.clone(),
            })
        }
    }

    fn test_config() -> SshConnectConfig {
        SshConnectConfig {
            host: "example.com".to_string(),
            port: 22,
            username: "tester".to_string(),
            auth: SshAuth::Agent,
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None::<JumpServerConnectConfig>,
            proxy: None::<ProxyConnectConfig>,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
        }
    }

    #[tokio::test]
    async fn reuses_single_connector_invocation_for_repeated_client_access() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        let second = pool.client().await.expect("第二次获取 client 应成功");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalidate_forces_next_client_access_to_reconnect() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        pool.invalidate().await;
        let second = pool.client().await.expect("失效后再次获取 client 应成功");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn disconnect_clears_cached_client() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        let disconnect_count = first.lock().await.disconnect_count.clone();

        pool.disconnect().await.expect("disconnect 应成功");
        let second = pool.client().await.expect("断开后再次获取 client 应成功");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(disconnect_count.load(Ordering::SeqCst), 1);
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn shutdown_permanently_rejects_new_clients_and_is_idempotent() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("first client should connect");
        let disconnect_count = first.lock().await.disconnect_count.clone();

        pool.shutdown().await.expect("shutdown should succeed");
        pool.shutdown()
            .await
            .expect("repeated shutdown should remain successful");

        let error = match pool.client().await {
            Ok(_) => panic!("shutdown manager must never reconnect"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "SSH session manager is shut down");
        assert_eq!(disconnect_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            connector.connect_count.load(Ordering::SeqCst),
            1,
            "shutdown must be distinct from a reconnectable disconnect"
        );
    }

    #[tokio::test]
    async fn shutdown_during_connect_cancels_the_unpublished_client() {
        let connector = Arc::new(BlockingFakeConnector {
            connect_count: AtomicUsize::new(0),
            connect_started: Notify::new(),
            connect_permit: Semaphore::new(0),
            disconnect_count: Arc::new(AtomicUsize::new(0)),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector.clone()));

        let started = connector.connect_started.notified();
        let connecting_pool = pool.clone();
        let connecting = tokio::spawn(async move { connecting_pool.client().await });
        started.await;

        pool.request_shutdown();
        pool.shutdown().await.expect("shutdown should finish");
        let error = match connecting.await.expect("connect task should not panic") {
            Ok(_) => panic!("the in-flight client must not be published"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "SSH session manager is shut down");
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            connector.disconnect_count.load(Ordering::SeqCst),
            0,
            "connector future was cancelled before it could construct a client"
        );
    }

    #[tokio::test]
    async fn connector_panic_does_not_strand_the_single_flight_slot() {
        let connector = Arc::new(PanicOnceConnector {
            connect_count: AtomicUsize::new(0),
            first_connect_started: Semaphore::new(0),
            panic_permit: Semaphore::new(0),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector.clone()));

        let first_pool = pool.clone();
        let first = tokio::spawn(async move { first_pool.client().await });
        connector
            .first_connect_started
            .acquire()
            .await
            .expect("test start semaphore should remain open")
            .forget();

        let waiting_pool = pool.clone();
        let waiting = tokio::spawn(async move { waiting_pool.client().await });
        tokio::task::yield_now().await;
        connector.panic_permit.add_permits(1);

        let first_error = match first.await {
            Ok(_) => panic!("the first connector task should panic"),
            Err(error) => error,
        };
        assert!(first_error.is_panic());
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("a waiter must recover the abandoned connection flight")
            .expect("the waiting task should not panic")
            .expect("the retry should connect successfully");
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn shutdown_waits_for_an_unpublished_client_to_disconnect() {
        let controls = Arc::new(BlockingDisconnectControls {
            connect_started: Semaphore::new(0),
            connect_permit: Semaphore::new(0),
            connect_returned: Semaphore::new(0),
            disconnect_started: Semaphore::new(0),
            disconnect_permit: Semaphore::new(0),
            disconnect_count: AtomicUsize::new(0),
        });
        let connector = Arc::new(BlockingDisconnectConnector {
            controls: controls.clone(),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector));

        let connecting_pool = pool.clone();
        let connecting = tokio::spawn(async move { connecting_pool.client().await });
        controls
            .connect_started
            .acquire()
            .await
            .expect("test start semaphore should remain open")
            .forget();

        // Hold the state lock so the connector can return but cannot yet
        // publish.  This makes shutdown race the post-connect cleanup path.
        let state = pool.state.lock().await;
        controls.connect_permit.add_permits(1);
        controls
            .connect_returned
            .acquire()
            .await
            .expect("test return semaphore should remain open")
            .forget();
        pool.request_shutdown();

        let shutdown_pool = pool.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_pool.shutdown().await });
        drop(state);
        controls
            .disconnect_started
            .acquire()
            .await
            .expect("test disconnect semaphore should remain open")
            .forget();

        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
                .await
                .is_err(),
            "shutdown must retain the connection flight until cleanup finishes"
        );
        controls.disconnect_permit.add_permits(1);

        shutdown
            .await
            .expect("shutdown task should not panic")
            .expect("shutdown should finish after cleanup");
        let error = match connecting.await.expect("connect task should not panic") {
            Ok(_) => panic!("the post-shutdown client must not be published"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "SSH session manager is shut down");
        assert_eq!(controls.disconnect_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn shutdown_during_cached_ping_prevents_a_late_checkout() {
        let controls = Arc::new(BlockingPingControls {
            ping_started: Semaphore::new(0),
            ping_permit: Semaphore::new(0),
            disconnect_count: AtomicUsize::new(0),
        });
        let connector = Arc::new(BlockingPingConnector {
            connect_count: AtomicUsize::new(0),
            controls: controls.clone(),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector.clone()));

        pool.client()
            .await
            .expect("initial client should be published");
        pool.state.lock().await.last_ping = None;

        let inspecting_pool = pool.clone();
        let inspecting = tokio::spawn(async move { inspecting_pool.client().await });
        controls
            .ping_started
            .acquire()
            .await
            .expect("test ping semaphore should remain open")
            .forget();

        pool.request_shutdown();
        controls.ping_permit.add_permits(1);

        let error = match inspecting.await.expect("client task should not panic") {
            Ok(_) => panic!("a checkout that finishes after shutdown must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "SSH session manager is shut down");

        pool.shutdown().await.expect("shutdown should finish");
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 1);
        assert_eq!(controls.disconnect_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replace_config_is_used_by_next_connection() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());
        let _first = pool.client().await.expect("首次连接应成功");
        let mut latest = test_config();
        latest.host = "latest.example".to_string();
        latest.port = 2222;

        pool.replace_config(latest.clone());
        pool.disconnect().await.expect("旧连接应断开");
        let _second = pool.client().await.expect("更新配置后连接应成功");

        assert_eq!("latest.example", pool.config().host);
        assert_eq!(2222, pool.config().port);
        assert_eq!(
            vec!["example.com".to_string(), "latest.example".to_string()],
            *connector
                .connected_hosts
                .lock()
                .expect("connected hosts lock should not be poisoned")
        );
    }

    #[tokio::test]
    async fn coalesces_concurrent_client_connects() {
        let connector = Arc::new(SlowFakeConnector {
            connect_count: AtomicUsize::new(0),
            connect_started: Notify::new(),
            connected_hosts: StdMutex::new(Vec::new()),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector.clone()));

        let first_pool = pool.clone();
        let first =
            tokio::spawn(async move { first_pool.client().await.expect("第一次并发应成功") });

        connector.connect_started.notified().await;

        let second = pool.client().await.expect("第二次并发应成功");
        let first = first.await.expect("第一次任务应成功");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn config_replaced_during_connect_discards_stale_client() {
        let connector = Arc::new(SlowFakeConnector {
            connect_count: AtomicUsize::new(0),
            connect_started: Notify::new(),
            connected_hosts: StdMutex::new(Vec::new()),
        });
        let pool = Arc::new(SessionPool::new(test_config(), connector.clone()));
        let connecting_pool = pool.clone();
        let connection =
            tokio::spawn(async move { connecting_pool.client().await.expect("重连应成功") });

        connector.connect_started.notified().await;
        let mut latest = test_config();
        latest.host = "latest.example".to_string();
        pool.replace_config(latest);
        let _client = connection.await.expect("连接任务不应 panic");

        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            vec!["example.com".to_string(), "latest.example".to_string()],
            *connector
                .connected_hosts
                .lock()
                .expect("connected hosts lock should not be poisoned")
        );
    }

    #[tokio::test]
    async fn ping_failure_forces_reconnect_on_next_client_call() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("首次连接应成功");
        // 让下次 ping 失败，同时超出节流窗口。
        first.lock().await.ping_fails.store(true, Ordering::SeqCst);
        tokio::time::sleep(PING_THROTTLE + Duration::from_millis(20)).await;

        let second = pool.client().await.expect("ping 失败后应自动重连");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn ping_throttled_within_window() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("首次连接应成功");
        let ping_count = first.lock().await.ping_count.clone();
        // 连着再取 5 次，节流窗口内不应发新的 ping。
        for _ in 0..5 {
            let _ = pool.client().await.expect("命中缓存应成功");
        }

        assert_eq!(ping_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn shell_integration_cache_invalidates_with_client() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("首次连接应成功");
        pool.set_shell_integration(
            &first,
            ShellIntegrationSetup {
                home_dir: "/tmp/home".into(),
                session_dir: "/tmp/home/.config/onetcli/sessions/1".into(),
                login_shell: Some("/bin/zsh".into()),
            },
        )
        .await;
        assert!(pool.cached_shell_integration().await.is_some());

        pool.invalidate().await;
        assert!(
            pool.cached_shell_integration().await.is_none(),
            "invalidate 后 integration 缓存必须清空"
        );
    }

    #[tokio::test]
    async fn stale_integration_write_is_dropped_after_reconnect() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let old = pool.client().await.expect("首次连接");
        pool.invalidate().await;
        let _new = pool.client().await.expect("重连");

        // 拿旧 Arc 写缓存，不应该被接受（防止串到新 session）。
        pool.set_shell_integration(
            &old,
            ShellIntegrationSetup {
                home_dir: "/stale".into(),
                session_dir: "/stale".into(),
                login_shell: None,
            },
        )
        .await;
        assert!(pool.cached_shell_integration().await.is_none());
    }
}
