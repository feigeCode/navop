//! 本机 X server 解析：产品统一使用外置 X server（XQuartz / 本地 X.org）。
//!
//! 探测 DISPLAY 与 Xauthority 后做一次真实的 X11 setup 握手，成功才把
//! [`X11Proxy`] 交给 SSH 会话签发 fake cookie；失败只返回可见状态，不阻断
//! SSH 连接（与历史上的「X11 转发已停用」降级行为一致）。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::{ServerEndpoint, X11Proxy, detect_local_server};

/// 健康检查超时：与历史上的外置握手保持一致。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 本机 X server 的可用状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum X11ForwardingStatus {
    /// 探测通过，`server` 是端点名，用于状态栏展示。
    External { server: String },
    /// 没有可用的本机 X server；`reason` 直接展示给用户。
    Disabled { reason: String },
}

/// 一次解析的结果：可用时带 proxy，不可用时带原因。
#[derive(Clone)]
pub struct X11Resolution {
    pub proxy: Option<X11Proxy>,
    pub status: X11ForwardingStatus,
}

/// X server 端点变化时递增，让新会话不复用旧 X11 transport。
static GENERATION: AtomicU64 = AtomicU64::new(0);
static LAST_ENDPOINT: Mutex<Option<ServerEndpoint>> = Mutex::new(None);

/// 当前解析代数：端点变化（X server 换到另一个 display 或套接字）后递增。
pub fn resolution_generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// 解析本机外置 X server；失败时返回 [`X11ForwardingStatus::Disabled`]。
pub async fn resolve_external() -> X11Resolution {
    let proxy = match tokio::task::spawn_blocking(detect_local_server).await {
        Ok(Ok(proxy)) => proxy,
        Ok(Err(error)) => return disabled(format!("未检测到本机 X server：{error}")),
        Err(error) => return disabled(format!("本机 X server 探测任务失败：{error}")),
    };
    if let Err(error) = proxy.probe_local(PROBE_TIMEOUT).await {
        return disabled(format!("外置 X11 握手失败：{error}"));
    }
    note_endpoint(proxy.endpoint());
    let server = endpoint_name(proxy.endpoint());
    X11Resolution {
        proxy: Some(proxy),
        status: X11ForwardingStatus::External { server },
    }
}

fn disabled(reason: String) -> X11Resolution {
    X11Resolution {
        proxy: None,
        status: X11ForwardingStatus::Disabled { reason },
    }
}

fn note_endpoint(endpoint: &ServerEndpoint) {
    let mut last = LAST_ENDPOINT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if last.as_ref() != Some(endpoint) {
        *last = Some(endpoint.clone());
        GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

fn endpoint_name(endpoint: &ServerEndpoint) -> String {
    match endpoint {
        ServerEndpoint::Unix(path) => path.display().to_string(),
        ServerEndpoint::Inet { host, port } => format!("{host}:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn endpoint_names_are_display_and_host_port() {
        assert_eq!(
            "/private/tmp/.X11-unix/X0",
            endpoint_name(&ServerEndpoint::Unix(PathBuf::from(
                "/private/tmp/.X11-unix/X0"
            )))
        );
        assert_eq!(
            "127.0.0.1:6000",
            endpoint_name(&ServerEndpoint::Inet {
                host: "127.0.0.1".into(),
                port: 6000,
            })
        );
    }

    #[test]
    fn generation_only_moves_when_the_endpoint_changes() {
        let first = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: 6001,
        };
        let second = ServerEndpoint::Unix(PathBuf::from("/tmp/.X11-unix/X7"));

        note_endpoint(&first);
        let settled = resolution_generation();
        note_endpoint(&first);
        assert_eq!(settled, resolution_generation(), "同一端点不应改变代数");

        note_endpoint(&second);
        assert!(
            resolution_generation() > settled,
            "端点变化必须让新会话重建 X11 transport"
        );
    }
}
