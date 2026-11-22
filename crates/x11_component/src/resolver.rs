use std::path::PathBuf;

use std::time::Duration;

use x11_forwarding::{ServerEndpoint, X11Proxy, detect_local_server};

use crate::installer::prepare;
use crate::launch::LaunchSpec;
use crate::metrics::{MetricEvent, record};
use crate::paths::{component_root, default_config, read_installed};
use crate::state::{configured_data_dir, embedded_disable_reason, update_status};
use crate::supervisor::start;
use crate::{X11ComponentError, X11ForwardingStatus, X11Resolution, X11ServerMode};

static RESOLVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn resolve_server(mode: X11ServerMode) -> X11Resolution {
    let _guard = RESOLVE_LOCK.lock().await;
    let resolution = match mode {
        X11ServerMode::External => external().await,
        X11ServerMode::Embedded => embedded(mode).await,
        X11ServerMode::Auto => match external().await {
            Ok(resolution) => Ok(resolution),
            Err(external_error) => {
                record(MetricEvent::AutoExternalFallback);
                match embedded(mode).await {
                    Ok(resolution) => Ok(resolution),
                    Err(embedded_error) => Err(X11ComponentError::Config(format!(
                        "外置 X server 不可用：{external_error}；内置 X server 不可用：{embedded_error}"
                    ))),
                }
            }
        },
    };
    if resolution.is_err() && mode != X11ServerMode::External {
        record(MetricEvent::EmbeddedFailure);
    }
    let resolution = resolution.unwrap_or_else(disabled);
    update_status(resolution.status.clone());
    resolution
}

pub fn installed_version() -> Option<String> {
    let data_dir = configured_data_dir().ok()?;
    read_installed(&component_root(&data_dir))
}

pub async fn reinstall() -> Result<String, X11ComponentError> {
    if !cfg!(any(
        target_os = "windows",
        all(target_os = "macos", target_arch = "aarch64")
    )) {
        return Err(X11ComponentError::UnsupportedPlatform(format!(
            "{}；可改用外置 XQuartz",
            std::env::consts::OS
        )));
    }
    crate::reinstall_with_config(default_config()?).await
}

async fn external() -> Result<X11Resolution, X11ComponentError> {
    let proxy = tokio::task::spawn_blocking(detect_local_server)
        .await
        .map_err(|error| X11ComponentError::Process(error.to_string()))?
        .map_err(|error| X11ComponentError::Config(error.to_string()))?;
    external_proxy(proxy).await
}

/// External 与 Auto 共用的外置成功路径：健康检查只使用本机认证信息。
async fn external_proxy(proxy: X11Proxy) -> Result<X11Resolution, X11ComponentError> {
    proxy
        .probe_local(Duration::from_secs(2))
        .await
        .map_err(|error| X11ComponentError::Config(format!("外置 X11 握手失败：{error}")))?;
    let server = endpoint_name(proxy.endpoint());
    Ok(X11Resolution {
        proxy: Some(proxy),
        status: X11ForwardingStatus::External { server },
    })
}

async fn embedded(mode: X11ServerMode) -> Result<X11Resolution, X11ComponentError> {
    if let Some(reason) = embedded_disable_reason() {
        return Err(X11ComponentError::EmbeddedDisabled(reason));
    }
    if !cfg!(any(
        target_os = "windows",
        all(target_os = "macos", target_arch = "aarch64")
    )) {
        return Err(X11ComponentError::UnsupportedPlatform(format!(
            "{}；可改用外置 XQuartz",
            std::env::consts::OS
        )));
    }
    let manual = manual_binary();
    // 没有内嵌组件时提前给出准确原因：否则 `Auto` 每次都要走到安装阶段才失败，
    // 状态面板会显示成「安装失败」而不是「本构建未内嵌」。
    if manual.is_none() && !crate::bundle::bundled() {
        return Err(X11ComponentError::Config(
            "当前构建未内嵌 X11 组件（未启用 x11-bundled 特性），请改用外置 X server".into(),
        ));
    }
    let spec = match manual {
        Some(executable) => LaunchSpec::manual(configured_data_dir()?, executable)?,
        None => {
            let config = default_config()?;
            let data_dir = config.data_dir.clone();
            let component = prepare(config, mode).await?;
            LaunchSpec::installed(data_dir, component)?
        }
    };
    start(spec).await
}

