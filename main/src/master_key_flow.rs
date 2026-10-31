//! 主密钥修改与重置流程——`main` 侧兼容层
//!
//! 流程本体已下沉到 `app_kit::vault::{master_key_flow, reset}`，供 navop 全量版与
//! `navop-shell` / `navop-data` 两个独立产品共享。这里不再实现流程，只做两件事：
//!
//! 1. 原样 re-export 下沉后的公开符号，`crate::master_key_flow::*` 的调用点全部不变；
//! 2. 把"只有 main 才能做"的四件事注册成 [`app_kit::vault::VaultHostHooks`]：
//!    Navop Cloud / 个人同步的云端重加密、云端数据清空、云端密钥版本上报，以及
//!    笔记目录定位。注册发生在原有入口被调用时（解锁后、修改后、重置时），
//!    不新增初始化时序；独立产品不调用这里，钩子保持 no-op。

use std::sync::{Arc, OnceLock, RwLock};

use gpui::{App, AsyncApp};
use notes::NotesStorage;
use one_core::cloud_sync::{
    CloudApiClient, CloudReencryptStats, CloudSyncService, GlobalCloudUser, SyncEngine, SyncError,
    new_user_config, reencrypt_onet_cloud,
};
use one_core::gpui_tokio::Tokio;
use one_core::master_key_meta;
use one_core::settings::{AppSettings, SyncProvider};
use one_core::storage::GlobalStorageState;
use rust_i18n::t;

use crate::personal_sync_runtime;
use app_kit::vault::{CloudWipeScope, VaultHostHooks, install_host_hooks, master_key_flow};

// 流程与弹窗本体都在 app_kit；这里只留 main 还有调用点的两个符号。
// （修改/重置弹窗见 `master_key_dialogs`，流程符号按需从这里 re-export 即可。）
pub(crate) use app_kit::vault::master_key_flow::CloudReencryptNotifier;
// `RESET_CONFIRM_WORD` 目前只有测试在用（`home_tab::encryption` 的断言），
// 非测试构建里这个 re-export 会显得没被用到。
#[allow(unused_imports)]
pub(crate) use app_kit::vault::master_key_flow::RESET_CONFIRM_WORD;

/// 钩子只注册一次；`app_kit` 侧本身也是 `OnceLock`，这里多一层只为让注册点写法直白。
static HOOKS: OnceLock<()> = OnceLock::new();

pub(crate) fn register_host_hooks() {
    HOOKS.get_or_init(|| {
        install_host_hooks(VaultHostHooks {
            reencrypt_cloud: Arc::new(|old_key, new_key, key_version, notifier, cx| {
                start_cloud_reencrypt(old_key, new_key, key_version, notifier, cx)
            }),
            wipe_cloud: Arc::new(|scope: CloudWipeScope, cx| wipe_cloud_data(scope, cx)),
            report_key_version: Arc::new(report_key_version_to_cloud_inner),
            notes_root: Arc::new(|| NotesStorage::configured_root().ok()),
        });
    });
}

/// 解锁成功后触发 KDF 迁移
pub fn run_kdf_upgrade_after_unlock(cx: &mut App) {
    register_host_hooks();
    master_key_flow::run_kdf_upgrade_after_unlock(cx);
}

/// 把当前主密钥与版本号上报云端（首次设置与修改后都会调用）
pub fn report_key_version_to_cloud(cx: &mut App) {
    register_host_hooks();
    master_key_flow::report_key_version_to_cloud(cx);
}

