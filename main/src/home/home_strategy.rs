use crate::home_tab::HomePage;
use extension_view::{ExtensionViewHost, MarketplaceInstallOutcome};
use gpui::{App, AppContext, AsyncApp, ClickEvent, Context, ParentElement, SharedString, Styled, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::storage::{ConnectionType, ExtensionConnectionParams, StoredConnection, Workspace};
use one_core::tab_container::{TabItem, TabOpenMode};
use remote_desktop::RemoteDesktopProtocol;
use std::sync::Arc;

pub(crate) trait ConnectionOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    );
}

pub(crate) fn build_connection_open_strategy(
    connection: StoredConnection,
    workspace: Option<Workspace>,
) -> Box<dyn ConnectionOpenStrategy> {
    match connection.connection_type {
        ConnectionType::SshSftp => Box::new(SshOpenStrategy { connection }),
        ConnectionType::Database => Box::new(DatabaseOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::Redis => Box::new(RedisOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::MongoDB => Box::new(MongoOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::Mqtt => Box::new(MiddlewareExtensionOpenStrategy { connection }),
        ConnectionType::Serial => Box::new(SerialOpenStrategy { connection }),
        ConnectionType::Telnet => Box::new(TelnetOpenStrategy { connection }),
        ConnectionType::PortForwarding => Box::new(PortForwardingOpenStrategy { connection }),
        ConnectionType::Rdp => Box::new(RemoteDesktopOpenStrategy {
            connection,
            protocol: RemoteDesktopProtocol::Rdp,
        }),
        ConnectionType::Vnc => Box::new(RemoteDesktopOpenStrategy {
            connection,
            protocol: RemoteDesktopProtocol::Vnc,
        }),
        ConnectionType::Extension => Box::new(ExtensionOpenStrategy { connection }),
        _ => Box::new(NoopOpenStrategy),
    }
}

struct ExtensionOpenStrategy {
    connection: StoredConnection,
}

#[cfg(not(feature = "shell-plugins"))]
impl ConnectionOpenStrategy for ExtensionOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let Ok(params) = self.connection.to_extension_params() else {
            window.push_notification("Extension connection data is invalid", cx);
            return;
        };
        let Some(service) = cx
            .try_global::<universal_plugins::GlobalUniversalPluginService>()
            .map(|global| global.service())
        else {
            window.push_notification("Extension runtime is unavailable", cx);
            return;
        };
        // 无 Shell 构建以 workbench 绑定为决策点:
        // 有原生工作台即可打开(legacy shellViewId 不阻塞),
        // 没有工作台才提示需要 shell-plugins 构建。
        let Some(workbench) = service
            .resource_workbench_for_connection(&params.extension_id, &params.contribution_id)
        else {
            window.push_notification(
                "This extension connection requires the shell-plugins build",
                cx,
            );
            return;
        };
        let Some(contribution) =
            service.resource_connection(&params.extension_id, &params.contribution_id)
        else {
            window.push_notification("This extension contribution is unavailable", cx);
            return;
        };
        open_native_extension_connection(
            service,
            self.connection,
            contribution,
            workbench,
            mode,
            window,
            cx,
        );
    }
}

#[cfg(feature = "shell-plugins")]
impl ConnectionOpenStrategy for ExtensionOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let Ok(params) = self.connection.to_extension_params() else {
            window.push_notification("Extension connection data is invalid", cx);
            return;
        };
        let Some(host) = cx
            .try_global::<universal_plugins::ShellPluginHost>()
            .cloned()
        else {
            window.push_notification("Extension runtime is unavailable", cx);
            return;
        };
        let Some(contribution) =
            host.resource_connection(&params.extension_id, &params.contribution_id)
        else {
            window.push_notification(
                format!(
                    "Extension {} is missing or no longer provides connection {}",
                    params.extension_id, params.contribution_id
                ),
                cx,
            );
            return;
        };
        if let Some(workbench) =
            host.resource_workbench_for_connection(&params.extension_id, &params.contribution_id)
        {
            let service = cx
                .global::<universal_plugins::GlobalUniversalPluginService>()
                .service();
            open_native_extension_connection(
                service,
                self.connection,
                contribution,
                workbench,
                mode,
                window,
                cx,
            );
        } else if let Err(error) = host.open_connection(
            universal_plugins::ConnectionShellOpen {
                connection: self.connection,
                contribution,
                mode,
            },
            window,
            cx,
        ) {
            window.push_notification(format!("Failed to open extension connection: {error}"), cx);
        }
    }
}

