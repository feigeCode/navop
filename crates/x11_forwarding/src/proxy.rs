//! 连接级 X11 代理：保管本机真实 cookie，向各 SSH 会话签发 fake cookie，
//! 并把 sshd 回连的 x11 通道按 cookie 路由到对应的授权记录。
//!
//! 一条 SSH 连接上的多个终端各自持有不同 fake cookie；sshd 回连时不带
//! 会话标识，只能凭 setup 报文里的 cookie 找回授权——这也是 OpenSSH
//! 客户端采用的 cookie spoofing 模型。

use std::fmt;
use std::sync::Arc;

use dashmap::DashMap;

use crate::bridge::{self, CookieExchange};
use crate::{ForwardRequest, MagicCookie, ServerEndpoint, X11Result};

/// 本机 X server 要求的认证方式。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LocalAuth {
    Cookie(MagicCookie),
    #[cfg(any(target_os = "windows", test))]
    None,
}

/// 一次签发留下的授权记录。
struct Grant {
    /// 本机认证信息（仅保存在本机内存）。
    local: LocalAuth,
    /// 对应 SSH single-connection 语义：首个回连用掉即作废。
    single_use: bool,
}

struct State {
    endpoint: ServerEndpoint,
    /// key 为签发出的 fake cookie。
    grants: DashMap<[u8; 16], Grant>,
}

impl CookieExchange for Arc<State> {
    fn exchange(&self, presented: &MagicCookie) -> Option<LocalAuth> {
        let key = presented.bytes();
        if self.grants.get(key).is_some_and(|g| g.single_use) {
            return self.grants.remove(key).map(|(_, g)| g.local);
        }
        self.grants.get(key).map(|g| g.local.clone())
    }
}

/// 本机 X server 代理（每条启用转发的 SSH 连接一个实例）。
#[derive(Clone)]
pub struct X11Proxy {
    screen: u32,
    local: LocalAuth,
    state: Arc<State>,
}

impl fmt::Debug for X11Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("X11Proxy")
            .field("endpoint", &self.state.endpoint)
            .field("screen", &self.screen)
            .finish_non_exhaustive()
    }
}

impl X11Proxy {
    /// 为组件管理的本机 X server 创建代理，继续沿用 fake cookie 委托链路。
    pub fn for_endpoint(endpoint: ServerEndpoint, local: MagicCookie) -> Self {
        Self::new(endpoint, 0, local)
    }

    pub(crate) fn new(endpoint: ServerEndpoint, screen: u32, local: MagicCookie) -> Self {
        Self::with_auth(endpoint, screen, LocalAuth::Cookie(local))
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn new_without_auth(endpoint: ServerEndpoint, screen: u32) -> Self {
        Self::with_auth(endpoint, screen, LocalAuth::None)
    }

    fn with_auth(endpoint: ServerEndpoint, screen: u32, local: LocalAuth) -> Self {
        Self {
            screen,
            local,
            state: Arc::new(State {
                endpoint,
                grants: DashMap::new(),
            }),
        }
    }

    pub fn endpoint(&self) -> &ServerEndpoint {
        &self.state.endpoint
    }

    /// 使用本机认证信息完成健康检查，不签发或出示 fake cookie。
    pub async fn probe_local(&self, timeout: std::time::Duration) -> X11Result<()> {
        match &self.local {
            LocalAuth::Cookie(cookie) => {
                crate::probe::probe_endpoint(&self.state.endpoint, cookie, timeout).await
            }
            #[cfg(any(target_os = "windows", test))]
            LocalAuth::None => {
                crate::probe::probe_endpoint_without_auth(&self.state.endpoint, timeout).await
            }
        }
    }

    /// 为一个新 SSH 会话签发转发请求：生成 fake cookie 并登记授权。
    pub fn issue_request(&self, single_use: bool) -> ForwardRequest {
        let fake = MagicCookie::generate();
        self.state.grants.insert(
            *fake.bytes(),
            Grant {
                local: self.local.clone(),
                single_use,
            },
        );
        ForwardRequest::new(single_use, self.screen, fake.hex())
    }

    /// 服务端拒绝请求后撤销授权，避免残留死记录。
    pub fn retract_request(&self, request: &ForwardRequest) {
        if let Ok(fake) = MagicCookie::from_hex(request.cookie_hex()) {
            self.state.grants.remove(fake.bytes());
        }
    }

    /// 交给 SSH client handler 的回连处理句柄。
    pub fn handle(&self) -> X11ProxyHandle {
        X11ProxyHandle {
            state: self.state.clone(),
        }
    }
}

/// 处理 sshd 回连 x11 通道的轻量句柄（可克隆、跨线程共享）。
#[derive(Clone)]
pub struct X11ProxyHandle {
    state: Arc<State>,
}

impl fmt::Debug for X11ProxyHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("X11ProxyHandle")
            .field("endpoint", &self.state.endpoint)
            .finish_non_exhaustive()
    }
}