fn manual_binary() -> Option<PathBuf> {
    std::env::var_os("NAVOP_X11_YSERVER_PATH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn endpoint_name(endpoint: &ServerEndpoint) -> String {
    match endpoint {
        ServerEndpoint::Unix(path) => path.display().to_string(),
        ServerEndpoint::Inet { host, port } => format!("{host}:{port}"),
    }
}

fn disabled(error: X11ComponentError) -> X11Resolution {
    X11Resolution {
        proxy: None,
        status: X11ForwardingStatus::Disabled {
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use x11_forwarding::MagicCookie;

    use super::*;

    async fn authenticating_server(
        expected_cookie: MagicCookie,
    ) -> (ServerEndpoint, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 12];
            stream.read_exact(&mut header).await.unwrap();
            let little_endian = header[0] == b'l';
            let read_u16 = |bytes: [u8; 2]| {
                if little_endian {
                    u16::from_le_bytes(bytes)
                } else {
                    u16::from_be_bytes(bytes)
                }
            };
            let name_len = usize::from(read_u16([header[6], header[7]]));
            let data_len = usize::from(read_u16([header[8], header[9]]));
            let padded = |len: usize| (len + 3) & !3;
            let mut packet = header.to_vec();
            packet.resize(12 + padded(name_len) + padded(data_len), 0);
            stream.read_exact(&mut packet[12..]).await.unwrap();
            let name = &packet[12..12 + name_len];
            let data = &packet[12 + padded(name_len)..12 + padded(name_len) + data_len];
            assert_eq!(name, b"MIT-MAGIC-COOKIE-1");
            assert_eq!(data, expected_cookie.bytes());
            stream.write_all(&[1, 0, 11, 0, 0, 0, 0, 0]).await.unwrap();
        });
        (endpoint, server)
    }

    /// 默认构建（没有内嵌组件）里 `Embedded` 必须立刻报「未内嵌」，而不是先尝试安装。
    #[cfg(not(feature = "bundled"))]
    #[tokio::test]
    async fn embedded_without_the_bundled_component_is_a_config_error() {
        let error = embedded(X11ServerMode::Embedded).await.unwrap_err();
        assert!(
            matches!(error, X11ComponentError::Config(ref message) if message.contains("未内嵌 X11 组件")),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn external_resolution_probes_with_real_cookie() {
        let cookie = MagicCookie::from_slice(&[0x5c; 16]).unwrap();
        let (endpoint, server) = authenticating_server(cookie.clone()).await;

        let resolution = external_proxy(X11Proxy::for_endpoint(endpoint, cookie))
            .await
            .unwrap();

        assert!(matches!(
            resolution.status,
            X11ForwardingStatus::External { .. }
        ));
        assert!(resolution.proxy.is_some());
        server.await.unwrap();
    }

    #[cfg(target_os = "macos")]
    fn assert_external(resolution: X11Resolution) {
        assert!(
            matches!(resolution.status, X11ForwardingStatus::External { .. }),
            "外置 XQuartz 应直接可用，不得回退内置：{:?}",
            resolution.status
        );
        assert!(resolution.proxy.is_some());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires a locally installed and running XQuartz"]
    async fn resolves_running_xquartz_in_external_mode() {
        assert_external(resolve_server(X11ServerMode::External).await);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires a locally installed and running XQuartz"]
    async fn resolves_running_xquartz_in_auto_mode_without_starting_embedded() {
        assert_external(resolve_server(X11ServerMode::Auto).await);
    }
}