/// 打开原生扩展连接工作台 tab(共享实现,两种 feature 构建都使用)。
/// headless tab 到 ShellPluginHost 的注册由 ExtensionConnectionTab::load
/// 内部按 global 存在性自行处理。
#[allow(clippy::too_many_arguments)]
fn open_native_extension_connection(
    service: universal_plugins::UniversalPluginService,
    connection: StoredConnection,
    contribution: extension_runtime::RegisteredResourceConnectionContribution,
    workbench: extension_runtime::RegisteredResourceWorkbenchContribution,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut App,
) {
    let connection_id = connection.id.expect("saved extension connection");
    let tab_id = format!("extension-connection:{connection_id}");
    let tabs = cx
        .global::<one_core::tab_container::GlobalTabContainer>()
        .primary_pane();
    // 把 tab 创建与容器更新移出 HomePage 租约,避免激活已有 tab 时同步触发
    // HomePage::on_deactivate 造成实体租约重入。
    window.defer(cx, move |window, cx| {
        let tab = universal_plugins::ExtensionConnectionTab::load(
            service,
            connection,
            contribution,
            workbench,
            cx,
        );
        tabs.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy_with_mode(
                tab_id.clone(),
                mode,
                move |_, _| TabItem::new(tab_id.clone(), "extension-connection", tab.clone()),
                window,
                cx,
            );
        });
    });
}

struct MiddlewareExtensionOpenStrategy {
    connection: StoredConnection,
}

/// 旧内置 MQTT 连接(ConnectionType::Mqtt)在扩展化改造后按需下载扩展再打开。
///
/// 优先把历史连接参数迁移为扩展连接形态(com.navop.middleware.mqtt);若对应
/// 扩展尚未安装,则先从扩展市场下载安装,安装完成后打开连接。
impl ConnectionOpenStrategy for MiddlewareExtensionOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let mut connection = self.connection;
        if !connection.try_migrate_legacy_middleware_connection() {
            window.push_notification("MQTT connection data is invalid", cx);
            return;
        }
        let Ok(params) = connection.to_extension_params() else {
            window.push_notification("MQTT connection data is invalid", cx);
            return;
        };
        if extension_connection_is_registered(&params, cx) {
            open_extension_connection_now(connection, mode, window, cx);
        } else {
            install_middleware_extension(connection, params, mode, window, cx);
        }
    }
}

/// 扩展是否已安装并注册了对应连接贡献项。
fn extension_connection_is_registered(params: &ExtensionConnectionParams, cx: &App) -> bool {
    #[cfg(feature = "shell-plugins")]
    {
        cx.try_global::<universal_plugins::ShellPluginHost>()
            .map(|host| {
                host.resource_connection(&params.extension_id, &params.contribution_id)
                    .is_some()
            })
            .unwrap_or(false)
    }
    #[cfg(not(feature = "shell-plugins"))]
    {
        cx.try_global::<universal_plugins::GlobalUniversalPluginService>()
            .map(|global| {
                global
                    .service()
                    .resource_connection(&params.extension_id, &params.contribution_id)
                    .is_some()
            })
            .unwrap_or(false)
    }
}

/// 从扩展市场下载并安装缺失的中间件扩展,完成后打开连接。
fn install_middleware_extension(
    connection: StoredConnection,
    params: ExtensionConnectionParams,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) {
    let extension_id = params.extension_id.clone();
    let window_handle = window.window_handle();
    let host: Arc<dyn ExtensionViewHost> = Arc::new(extension_runtime::MainExtensionViewHost);
    let http_client = cx.http_client();
    window.push_notification(
        Notification::info(format!("Downloading extension {extension_id}...")).autohide(true),
        cx,
    );
    let install_host = host.clone();
    let task = cx.background_spawn(async move {
        let entries = install_host.load_marketplace_entries(http_client.clone()).await?;
        let entry = entries
            .into_iter()
            .find(|entry| extension_view::marketplace_entry_install_id(entry) == extension_id)
            .ok_or_else(|| anyhow::anyhow!("marketplace entry `{extension_id}` not found"))?;
        install_host
            .review_marketplace_entry(http_client, entry)
            .await
    });
    cx.spawn(async move |_home, cx: &mut AsyncApp| {
        let outcome = task.await;
        let _ = cx.update_window(window_handle, |_, window, cx| match outcome {
            Ok(MarketplaceInstallOutcome::Installed(summary)) => {
                host.refresh_after_extension_change(summary.kind, cx);
                open_extension_connection_now(connection, mode, window, cx);
            }
            Ok(MarketplaceInstallOutcome::NeedsPermission(downloaded)) => {
                prompt_middleware_extension_install(
                    downloaded,
                    connection,
                    mode,
                    host.clone(),
                    window,
                    cx,
                );
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(format!("Failed to download extension: {error}"))
                        .autohide(true),
                    cx,
                );
            }
        });
    })
    .detach();
}

