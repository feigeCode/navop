use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{Mutex, Notify};

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
    /// 正在进行的 connect 协程会登记一个 Notify，其他等待者订阅它以避免并发 connect。
    connecting: Option<Arc<Notify>>,
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

struct SessionPool<C, K> {
    config: SshConnectConfig,
    connector: K,
    state: Mutex<SessionState<C>>,
}

impl<C, K> SessionPool<C, K>
where
    C: SharedSessionClient + 'static,
    K: SharedSessionConnector<C> + 'static,
{
    fn new(config: SshConnectConfig, connector: K) -> Self {
        Self {
            config,
            connector,
            state: Mutex::new(SessionState::default()),
        }
    }

    /// 获取一个"活着"的 client。命中缓存会做节流 ping 验真；失败或未命中走 connect 分支，
    /// 在 connect 期间**不持有 state 锁**，并通过 Notify 让并发等待者共享同一次连接。
    async fn client(&self) -> Result<Arc<Mutex<C>>> {
        loop {
            // Phase 1: 检查缓存与并发连接状态，尽快释放锁。
            let outcome = {
                let mut state = self.state.lock().await;

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
                } else if let Some(notify) = state.connecting.clone() {
                    Phase1::Wait(notify)
                } else {
                    let notify = Arc::new(Notify::new());
                    state.connecting = Some(notify.clone());
                    Phase1::Connect(notify)
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
                        return Ok(client);
                    }

                    // ping 带 3s 超时（RusshClient::ping 内实现），不持 state 锁。
                    let ping_ok = client.lock().await.ping().await.is_ok();
                    if ping_ok {
                        let mut state = self.state.lock().await;
                        if let Some(current) = &state.client {
                            if Arc::ptr_eq(current, &client) {
                                state.last_ping = Some(Instant::now());
                            }
                        }
                        return Ok(client);
                    }

                    self.clear_dead_client(&client).await;
                    // 落到下轮循环重连。
                }
                Phase1::Wait(notify) => {
                    notify.notified().await;
                }
                Phase1::Connect(notify) => {
                    // connect 期间不持 state 锁，别的调用者可以继续 inspect/wait。
                    let result = self.connector.connect(self.config.clone()).await;
                    let mut state = self.state.lock().await;
                    state.connecting = None;
                    match result {
                        Ok(new_client) => {
                            let arc = Arc::new(Mutex::new(new_client));
                            state.client = Some(arc.clone());
                            state.shell_integration = None;
                            state.last_ping = Some(Instant::now());
                            notify.notify_waiters();
                            return Ok(arc);
                        }
                        Err(err) => {
                            // 只清 connecting，等待者重跑一轮循环继续尝试（会再次走 Connect 分支）。
                            notify.notify_waiters();
                            return Err(err);
                        }
                    }
                }
            }
        }
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
    Wait(Arc<Notify>),
    Connect(Arc<Notify>),
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
        self.inner.config.clone()
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
    use super::{SessionPool, SharedSessionClient, SharedSessionConnector, PING_THROTTLE};
    use crate::{
        JumpServerConnectConfig, ProxyConnectConfig, ShellIntegrationSetup, SshAuth,
        SshConnectConfig,
    };
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tokio::time::{sleep, Duration};

    #[derive(Default)]
    struct FakeConnector {
        connect_count: AtomicUsize,
        fail_first: AtomicBool,
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
        async fn connect(&self, _config: SshConnectConfig) -> Result<FakeClient> {
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
        async fn connect(&self, _config: SshConnectConfig) -> Result<FakeClient> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
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
    async fn coalesces_concurrent_client_connects() {
        let connector = Arc::new(SlowFakeConnector {
            connect_count: AtomicUsize::new(0),
            connect_started: Notify::new(),
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
