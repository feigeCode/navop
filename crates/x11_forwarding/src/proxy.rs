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
use crate::{ForwardRequest, MagicCookie, ServerEndpoint};

/// 一次签发留下的授权记录。
struct Grant {
    /// 本机真实 cookie（仅保存在本机内存）。
    local: MagicCookie,
    /// 对应 SSH single-connection 语义：首个回连用掉即作废。
    single_use: bool,
}

struct State {
    endpoint: ServerEndpoint,
    /// key 为签发出的 fake cookie。
    grants: DashMap<[u8; 16], Grant>,
}

impl CookieExchange for Arc<State> {
    fn exchange(&self, presented: &MagicCookie) -> Option<MagicCookie> {
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
    local: MagicCookie,
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
    pub(crate) fn new(endpoint: ServerEndpoint, screen: u32, local: MagicCookie) -> Self {
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

    use super::*;

    fn proxy() -> X11Proxy {
        X11Proxy::new(
            ServerEndpoint::Unix(PathBuf::from("/tmp/.X11-unix/X0")),
            0,
            MagicCookie::from_slice(&[0x5a; 16]).unwrap(),
        )
    }

    #[test]
    fn issued_request_uses_fresh_fake_cookie() {
        let proxy = proxy();
        let request = proxy.issue_request(false);

        assert_eq!(request.auth_name(), "MIT-MAGIC-COOKIE-1");
        assert_eq!(request.cookie_hex().len(), 32);
        assert_ne!(
            request.cookie_hex(),
            proxy.local.hex(),
            "签发给远端的不能是本机真实 cookie"
        );

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