/// 扩展需要权限确认时弹出确认框,确认后安装并打开连接。
fn prompt_middleware_extension_install(
    downloaded: extension_view::DownloadedMarketplaceExtension,
    connection: StoredConnection,
    mode: TabOpenMode,
    host: Arc<dyn ExtensionViewHost>,
    window: &mut Window,
    cx: &mut App,
) {
    let review_summary = SharedString::from(downloaded.review.summary.clone());
    let staging = downloaded.staging.clone();
    let entry_name = downloaded.entry.name.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let host = host.clone();
        let staging = staging.clone();
        let connection = connection.clone();
        dialog
            .title(SharedString::from(format!("Install extension {entry_name}")))
            .child(
                gpui::div()
                    .child(review_summary.clone())
                    .max_w(gpui::px(480.0)),
            )
            .confirm()
            .on_ok(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                match host.install_confirmed_staging(staging.clone()) {
                    Ok(summary) => {
                        host.refresh_after_extension_change(summary.kind, cx);
                        open_extension_connection_now(connection.clone(), mode, window, cx);
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!("Failed to install extension: {error}")),
                        cx,
                    ),
                }
                true
            })
    });
}

/// 打开已注册的扩展连接(不依赖 HomePage,便于安装完成后在回调中复用)。
#[cfg(not(feature = "shell-plugins"))]
fn open_extension_connection_now(
    connection: StoredConnection,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(params) = connection.to_extension_params() else {
        window.push_notification("Extension connection data is invalid", cx);
        return;
    };
    let Some(service) = cx
        .try_global::<universal_plugins::GlobalUniversalPluginService>()
        .map(|global| global.service())
    else {
        window.push_notification("Extension runtime is unavailable", cx);
        return;
    };
    let Some(workbench) = service
        .resource_workbench_for_connection(&params.extension_id, &params.contribution_id)
    else {
        window.push_notification(
            "This extension connection requires the shell-plugins build",
            cx,
        );
        return;
    };
    let Some(contribution) =
        service.resource_connection(&params.extension_id, &params.contribution_id)
    else {
        window.push_notification("This extension contribution is unavailable", cx);
        return;
    };
    open_native_extension_connection(
        service,
        connection,
        contribution,
        workbench,
        mode,
        window,
        cx,
    );
}

#[cfg(feature = "shell-plugins")]
fn open_extension_connection_now(
    connection: StoredConnection,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(params) = connection.to_extension_params() else {
        window.push_notification("Extension connection data is invalid", cx);
        return;
    };
    let Some(host) = cx
        .try_global::<universal_plugins::ShellPluginHost>()
        .cloned()
    else {
        window.push_notification("Extension runtime is unavailable", cx);
        return;
    };
    let Some(contribution) =
        host.resource_connection(&params.extension_id, &params.contribution_id)
    else {
        window.push_notification(
            format!(
                "Extension {} is missing or no longer provides connection {}",
                params.extension_id, params.contribution_id
            ),
            cx,
        );
        return;
    };
    if let Some(workbench) =
        host.resource_workbench_for_connection(&params.extension_id, &params.contribution_id)
    {
        let service = cx
            .global::<universal_plugins::GlobalUniversalPluginService>()
            .service();
        open_native_extension_connection(
            service,
            connection,
            contribution,
            workbench,
            mode,
            window,
            cx,
        );
    } else if let Err(error) = host.open_connection(
        universal_plugins::ConnectionShellOpen {
            connection,
            contribution,
            mode,
        },
        window,
        cx,
    ) {
        window.push_notification(format!("Failed to open extension connection: {error}"), cx);
    }
}

struct SshOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for SshOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_ssh_terminal_with_mode(self.connection, mode, window, cx);
    }
}

struct DatabaseOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for DatabaseOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let DatabaseOpenStrategy {
            connection,
            workspace,
        } = *self;
        extension_runtime::database_driver_install::open_database_connection_with_driver_guard(
            home, connection, workspace, mode, window, cx,
        );
    }
}

impl extension_runtime::remote_desktop_provider_install::RemoteDesktopConnectionOpener
    for HomePage
{
    fn open_remote_desktop_connection(
        &mut self,
        connection: &StoredConnection,
        protocol: RemoteDesktopProtocol,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_remote_desktop_with_mode(connection.clone(), protocol, mode, window, cx);
    }
}

impl extension_runtime::database_driver_install::DatabaseDriverConnectionOpener for HomePage {
    fn open_database_connection(
        &mut self,
        connection: &StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_item_to_tab_with_mode(connection, workspace, mode, window, cx);
    }
}

struct RedisOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for RedisOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let RedisOpenStrategy {
            connection,
            workspace,
        } = *self;
        home.open_redis_tab_with_mode(connection, workspace, mode, window, cx);
    }
}

struct MongoOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