/// 启动云端重加密。返回是否真的启动了（未登录/未配置同步时为 false）
fn start_cloud_reencrypt(
    old_key: &str,
    new_key: &str,
    key_version: u32,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> bool {
    match AppSettings::global(cx).sync_provider {
        SyncProvider::Personal => {
            personal_sync_runtime::reencrypt_cloud_data(old_key, new_key, key_version, notifier, cx)
        }
        SyncProvider::OnetCloud => {
            start_onet_cloud_reencrypt(old_key, new_key, key_version, notifier, cx)
        }
    }
}

/// 一次 Navop Cloud 重加密任务需要的全部东西（避免 `Tokio::spawn` 闭包参数过长）
struct OnetReencryptJob {
    client: Arc<dyn CloudApiClient>,
    service: Arc<RwLock<CloudSyncService>>,
    storage: one_core::storage::StorageManager,
    old_key: String,
    new_key: String,
    key_version: u32,
    user_id: String,
}

/// Navop Cloud 侧重加密：先把新密钥塞进同步服务，再异步重加密并全量同步一次
fn start_onet_cloud_reencrypt(
    old_key: &str,
    new_key: &str,
    key_version: u32,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> bool {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        tracing::info!("未登录 Navop Cloud，跳过云端重加密");
        return false;
    };
    let job = OnetReencryptJob {
        client: crate::auth::get_auth_service(cx).cloud_client(),
        service: sync_service(&user.id, new_key),
        storage: cx.global::<GlobalStorageState>().storage.clone(),
        old_key: old_key.to_string(),
        new_key: new_key.to_string(),
        key_version,
        user_id: user.id.clone(),
    };
    let task = Tokio::spawn(cx, async move { reencrypt_and_sync(job).await });
    report_reencrypt(task, notifier, cx);
    true
}

/// 同步服务先登录并换上新的内存主密钥，重加密与后续同步都用它
fn sync_service(user_id: &str, new_key: &str) -> Arc<RwLock<CloudSyncService>> {
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    if let Ok(mut guard) = service.write() {
        let _ = guard.set_logged_in(user_id.to_string());
        let _ = guard.set_master_key_directly(new_key.to_string());
    }
    service
}

/// 重加密云端数据，顺带触发一次全量同步
async fn reencrypt_and_sync(job: OnetReencryptJob) -> Result<CloudReencryptStats, SyncError> {
    let stats = reencrypt_onet_cloud(
        &job.client,
        &job.service,
        &job.old_key,
        &job.new_key,
        job.key_version,
        &job.user_id,
    )
    .await?;
    // 版本信息已随 user_configs 上传，立刻全量同步一次，
    // 让其他设备尽早看到"云端密钥已更换"
    let engine = SyncEngine::new(job.client, job.service, job.storage);
    let _ = engine.sync().await;
    Ok(stats)
}

/// 等重加密任务结束，回到窗口线程弹提示
fn report_reencrypt<E>(
    task: gpui::Task<Result<Result<CloudReencryptStats, SyncError>, E>>,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) where
    E: std::fmt::Display + 'static,
{
    cx.spawn(async move |cx: &mut AsyncApp| {
        let message = reencrypt_message(&task.await);
        let _ = cx.update(move |cx: &mut App| {
            if let Some(notify) = notifier.as_ref() {
                notify(cx, message.clone());
            }
        });
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

/// 把重加密结果转成提示文案并记 pending 标记
fn reencrypt_message<E: std::fmt::Display>(
    outcome: &Result<Result<CloudReencryptStats, SyncError>, E>,
) -> String {
    match outcome {
        Ok(Ok(stats)) => {
            master_key_meta::set_pending_cloud_reencrypt(stats.incomplete());
            tracing::info!(
                "云端重加密完成：共 {} 条，重加密 {} 条，失败 {} 条，跳过团队数据 {} 条",
                stats.total,
                stats.re_encrypted,
                stats.failed,
                stats.skipped_team
            );
            if stats.incomplete() {
                t!("Encryption.cloud_reencrypt_failed").to_string()
            } else {
                t!("Encryption.cloud_reencrypt_done").to_string()
            }
        }
        Ok(Err(error)) => {
            master_key_meta::set_pending_cloud_reencrypt(true);
            tracing::error!("云端重加密失败（已记 pending，下次同步前补做）: {error}");
            t!("Encryption.cloud_reencrypt_failed").to_string()
        }
        Err(error) => {
            master_key_meta::set_pending_cloud_reencrypt(true);
            tracing::error!("云端重加密任务失败（已记 pending）: {error}");
            t!("Encryption.cloud_reencrypt_failed").to_string()
        }
    }
}

/// 按重置范围清空云端：个人同步数据与 Navop Cloud 数据分别按勾选处理
fn wipe_cloud_data(scope: CloudWipeScope, cx: &mut App) {
    if scope.personal_sync {
        personal_sync_runtime::wipe_cloud_data(cx);
    }
    if scope.personal_sync || scope.team_data {
        wipe_onet_cloud(scope.team_data, cx);
    }
}

/// 把当前主密钥与版本号写进 `user_configs`
fn report_key_version_to_cloud_inner(cx: &mut App) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        return;
    };
    let Some(raw_key) = one_core::crypto::get_raw_master_key() else {
        return;
    };
    if AppSettings::global(cx).sync_provider != SyncProvider::OnetCloud {
        return;
    }

    let client = crate::auth::get_auth_service(cx).cloud_client();
    let user_id = user.id.clone();
    let config = new_user_config(&user_id, &raw_key, master_key_meta::key_version());

    let task = Tokio::spawn(cx, async move {
        client
            .save_user_config(&config)
            .await
            .map_err(|error| format!("{error:?}"))
    });
    cx.spawn(async move |_cx: &mut AsyncApp| {
        if let Ok(Err(error)) = task.await {
            tracing::warn!("上报云端密钥版本失败: {error}");
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

/// 清空 Navop Cloud 侧数据：个人记录按 `cloud_sync` 勾选，团队记录按 `team_data` 勾选
fn wipe_onet_cloud(include_team: bool, cx: &mut App) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        return;
    };
    if AppSettings::global(cx).sync_provider != SyncProvider::OnetCloud {
        return;
    }
    let client: Arc<dyn CloudApiClient> = crate::auth::get_auth_service(cx).cloud_client();
    let user_id = user.id.clone();
    let task = Tokio::spawn(cx, async move {
        delete_cloud_records(&client, &user_id, include_team).await
    });
    report_wipe(task, cx);
}

/// 逐条删除云端同步记录，并把用户配置清成空版本
async fn delete_cloud_records(
    client: &Arc<dyn CloudApiClient>,
    user_id: &str,
    include_team: bool,
) -> Result<usize, String> {
    let records = client
        .list_sync_data(None, None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let mut deleted = 0usize;
    for record in records {
        if record.team_id.is_some() && !include_team {
            continue;
        }
        client
            .delete_sync_data(&record.id)
            .await
            .map_err(|error| format!("{error:?}"))?;
        deleted += 1;
    }
    // 用户配置一并清空：验证数据与旧版本号对全新安装没有意义
    let empty = new_user_config(user_id, "", 0);
    client
        .save_user_config(&empty)
        .await
        .map_err(|error| format!("{error:?}"))?;
    Ok(deleted)
}

/// 等云端清空任务结束，按结果落 pending 标记
fn report_wipe<E>(task: gpui::Task<Result<Result<usize, String>, E>>, cx: &mut App)
where
    E: std::fmt::Display + 'static,
{
    cx.spawn(async move |_cx: &mut AsyncApp| {
        match task.await {
            Ok(Ok(deleted)) => {
                master_key_meta::set_pending_cloud_wipe(false);
                tracing::info!("云端数据已清空：{deleted} 条");
            }
            Ok(Err(error)) => {
                master_key_meta::set_pending_cloud_wipe(true);
                tracing::error!("云端数据清空失败（已记 pending）: {error}");
            }
            Err(error) => {
                master_key_meta::set_pending_cloud_wipe(true);
                tracing::error!("云端数据清空任务失败（已记 pending）: {error}");
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}