impl X11ProxyHandle {
    pub async fn run_channel<S>(&self, stream: S, originator: Option<(String, u16)>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        tracing::debug!(target: "ssh.x11", ?originator, "收到 x11 回连通道");
        bridge::relay(stream, &self.state.endpoint, &self.state).await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::setup::ClientHello;

    async fn authenticating_server(expected_cookie: MagicCookie) -> ServerEndpoint {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 12];
            stream.read_exact(&mut header).await.unwrap();
            let auth_len = ClientHello::auth_section_len(&header).unwrap();
            let mut packet = header.to_vec();
            packet.resize(12 + auth_len, 0);
            stream.read_exact(&mut packet[12..]).await.unwrap();
            let hello = ClientHello::decode(&packet).unwrap();
            assert_eq!(hello.auth_name, "MIT-MAGIC-COOKIE-1");
            if hello.auth_data == expected_cookie.bytes() {
                stream.write_all(&[1, 0, 11, 0, 0, 0, 0, 0]).await.unwrap();
            } else {
                stream
                    .write_all(&hello.failure_reply("invalid cookie"))
                    .await
                    .unwrap();
            }
        });
        endpoint
    }

    fn proxy() -> X11Proxy {
        X11Proxy::new(
            ServerEndpoint::Unix(PathBuf::from("/tmp/.X11-unix/X0")),
            0,
            MagicCookie::from_slice(&[0x5a; 16]).unwrap(),
        )
    }

    #[tokio::test]
    async fn local_probe_presents_real_cookie_without_issuing_a_grant() {
        let cookie = MagicCookie::from_slice(&[0xa5; 16]).unwrap();
        let endpoint = authenticating_server(cookie.clone()).await;
        let proxy = X11Proxy::for_endpoint(endpoint, cookie);

        proxy.probe_local(Duration::from_secs(1)).await.unwrap();

        assert!(proxy.state.grants.is_empty());
    }

    #[test]
    fn endpoint_constructor_preserves_cookie_delegation() {
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: 6137,
        };
        let cookie = MagicCookie::from_slice(&[0x37; 16]).unwrap();
        let proxy = X11Proxy::for_endpoint(endpoint.clone(), cookie.clone());

        assert_eq!(proxy.endpoint(), &endpoint);
        let request = proxy.issue_request(false);
        let fake = MagicCookie::from_hex(request.cookie_hex()).unwrap();
        assert_eq!(proxy.state.exchange(&fake), Some(LocalAuth::Cookie(cookie)));
    }

    #[test]
    fn issued_request_uses_fresh_fake_cookie() {
        let proxy = proxy();
        let request = proxy.issue_request(false);

        assert_eq!(request.auth_name(), "MIT-MAGIC-COOKIE-1");
        assert_eq!(request.cookie_hex().len(), 32);
        let LocalAuth::Cookie(local) = &proxy.local else {
            panic!("test proxy should use cookie authentication");
        };
        assert_ne!(request.cookie_hex(), local.hex());

        let fake = MagicCookie::from_hex(request.cookie_hex()).unwrap();
        let exchanged = proxy.state.exchange(&fake).unwrap();
        assert_eq!(exchanged, proxy.local);
    }

    #[test]
    fn multi_use_grant_survives_repeated_exchange() {
        let proxy = proxy();
        let request = proxy.issue_request(false);
        let fake = MagicCookie::from_hex(request.cookie_hex()).unwrap();

        assert!(proxy.state.exchange(&fake).is_some());
        assert!(proxy.state.exchange(&fake).is_some());
    }

    #[test]
    fn single_use_grant_is_consumed_once() {
        let proxy = proxy();
        let request = proxy.issue_request(true);
        let fake = MagicCookie::from_hex(request.cookie_hex()).unwrap();

        assert!(proxy.state.exchange(&fake).is_some());
        assert!(proxy.state.exchange(&fake).is_none());
    }

    #[test]
    fn retract_removes_grant() {
        let proxy = proxy();
        let request = proxy.issue_request(false);
        let fake = MagicCookie::from_hex(request.cookie_hex()).unwrap();

        proxy.retract_request(&request);

        assert!(proxy.state.exchange(&fake).is_none());
    }

    #[test]
    fn unregistered_cookie_is_refused() {
        let proxy = proxy();
        assert!(proxy.state.exchange(&MagicCookie::generate()).is_none());
    }
}