fn mongodb_driver_id(connection: &StoredConnection) -> String {
    connection
        .to_mongodb_params()
        .map(|params| params.driver_variant.driver_id().to_string())
        .unwrap_or_else(|_| mongodb_runtime::DEFAULT_MONGODB_MODERN_DRIVER_ID.to_string())
}

impl ConnectionOpenStrategy for MongoOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let MongoOpenStrategy {
            connection,
            workspace,
        } = *self;
        let connection_name = connection.name.clone();
        let driver_id = mongodb_driver_id(&connection);
        let requirement = extension_runtime::database_driver_install::required_native_driver(
            "mongodb",
            extension_runtime::database_driver_install::NativeDriverBackend::Ipc { driver_id },
        );
        extension_runtime::database_driver_install::open_native_driver_connection_with_guard(
            home,
            requirement,
            connection_name,
            window,
            cx,
            move |home, window, cx| {
                home.open_mongodb_tab_with_mode(connection, workspace, mode, window, cx);
            },
        );
    }
}

struct NoopOpenStrategy;

struct SerialOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for SerialOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_serial_terminal_with_mode(self.connection, mode, window, cx);
    }
}

struct TelnetOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for TelnetOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_telnet_terminal_with_mode(self.connection, mode, window, cx);
    }
}

struct PortForwardingOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for PortForwardingOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        _mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        home.open_port_forwarding_tab(self.connection, _mode, window, cx);
    }
}

struct RemoteDesktopOpenStrategy {
    connection: StoredConnection,
    protocol: RemoteDesktopProtocol,
}

impl ConnectionOpenStrategy for RemoteDesktopOpenStrategy {
    fn open(
        self: Box<Self>,
        home: &mut HomePage,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<HomePage>,
    ) {
        let RemoteDesktopOpenStrategy {
            connection,
            protocol,
        } = *self;
        // 双击等常规打开一律走 tab；独立全屏窗口仅保留在连接的右键菜单里
        // （Connection.open_in_fullscreen_window）。
        extension_runtime::remote_desktop_provider_install::open_remote_desktop_connection_with_provider_guard(
            home, connection, protocol, mode, window, cx,
        );
    }
}

impl ConnectionOpenStrategy for NoopOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        _mode: TabOpenMode,
        _window: &mut Window,
        _cx: &mut Context<HomePage>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::mongodb_driver_id;
    use one_core::storage::{MongoDBParams, MongoDriverVariant, StoredConnection};

    #[test]
    fn remote_desktop_double_click_opens_in_a_tab() {
        let source = include_str!("home_strategy.rs").replace("\r\n", "\n");
        let start = source
            .find("impl ConnectionOpenStrategy for RemoteDesktopOpenStrategy")
            .expect("remote desktop open strategy");
        let end = source[start..]
            .find("impl ConnectionOpenStrategy for NoopOpenStrategy")
            .map(|offset| start + offset)
            .expect("next strategy");
        let strategy = &source[start..end];

        assert!(
            strategy.contains("open_remote_desktop_connection_with_provider_guard("),
            "double-click must open the remote desktop in a tab"
        );
        assert!(
            !strategy.contains("open_remote_desktop_fullscreen_window"),
            "the fullscreen window must stay behind the context-menu action"
        );
    }

    #[test]
    fn mongodb_driver_id_follows_the_saved_variant() {
        let connection = StoredConnection::new_mongodb(
            "legacy mongo".to_string(),
            MongoDBParams {
                driver_variant: MongoDriverVariant::Legacy,
                connection_string: String::new(),
                host: "127.0.0.1".to_string(),
                port: Some(27017),
                database: None,
                username: None,
                password: None,
                credential_reference: None,
                auth_source: None,
                replica_set: None,
                read_preference: None,
                use_srv_record: false,
                direct_connection: false,
                use_tls: false,
                connect_timeout_seconds: None,
                application_name: None,
                ssh_tunnel: None,
            },
            None,
        );

        assert_eq!("mongodb-legacy", mongodb_driver_id(&connection));
    }

    #[test]
    fn mongodb_driver_id_supports_the_mongodb_3_2_variant() {
        let connection = StoredConnection::new_mongodb(
            "mongo 3.2".to_string(),
            MongoDBParams {
                driver_variant: MongoDriverVariant::Legacy32,
                connection_string: String::new(),
                host: "127.0.0.1".to_string(),
                port: Some(27017),
                database: None,
                username: None,
                password: None,
                credential_reference: None,
                auth_source: None,
                replica_set: None,
                read_preference: None,
                use_srv_record: false,
                direct_connection: false,
                use_tls: false,
                connect_timeout_seconds: None,
                application_name: None,
                ssh_tunnel: None,
            },
            None,
        );

        assert_eq!("mongodb-legacy-3-2", mongodb_driver_id(&connection));
    }
}
