use std::sync::{Mutex, OnceLock};

use std::sync::Arc;

use domain_api::{
    HomeContext, PortForwardingRuntime, RemoteProtocol, ShellExtras, WslDistributionInfo,
};
use gpui::{App, AppContext as _, Window};
use gpui_component::ActiveTheme as _;
use one_core::storage::StoredConnection;
use one_core::tab_container::{TabItem, TabOpenMode};
use terminal_view::{TerminalConnectionKind, TerminalWorkspace};

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

    fn terminal_theme_section(&self, cx: &App) -> Option<gpui::AnyElement> {
        Some(crate::terminal_theme_section::render(cx))
    }

    fn sidebar_colors(&self, cx: &App) -> Option<domain_api::SidebarColors> {
        let colors = terminal_view::TerminalColors::from_application_theme(cx.theme());
        Some(domain_api::SidebarColors {
            background: colors.background,
            foreground: colors.foreground,
            muted: colors.muted,
            muted_foreground: colors.muted_foreground,
            border: colors.border,
            accent: colors.accent,
        })
    }

    fn save_temporary_connection(
        &self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !app_kit::vault::ensure_master_key_ready(window, cx) {
            return false;
        }
        let workspaces = cx
            .try_global::<one_core::storage::GlobalStorageState>()
            .and_then(|state| state.storage.get::<one_core::storage::WorkspaceRepository>())
            .and_then(|repository| {
                use one_core::storage::traits::Repository as _;
                repository.list().ok()
            })
            .unwrap_or_default();
        let config = ssh_form::SshFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(connection),
            initial_workspace_id: None,
            on_saved: Some(Arc::new(|saved, _action, window, cx| {
                use gpui_component::WindowExt as _;
                window.push_notification(
                    format!("已保存连接：{}", saved.name),
                    cx,
                );
            })),
            workspaces,
            teams: one_core::cloud_sync::get_cached_team_options(cx),
        };
        one_core::popup_window::open_reusable_popup_window(
            one_core::popup_window::PopupWindowOptions::new("保存为连接").size(820.0, 750.0),
            "app-kit-save-temporary-ssh".to_string(),
            move |window, cx| {
                cx.new(|cx| ssh_form::SshFormWindow::new(config.clone(), window, cx))
            },
            Some(window),
            cx,
        );
        true
    }

    fn open_session_log(&self, path: std::path::PathBuf, window: &mut Window, cx: &mut App) -> bool {
        crate::recording::open_session_log_file(path, window, cx);
        true
    }

    fn open_terminal_in_dir(
        &self,
        home: &mut dyn HomeContext,
        working_dir: Option<String>,
        connection: Option<StoredConnection>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        match connection {
            Some(connection) => crate::SshConnectionOpener.open_ssh_terminal(
                &connection,
                working_dir,
                TabOpenMode::Activate,
                home,
                window,
                cx,
            ),
            None => {
                crate::terminals::open_local_terminal_with_dir(working_dir, home, window, cx)
            }
        }
    }

    fn duplicate_terminal_tab(
        &self,
        tab: &TabItem,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let view = tab.content().view();
        let Ok(workspace) = view.downcast::<TerminalWorkspace>() else {
            return false;
        };
        let kind = workspace.read(cx).connection_kind(cx);
        if kind == TerminalConnectionKind::Local {
            return app_kit::registry().open_tab(
                &domain_api::TabIntent::Terminal { profile: None },
                TabOpenMode::Activate,
                home,
                window,
                cx,
            );
        }
        let Some(connection_id) = workspace.read(cx).connection_id(cx) else {
            return false;
        };
        let Some(connection) = home
            .connections()
            .iter()
            .find(|connection| connection.id == Some(connection_id))
            .cloned()
        else {
            return false;
        };
        app_kit::registry().open_connection(&connection, TabOpenMode::Activate, home, window, cx)
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
