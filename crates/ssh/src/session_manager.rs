use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{RusshChannel, RusshClient, SshClient, SshConnectConfig};

#[async_trait]
trait SharedSessionClient: Send + Sync {
    fn is_connected(&self) -> bool;
    async fn disconnect(&mut self) -> Result<()>;
}

#[async_trait]
trait SharedSessionConnector<C>: Send + Sync {
    async fn connect(&self, config: SshConnectConfig) -> Result<C>;
}

#[derive(Default)]
struct SessionState<C> {
    client: Option<Arc<Mutex<C>>>,
}

struct SessionPool<C, K> {
    config: SshConnectConfig,
    connector: K,
    state: Arc<Mutex<SessionState<C>>>,
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
            state: Arc::new(Mutex::new(SessionState { client: None })),
        }
    }

    async fn client(&self) -> Result<Arc<Mutex<C>>> {
        let existing = {
            let state = self.state.lock().await;
            state.client.clone()
        };

        if let Some(client) = existing {
            if client.lock().await.is_connected() {
                return Ok(client);
            }
        }

        let connected = Arc::new(Mutex::new(
            self.connector.connect(self.config.clone()).await?,
        ));
        let mut state = self.state.lock().await;
        state.client = Some(connected.clone());
        Ok(connected)
    }

    async fn invalidate(&self) {
        let mut state = self.state.lock().await;
        state.client = None;
    }

    async fn disconnect(&self) -> Result<()> {
        let client = {
            let mut state = self.state.lock().await;
            state.client.take()
        };

        if let Some(client) = client {
            client.lock().await.disconnect().await?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct RusshClientConnector;

#[async_trait]
impl SharedSessionClient for RusshClient {
    fn is_connected(&self) -> bool {
        SshClient::is_connected(self)
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
}

#[cfg(test)]
mod tests {
    use super::{SessionPool, SharedSessionClient, SharedSessionConnector};
    use crate::{JumpServerConnectConfig, ProxyConnectConfig, SshAuth, SshConnectConfig};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeConnector {
        connect_count: AtomicUsize,
    }

    struct FakeClient {
        connected: bool,
        disconnect_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SharedSessionClient for FakeClient {
        fn is_connected(&self) -> bool {
            self.connected
        }

        async fn disconnect(&mut self) -> Result<()> {
            self.connected = false;
            self.disconnect_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl SharedSessionConnector<FakeClient> for Arc<FakeConnector> {
        async fn connect(&self, _config: SshConnectConfig) -> Result<FakeClient> {
            self.connect_count.fetch_add(1, Ordering::SeqCst);
            Ok(FakeClient {
                connected: true,
                disconnect_count: Arc::new(AtomicUsize::new(0)),
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
    async fn session_manager_reuses_single_connector_invocation_for_repeated_client_access() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        let second = pool.client().await.expect("第二次获取 client 应成功");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn session_manager_invalidate_forces_next_client_access_to_reconnect() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        pool.invalidate().await;
        let second = pool.client().await.expect("失效后再次获取 client 应成功");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn session_manager_disconnect_clears_cached_client() {
        let connector = Arc::new(FakeConnector::default());
        let pool = SessionPool::new(test_config(), connector.clone());

        let first = pool.client().await.expect("第一次获取 client 应成功");
        let disconnect_count = {
            let client = first.lock().await;
            client.disconnect_count.clone()
        };

        pool.disconnect().await.expect("disconnect 应成功");
        let second = pool.client().await.expect("断开后再次获取 client 应成功");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(disconnect_count.load(Ordering::SeqCst), 1);
        assert_eq!(connector.connect_count.load(Ordering::SeqCst), 2);
    }
}
