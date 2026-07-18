use extension_protocol::error::{ProtocolError, error_codes};
use gpui::{AppContext, AsyncApp, Context, PromptLevel, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{DatabaseType, DbConnectionConfig, StoredConnection, Workspace};
use one_core::tab_container::TabOpenMode;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::database_driver_install_progress::{
    DriverInstallProgressSnapshot, DriverInstallProgressView, driver_install_progress_callback,
    mark_driver_install_finished, open_driver_install_progress_dialog,
    watch_driver_install_progress,
};
use crate::extension::ExtensionKind;
use crate::extension::{ExtensionRegistry, ExtensionSummary};
use crate::extension_downloader::{
    DownloadProgressCallback, MarketplaceEntry,
    download_marketplace_entry_to_staging_with_progress, fetch_default_manifest_url,
    fetch_manifest_url, install_from_staging_generic, install_marketplace_entry_generic,
};
const DUCKDB_DRIVER_ID: &str = "duckdb";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRequirement {
    NotRequired,
    Required { driver_id: String },
    InvalidConfig { message: String },
}

/// Generic sidecar requirement shared by every native driver API.
///
/// Domain crates decide whether their selected backend is built in or IPC;
/// this layer only translates that decision into an installable `(api, id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeDriverRequirement {
    NotRequired,
    Required { api: String, driver_id: String },
    InvalidConfig { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeDriverBackend {
    Builtin,
    Ipc { driver_id: String },
}

pub fn required_native_driver(
    api: impl Into<String>,
    backend: NativeDriverBackend,
) -> NativeDriverRequirement {
    let api = api.into();
    if api.trim().is_empty() {
        return NativeDriverRequirement::InvalidConfig {
            message: "native driver api is required".to_string(),
        };
    }
    match backend {
        NativeDriverBackend::Builtin => NativeDriverRequirement::NotRequired,
        NativeDriverBackend::Ipc { driver_id } if driver_id.trim().is_empty() => {
            NativeDriverRequirement::InvalidConfig {
                message: format!("native driver id is required for api `{api}`"),
            }
        }
        NativeDriverBackend::Ipc { driver_id } => NativeDriverRequirement::Required {
            api,
            driver_id: driver_id.trim().to_string(),
        },
    }
}

/// Returns an explicit fallback requirement only for a structured server
/// incompatibility. This keeps auth, TLS, timeout and other operational errors
/// from silently switching driver implementations.
pub fn fallback_native_driver_for_error(
    api: impl Into<String>,
    fallback_driver_id: impl Into<String>,
    error: &ProtocolError,
) -> Option<NativeDriverRequirement> {
    (error.code == error_codes::SERVER_INCOMPATIBLE).then(|| {
        required_native_driver(
            api,
            NativeDriverBackend::Ipc {
                driver_id: fallback_driver_id.into(),
            },
        )
    })
}

pub trait DatabaseDriverConnectionOpener: Sized + 'static {
    fn open_database_connection(
        &mut self,
        connection: &StoredConnection,
        workspace: Option<Workspace>,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

pub fn required_driver_for_config(config: &DbConnectionConfig) -> DriverRequirement {
    match &config.database_type {
        DatabaseType::DuckDB => DriverRequirement::Required {
            driver_id: DUCKDB_DRIVER_ID.to_string(),
        },
        DatabaseType::External { .. } => required_external_driver(config),
        _ => DriverRequirement::NotRequired,
    }
}

pub fn find_database_driver_entry<'a>(
    entries: &'a [MarketplaceEntry],
    driver_id: &str,
) -> Option<&'a MarketplaceEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == ExtensionKind::DatabaseDriver && entry.id == driver_id)
}

pub async fn install_database_driver_from_marketplace_with_registry(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    manifest_url: &str,
    driver_id: &str,
    registry: &ExtensionRegistry,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_manifest_url(http_client.clone(), manifest_url).await?;
    let entries = manifest.into_entries();
    let entry = find_database_driver_entry(&entries, driver_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到数据库驱动 {driver_id}"))?;
    install_marketplace_entry_generic(http_client, &entry, registry).await
}

pub fn open_database_connection_with_driver_guard<T>(
    home: &mut T,
    connection: StoredConnection,
    workspace: Option<Workspace>,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: DatabaseDriverConnectionOpener,
{
    let config = match connection.to_db_connection() {
        Ok(config) => config,
        Err(error) => {
            notify_error(window, cx, format!("数据库连接配置无效: {error}"));
            return;
        }
    };

    match required_driver_for_config(&config) {
        DriverRequirement::NotRequired => {
            home.open_database_connection(&connection, workspace, mode, window, cx)
        }
        DriverRequirement::InvalidConfig { message } => notify_error(window, cx, message),
        DriverRequirement::Required { driver_id } => {
            if db::ipc::IpcDriverRegistry::load_default()
                .find(&driver_id)
                .is_some()
            {
                home.open_database_connection(&connection, workspace, mode, window, cx);
            } else {
                prompt_install_driver(connection, workspace, driver_id, mode, window, cx);
            }
        }
    }
}

fn prompt_install_driver<T>(
    connection: StoredConnection,
    workspace: Option<Workspace>,
    driver_id: String,
    mode: TabOpenMode,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: DatabaseDriverConnectionOpener,
{
    let connection_name = connection.name.clone();
    prompt_install_driver_with_completion(
        "database".to_string(),
        driver_id.clone(),
        connection_name,
        window,
        cx,
        move |home, window, cx| {
            home.open_database_connection(&connection, workspace, mode, window, cx);
        },
    );
}

pub fn prompt_install_database_driver<T>(
    driver_id: String,
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: 'static,
{
    prompt_install_native_driver(
        "database".to_string(),
        driver_id,
        connection_name,
        window,
        cx,
    );
}

pub fn prompt_install_native_driver<T>(
    api: String,
    driver_id: String,
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: 'static,
{
    prompt_install_driver_with_completion(
        api,
        driver_id,
        connection_name,
        window,
        cx,
        |_, _, _| {},
    );
}

fn prompt_install_driver_with_completion<T, F>(
    api: String,
    driver_id: String,
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
    on_success: F,
) where
    T: 'static,
    F: FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
{
    if ExtensionRegistry::global().is_none() {
        notify_error(window, cx, format!("扩展系统未初始化，无法安装 {api} 驱动"));
        return;
    }

    let answer = window.prompt(
        PromptLevel::Warning,
        "需要安装驱动",
        Some(&format!(
            "连接「{}」需要安装「{}」{} 驱动。",
            connection_name, driver_id, api
        )),
        &["下载并安装", "取消"],
        cx,
    );
    let http_client = cx.http_client();
    let window_handle = window.window_handle();
    let progress_view = cx.new(|_| DriverInstallProgressView::new(&driver_id, &connection_name));
    let progress_view_weak = progress_view.downgrade();
    let progress_snapshot = Arc::new(Mutex::new(DriverInstallProgressSnapshot::default()));
    let progress_finished = Arc::new(AtomicBool::new(false));
    watch_driver_install_progress(
        progress_view_weak.clone(),
        Arc::clone(&progress_snapshot),
        Arc::clone(&progress_finished),
        cx,
    );

    cx.spawn(async move |this: WeakEntity<T>, cx: &mut AsyncApp| {
        if answer.await.ok() != Some(0) {
            progress_finished.store(true, Ordering::Relaxed);
            return;
        }
        open_install_progress_dialog(window_handle, progress_view, cx);
        let install_driver_id = driver_id.clone();
        let progress_callback = driver_install_progress_callback(progress_snapshot);
        let task = Tokio::spawn(cx, async move {
            install_database_driver_from_marketplace(
                http_client,
                &install_driver_id,
                progress_callback,
            )
            .await
        });
        let outcome = match task.await {
            Ok(Ok(_summary)) => Ok(()),
            Ok(Err(error)) => Err(format!("{error:?}")),
            Err(error) => Err(format!("任务执行失败: {error}")),
        };
        progress_finished.store(true, Ordering::Relaxed);
        finish_install_and_open(
            window_handle,
            this,
            api,
            driver_id,
            progress_view_weak,
            outcome,
            on_success,
            cx,
        );
    })
    .detach();
}

async fn install_database_driver_from_marketplace(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    driver_id: &str,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_default_manifest_url(http_client.clone()).await?;
    let entries = manifest.into_entries();
    let entry = find_database_driver_entry(&entries, driver_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到数据库驱动 {driver_id}"))?;
    let staging =
        download_marketplace_entry_to_staging_with_progress(http_client, &entry, on_progress)
            .await?;
    let result = install_staged_database_driver(&staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_staged_database_driver(staging: &std::path::Path) -> anyhow::Result<ExtensionSummary> {
    let registry =
        ExtensionRegistry::global().ok_or_else(|| anyhow::anyhow!("扩展系统未初始化"))?;
    let registry = registry
        .read()
        .map_err(|error| anyhow::anyhow!("registry lock poisoned: {error}"))?;
    install_from_staging_generic(staging, &registry, Some(ExtensionKind::DatabaseDriver))
}

fn open_install_progress_dialog(
    window_handle: gpui::AnyWindowHandle,
    progress_view: gpui::Entity<DriverInstallProgressView>,
    cx: &mut AsyncApp,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        open_driver_install_progress_dialog(progress_view, window, cx);
    });
}

fn finish_install_and_open<T: 'static>(
    window_handle: gpui::AnyWindowHandle,
    target: WeakEntity<T>,
    api: String,
    driver_id: String,
    progress_view: gpui::WeakEntity<DriverInstallProgressView>,
    outcome: Result<(), String>,
    on_success: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    cx: &mut AsyncApp,
) {
    if outcome.is_ok() {
        mark_driver_install_finished(&progress_view, cx);
    }
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.close_dialog(cx);
        if let Some(target) = target.upgrade() {
            target.update(cx, |target, cx| match outcome {
                Ok(()) => {
                    notify_success(window, cx, format!("已安装 {driver_id} {api} 驱动"));
                    on_success(target, window, cx);
                }
                Err(error) => notify_error(window, cx, format!("安装 {api} 驱动失败: {error}")),
            });
        }
    });
}

fn notify_error<T>(window: &mut Window, cx: &mut Context<T>, message: impl Into<String>) {
    window.push_notification(Notification::error(message.into()), cx);
}

fn notify_success<T>(window: &mut Window, cx: &mut Context<T>, message: impl Into<String>) {
    window.push_notification(Notification::success(message.into()), cx);
}

fn required_external_driver(config: &DbConnectionConfig) -> DriverRequirement {
    let driver_id = config
        .database_type
        .external_driver_id()
        .map(str::trim)
        .unwrap_or_default();
    if driver_id.is_empty() {
        return DriverRequirement::InvalidConfig {
            message: "外部数据库连接缺少 driver_id，无法确定需要安装的驱动".to_string(),
        };
    }
    DriverRequirement::Required {
        driver_id: driver_id.to_string(),
    }
}
