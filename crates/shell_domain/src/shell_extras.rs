use std::sync::{Mutex, OnceLock};

use std::sync::Arc;

use domain_api::{PortForwardingRuntime, RemoteProtocol, ShellExtras, WslDistributionInfo};
use gpui::App;
use one_core::storage::StoredConnection;

static PORT_FORWARDING: OnceLock<Arc<tokio::sync::Mutex<PortForwardingRuntime>>> = OnceLock::new();

static WSL_DISTRIBUTIONS: OnceLock<Mutex<Option<Vec<WslDistributionInfo>>>> = OnceLock::new();

fn distributions_cache() -> &'static Mutex<Option<Vec<WslDistributionInfo>>> {
    WSL_DISTRIBUTIONS.get_or_init(|| Mutex::new(None))
}

pub struct ShellExtrasImpl;

impl ShellExtras for ShellExtrasImpl {
    fn wsl_distributions(&self, _cx: &App) -> Option<Vec<WslDistributionInfo>> {
        distributions_cache()
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    fn refresh_wsl_distributions(&self, cx: &mut App) {
        detect(cx);
    }

    fn open_remote_desktop_fullscreen(
        &self,
        connection: &StoredConnection,
        protocol: RemoteProtocol,
        title: String,
        cx: &mut App,
    ) -> bool {
        let Ok(mut params) = connection.to_remote_desktop_params() else {
            return false;
        };
        params.protocol = match protocol {
            RemoteProtocol::Rdp => one_core::storage::RemoteDesktopProtocol::Rdp,
            RemoteProtocol::Vnc => one_core::storage::RemoteDesktopProtocol::Vnc,
        };
        let options = remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(params);
        crate::remote_desktop_window::open_remote_desktop_fullscreen_window(options, title, cx);
        true
    }

    fn launch_remote_desktop_external(&self, connection: &StoredConnection) -> Result<(), String> {
        let params = connection
            .to_remote_desktop_params()
            .map_err(|error| format!("{error:#}"))?;
        #[cfg(target_os = "windows")]
        {
            crate::remote_desktop_window::launch_mstsc_fullscreen(&params)
                .map_err(|error| format!("{error:#}"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = params;
            Err("当前平台不支持外部远程桌面客户端".to_string())
        }
    }

    fn port_forwarding_runtime(&self) -> Option<Arc<tokio::sync::Mutex<PortForwardingRuntime>>> {
        Some(port_forwarding_runtime())
    }
}

/// 端口转发运行时：由 shell 领域持有，连接打开器与主页共享同一份状态。
pub(crate) fn port_forwarding_runtime() -> Arc<tokio::sync::Mutex<PortForwardingRuntime>> {
    PORT_FORWARDING
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(PortForwardingRuntime::new())))
        .clone()
}

pub(crate) fn init(cx: &mut App) {
    detect(cx);
}

/// 后台识别 WSL 发行版，结果写入进程内缓存；识别完成前菜单不显示该区段。
fn detect(cx: &mut App) {
    if !cfg!(target_os = "windows") {
        return;
    }
    cx.spawn(async move |_cx| {
        let detected = smol::unblock(terminal::list_wsl_distributions).await;
        let Ok(distributions) = detected else {
            return;
        };
        let infos = distributions
            .into_iter()
            .map(|distribution| WslDistributionInfo {
                name: distribution.name.into(),
                is_default: distribution.is_default,
            })
            .collect::<Vec<_>>();
        if let Ok(mut guard) = distributions_cache().lock() {
            *guard = Some(infos);
        }
    })
    .detach();
}
