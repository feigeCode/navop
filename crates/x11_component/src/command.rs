use std::path::{Path, PathBuf};

use rand::{RngCore as _, rngs::OsRng};
use x11_forwarding::ServerEndpoint;

use crate::X11ComponentError;

#[derive(Clone, Debug)]
pub struct EndpointPlan(ServerEndpoint);

impl EndpointPlan {
    pub fn random(manual: bool) -> Result<Self, X11ComponentError> {
        if manual || cfg!(any(target_os = "linux", target_os = "macos")) {
            #[cfg(unix)]
            return Self::random_unix();
        }
        Self::random_tcp()
    }

    pub fn random_tcp() -> Result<Self, X11ComponentError> {
        for _ in 0..128 {
            // yserver 的 TCP 端口固定为 6000 + display，只在合法范围内随机。
            let display = 100 + (OsRng.next_u32() % 59_436) as u16;
            let port = 6000 + display;
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Ok(Self(ServerEndpoint::Inet {
                    host: "127.0.0.1".into(),
                    port,
                }));
            }
        }
        Err(X11ComponentError::Process(
            "无法在 6000 + display 范围内找到空闲回环端口".into(),
        ))
    }

    #[cfg(unix)]
    fn random_unix() -> Result<Self, X11ComponentError> {
        for _ in 0..128 {
            let display = 100 + (OsRng.next_u32() % 59_436) as u16;
            if !PathBuf::from(format!("/tmp/.X11-unix/X{display}")).exists() {
                return Ok(Self(ServerEndpoint::Unix(PathBuf::from(format!(
                    "/tmp/.X11-unix/X{display}"
                )))));
            }
        }
        Err(X11ComponentError::Process(
            "无法找到空闲 X11 Unix socket".into(),
        ))
    }

    #[cfg(test)]
    pub fn persisted(endpoint: ServerEndpoint) -> Self {
        Self(endpoint)
    }

    pub fn endpoint(&self) -> ServerEndpoint {
        self.0.clone()
    }

    #[cfg(test)]
    pub fn display_number(&self) -> Option<u16> {
        match &self.0 {
            ServerEndpoint::Inet { port, .. } => port.checked_sub(6000),
            ServerEndpoint::Unix(path) => display_from_socket(path),
        }
    }
}

#[derive(Clone, Copy)]
enum TcpListen {
    Disabled,
    Loopback,
    LegacyAllInterfaces,
}

/// 所有上游启动参数集中在此处；只有固定 Windows fork 可以声明 loopback 监听。
pub struct CommandSpec {
    executable: PathBuf,
    endpoint: ServerEndpoint,
    xauth: PathBuf,
    tcp_listen: TcpListen,
}

impl CommandSpec {
    pub fn installed_tcp(executable: PathBuf, endpoint: &ServerEndpoint, xauth: PathBuf) -> Self {
        Self {
            executable,
            endpoint: endpoint.clone(),
            xauth,
            tcp_listen: TcpListen::Loopback,
        }
    }

    pub fn manual_upstream_tcp(
        executable: PathBuf,
        endpoint: &ServerEndpoint,
        xauth: PathBuf,
    ) -> Self {
        Self {
            executable,
            endpoint: endpoint.clone(),
            xauth,
            tcp_listen: TcpListen::LegacyAllInterfaces,
        }
    }

    pub fn unix(executable: PathBuf, endpoint: &ServerEndpoint, xauth: PathBuf) -> Self {
        Self {
            executable,
            endpoint: endpoint.clone(),
            xauth,
            tcp_listen: TcpListen::Disabled,
        }
    }

    pub fn validate_secure_endpoint(&self) -> Result<(), X11ComponentError> {
        match (&self.endpoint, self.tcp_listen) {
            (ServerEndpoint::Unix(_), TcpListen::Disabled) => Ok(()),
            (ServerEndpoint::Inet { host, port }, TcpListen::Loopback)
                if host == "127.0.0.1" && *port >= 6000 =>
            {
                Ok(())
            }
            (ServerEndpoint::Inet { .. }, TcpListen::LegacyAllInterfaces) => {
                Err(X11ComponentError::EmbeddedDisabled(
                    "Windows 手动 yserver 不支持 TCP：无法验证旧上游监听地址，请使用内置组件或 Unix socket"
                        .into(),
                ))
            }
            _ => Err(X11ComponentError::Config("内置端点不是本地安全地址".into())),
        }
    }

    pub fn display_argument(&self) -> String {
        let number = match &self.endpoint {
            ServerEndpoint::Inet { port, .. } => port.saturating_sub(6000),
            ServerEndpoint::Unix(path) => display_from_socket(path).unwrap_or(0),
        };
        format!(":{number}")
    }

    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec![
            self.display_argument(),
            "-auth".into(),
            self.xauth.display().to_string(),
        ];
        #[cfg(target_os = "macos")]
        args.push("--rootless".into());
        match self.tcp_listen {
            TcpListen::Loopback => {
                args.push("--listen".into());
                args.push("127.0.0.1".into());
            }
            TcpListen::LegacyAllInterfaces => {
                args.push("-listen".into());
                args.push("tcp".into());
            }
            TcpListen::Disabled => {
                args.push("-nolisten".into());
                args.push("tcp".into());
            }
        }
        args
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

#[cfg(test)]
pub fn random_loopback_endpoint(endpoint: &ServerEndpoint) -> Option<ServerEndpoint> {
    match endpoint {
        ServerEndpoint::Inet { host, .. } if host == "127.0.0.1" => Some(endpoint.clone()),
        _ => None,
    }
}

fn display_from_socket(path: &Path) -> Option<u16> {
    path.file_name()?.to_str()?.strip_prefix('X')?.parse().ok()
}

#[cfg(test)]
mod tests;
