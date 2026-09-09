use gpui::{Context, Window};
use one_core::storage::{DatabaseType, DbConnectionConfig, StoredConnection, Workspace};
use one_core::tab_container::TabOpenMode;
use std::sync::Arc;

use crate::extension::ExtensionKind;
use crate::extension::{ExtensionRegistry, ExtensionSummary};
use crate::extension_downloader::{
    DownloadProgressCallback, MarketplaceEntry, fetch_default_manifest_url, fetch_manifest_url,
    install_marketplace_entry_generic, install_marketplace_entry_with_progress,
};
use crate::install_flow::{notify_error, run_install_with_progress_prompt};
const DUCKDB_DRIVER_ID: &str = "duckdb";
/// TDengine 主仓已移除原生插件,统一走 tdengine IPC 外部驱动。
const TDENGINE_DRIVER_ID: &str = "tdengine";

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
    Required {
        api: String,
        driver_id: String,
        minimum_version: Option<String>,
    },
    InvalidConfig {
        message: String,
    },
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
    required_native_driver_with_minimum(api, backend, None)
}

pub fn required_native_driver_at_least(
    api: impl Into<String>,
    backend: NativeDriverBackend,
    minimum_version: impl Into<String>,
) -> NativeDriverRequirement {
    required_native_driver_with_minimum(api, backend, Some(minimum_version.into()))
}

fn required_native_driver_with_minimum(
    api: impl Into<String>,
    backend: NativeDriverBackend,
    minimum_version: Option<String>,
) -> NativeDriverRequirement {
    let api = api.into();
    if api.trim().is_empty() {
        return NativeDriverRequirement::InvalidConfig {
            message: "native driver api is required".to_string(),
        };
    }
    let minimum_version = match minimum_version {
        Some(version) if semver::Version::parse(version.trim()).is_err() => {
            return NativeDriverRequirement::InvalidConfig {
                message: format!(
                    "native driver minimum version `{}` is invalid for api `{api}`",
                    version.trim()
                ),
            };
        }
        Some(version) => Some(version.trim().to_string()),
        None => None,
    };
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
            minimum_version,
        },
    }
}

pub fn native_driver_is_installed(api: &str, driver_id: &str) -> bool {
    db::ipc::IpcDriverRegistry::load_default()
        .find_by_api(api, driver_id)
        .is_some()
}

fn native_driver_meets_requirement(
    api: &str,
    driver_id: &str,
    minimum_version: Option<&str>,
) -> bool {
    db::ipc::IpcDriverRegistry::load_default()
        .find_by_api(api, driver_id)
        .is_some_and(|driver| driver_version_meets_minimum(&driver.version, minimum_version))
}

pub(crate) fn driver_version_meets_minimum(
    installed_version: &str,
    minimum_version: Option<&str>,
) -> bool {
    let Some(minimum_version) = minimum_version else {
        return true;
    };
    let Ok(installed) = semver::Version::parse(installed_version.trim()) else {
        return false;
    };
    let Ok(minimum) = semver::Version::parse(minimum_version.trim()) else {
        return false;
    };
    installed >= minimum
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
        // TDengine 连接(历史存储的 DatabaseType::TDengine)按外部驱动守卫:
        // 未安装 tdengine 驱动时引导用户从扩展市场安装,与 DuckDB 同策略。
        DatabaseType::TDengine => DriverRequirement::Required {
            driver_id: TDENGINE_DRIVER_ID.to_string(),
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

pub fn open_native_driver_connection_with_guard<T, F>(
    target: &mut T,
    requirement: NativeDriverRequirement,
    connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
    on_ready: F,
) where
    T: 'static,
    F: FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
{
    match requirement {
        NativeDriverRequirement::NotRequired => on_ready(target, window, cx),
        NativeDriverRequirement::InvalidConfig { message } => notify_error(window, cx, message),
        NativeDriverRequirement::Required {
            api,
            driver_id,
            minimum_version,
        } => {
            if native_driver_meets_requirement(&api, &driver_id, minimum_version.as_deref()) {
                on_ready(target, window, cx);
            } else {
                prompt_install_driver_with_completion(
                    api,
                    driver_id,
                    minimum_version,
                    connection_name,
                    window,
                    cx,
                    on_ready,
                );
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
        None,
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
        None,
        connection_name,
        window,
        cx,
        |_, _, _| {},
    );
}

fn prompt_install_driver_with_completion<T, F>(
    api: String,
    driver_id: String,
    minimum_version: Option<String>,
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
    let requirement_message = minimum_version
        .as_deref()
        .map(|version| format!("（最低版本 {version}）"))
        .unwrap_or_default();
    let install_driver_id = driver_id.clone();
    let install_minimum_version = minimum_version.clone();
    run_install_with_progress_prompt(
        window,
        cx,
        (driver_id.clone(), connection_name.clone()),
        "需要安装或更新驱动",
        format!(
            "连接「{}」需要安装或更新「{}」{} 驱动{}。",
            connection_name, driver_id, api, requirement_message
        ),
        &["下载并安装/更新", "取消"],
        move |http_client, progress_callback| {
            install_database_driver_from_marketplace(
                http_client,
                install_driver_id,
                install_minimum_version,
                progress_callback,
            )
        },
        on_success,
        format!("已安装 {driver_id} {api} 驱动"),
        format!("安装 {api} 驱动失败"),
    );
}

async fn install_database_driver_from_marketplace(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    driver_id: String,
    minimum_version: Option<String>,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_default_manifest_url(http_client.clone()).await?;
    let entries = manifest.into_entries();
    let entry = find_database_driver_entry_for_requirement(
        &entries,
        &driver_id,
        minimum_version.as_deref(),
    )?
    .clone();
    let summary = install_marketplace_entry_with_progress(
        http_client,
        &entry,
        ExtensionKind::DatabaseDriver,
        on_progress,
    )
    .await?;
    db::ipc::IpcDriverRegistry::refresh_global_registry();
    Ok(summary)
}

pub(crate) fn find_database_driver_entry_for_requirement<'a>(
    entries: &'a [MarketplaceEntry],
    driver_id: &str,
    minimum_version: Option<&str>,
) -> anyhow::Result<&'a MarketplaceEntry> {
    let entry = find_database_driver_entry(entries, driver_id)
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到数据库驱动 {driver_id}"))?;
    if !driver_version_meets_minimum(&entry.version, minimum_version) {
        let required = minimum_version.unwrap_or("<unknown>");
        anyhow::bail!(
            "扩展市场中的数据库驱动 {driver_id} 版本 {} 不满足宿主要求的最低版本 {required}",
            display_driver_version(&entry.version)
        );
    }
    Ok(entry)
}

fn display_driver_version(version: &str) -> &str {
    let version = version.trim();
    if version.is_empty() {
        "<empty>"
    } else {
        version
    }
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
