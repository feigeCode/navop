//! 主密钥修改与重置流程编排
//!
//! 这里只放"做什么、按什么顺序做"，界面在 `master_key_dialogs.rs`。
//! 关键顺序约束（详见 `plan-master-key-change.md`）：
//!
//! 1. 本地先行：先在一个 SQLite 事务里把连接、钥匙串、团队密钥缓存和冲突快照
//!    全部换成新密钥，成功后才切换验证数据；中途失败本地整体回滚，不会出现
//!    "一半新一半旧"的密文。
//! 2. 云端不回滚：云端重加密依赖网络，失败时记 pending 标记下次补做，而不是把
//!    本地已经自洽的新密钥退回去——回滚同样要网络，且只会制造更大的不一致。
//! 3. 重置等价全新安装：验证文件、主密钥文件、版本文件与业务数据一起清掉，
//!    清完 `has_repo_password_set()` 为 false，下次进入会走首次设置流程。

use std::sync::{Arc, RwLock};

use gpui::{App, AsyncApp, Global};
use notes::NotesStorage;
use one_core::cloud_sync::personal::reseal_webdav_password_with_keys;
use one_core::cloud_sync::{
    CloudApiClient, CloudSyncService, GlobalCloudUser, SyncEngine, SyncError, new_user_config,
    reencrypt_navop_cloud,
};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::master_key_meta;
use one_core::settings::{AppSettings, SyncProvider};
use one_core::storage::GlobalStorageState;
use rust_i18n::t;

use crate::personal_sync_runtime;

/// 重置确认词
pub const RESET_CONFIRM_WORD: &str = "RESET";

/// 云端重加密结束后的结果通知：参数是给用户看的提示文案。
///
/// 用闭包而不是直接传 `WindowHandle`：句柄是泛型类型，在异步任务里还要跨
/// App / Window 边界，交给调用方（握有 window 的那一侧）封装最省事。
pub type CloudReencryptNotifier = Box<dyn Fn(&mut App, String) + 'static>;

/// 主密钥固定必清的本地表：连接、钥匙串、分组与各类同步缓存。
///
/// 这些数据要么本身是密文，要么是密文派生出来的索引，保留它们等于留下一份
/// 永远解不开的数据，因此不提供"保留"选项。
const REQUIRED_LOCAL_TABLES: &[&str] = &[
    "connections",
    "credential_entries",
    "workspaces",
    "team_key_cache",
    "team_membership_cache",
    "personal_sync_conflicts",
    "personal_sync_status",
    "pending_cloud_deletions",
];

/// 重置范围：勾选 = 删除该项，默认全部勾选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetScope {
    /// 云端团队共享数据（团队密钥加密，主密钥重置不影响其可读性）
    pub team_data: bool,
    /// 查询历史
    pub query_history: bool,
    /// 命令历史与快捷命令
    pub command_history: bool,
    /// 笔记（磁盘文件）
    pub notes: bool,
    /// 云同步数据与版本信息
    pub cloud_sync: bool,
}

impl Default for ResetScope {
    fn default() -> Self {
        Self {
            team_data: true,
            query_history: true,
            command_history: true,
            notes: true,
            cloud_sync: true,
        }
    }
}

/// 重置范围需要在设置页勾选、在侧边栏入口读取，因此放进全局状态
pub struct GlobalResetScope {
    pub scope: ResetScope,
}

impl Global for GlobalResetScope {}

pub fn reset_scope(cx: &App) -> ResetScope {
    cx.try_global::<GlobalResetScope>()
        .map(|state| state.scope)
        .unwrap_or_default()
}

pub fn set_reset_scope(scope: ResetScope, cx: &mut App) {
    if !cx.has_global::<GlobalResetScope>() {
        cx.set_global(GlobalResetScope { scope });
    }
    cx.global_mut::<GlobalResetScope>().scope = scope;
}

/// 修改主密钥的结果
pub struct ChangeOutcome {
    pub connections: usize,
    pub credentials: usize,
    pub team_key_caches: usize,
    pub key_version: u32,
    /// 本次是否真的启动了云端重加密：启动了才会有"云端重写完成 / 失败"的后续提示
    pub cloud_reencrypt_started: bool,
}

/// 执行主密钥修改：本地轮换 → 切换密钥 → 版本递增 → 启动云端重加密
///
/// `notifier` 用于云端重加密结束后补一条结果提示：异步任务拿不到窗口，只能
/// 由调用方把"怎么弹提示"封装好带进来。
pub fn apply_change_master_key(
    old_key: &str,
    new_key: &str,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> Result<ChangeOutcome, String> {
    crypto::validate_master_key_change(old_key, new_key, new_key)
        .map_err(|error| error.to_string())?;

    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let connection = storage.connection();
    let stats = one_core::storage::re_encrypt_secrets(&connection, old_key, new_key)
        .map_err(|error| format!("{error:#}"))?;

    // 启动锁模式下密钥只保存在内存里，不落盘
    let session_only = AppSettings::global(cx).master_key_on_startup_required();
    let switched = if session_only {
        crypto::change_master_key_for_session(old_key, new_key, new_key)
    } else {
        crypto::change_master_key(old_key, new_key, new_key)
    };
    if let Err(error) = switched {
        // 密钥切换失败：把数据库密文退回旧密钥，避免本地出现两套密文
        return match one_core::storage::re_encrypt_secrets(&connection, new_key, old_key) {
            Ok(_) => Err(error.to_string()),
            Err(rollback) => Err(format!("{error}; 数据库密钥回滚也失败: {rollback}")),
        };
    }

    reseal_webdav_password(old_key, new_key, cx);
    let key_version = master_key_meta::bump_key_version();
    let cloud_reencrypt_started =
        start_cloud_reencrypt(old_key, new_key, key_version, notifier, cx);

    Ok(ChangeOutcome {
        connections: stats.connections,
        credentials: stats.credentials,
        team_key_caches: stats.team_key_caches,
        key_version,
        cloud_reencrypt_started,
    })
}

/// WebDAV 密码在 settings.json 里是用主密钥封存的，主密钥换了就得重新封存
fn reseal_webdav_password(old_key: &str, new_key: &str, cx: &mut App) {
    let current = AppSettings::global(cx)
        .personal_sync
        .webdav
        .password
        .clone();
    if current.is_empty() {
        return;
    }
    let resealed = reseal_webdav_password_with_keys(&current, old_key, new_key);
    if resealed == current {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.personal_sync.webdav.password = resealed;
    });
}

/// 把当前主密钥与版本号上报云端（首次设置与修改后都会调用）。
///
/// 这是 `user_configs` 第一次进入生产链路：其他设备同步时先比对版本号，就能
/// 区分"云端主密钥被改过"和"自己输错了密钥"。
pub fn report_key_version_to_cloud(cx: &mut App) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        return;
    };
    let Some(raw_key) = crypto::get_raw_master_key() else {
        return;
    };
    if AppSettings::global(cx).sync_provider != SyncProvider::NavopCloud {
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
        SyncProvider::NavopCloud => {
            let Some(user) = GlobalCloudUser::get_user(cx) else {
                tracing::info!("未登录 Navop Cloud，跳过云端重加密");
                return false;
            };
            let client: Arc<dyn CloudApiClient> = crate::auth::get_auth_service(cx).cloud_client();
            let user_id = user.id.clone();
            let storage = cx.global::<GlobalStorageState>().storage.clone();
            let service = Arc::new(RwLock::new(CloudSyncService::new()));
            if let Ok(mut guard) = service.write() {
                let _ = guard.set_logged_in(user_id.clone());
                let _ = guard.set_master_key_directly(new_key.to_string());
            }
            let old_key = old_key.to_string();
            let new_key = new_key.to_string();

            let task = Tokio::spawn(cx, async move {
                let stats = reencrypt_navop_cloud(
                    &client,
                    &service,
                    &old_key,
                    &new_key,
                    key_version,
                    &user_id,
                )
                .await?;
                // 版本信息已随 user_configs 上传，立刻全量同步一次，
                // 让其他设备尽早看到"云端密钥已更换"
                let engine = SyncEngine::new(client, service, storage);
                let _ = engine.sync().await;
                Ok::<_, SyncError>(stats)
            });

            cx.spawn(async move |cx: &mut AsyncApp| {
                let message = match task.await {
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
                };
                let _ = cx.update(move |cx: &mut App| {
                    if let Some(notify) = notifier.as_ref() {
                        notify(cx, message.clone());
                    }
                });
                Ok::<(), anyhow::Error>(())
            })
            .detach();
            true
        }
    }
}

/// 重置结果
pub struct ResetOutcome {
    pub deleted: Vec<String>,
    pub failed: Vec<String>,
}

/// 执行重置：清本地（按勾选）→ 清云端（按勾选）→ 回到全新安装状态
pub fn reset_master_key_data(scope: ResetScope, cx: &mut App) -> Result<ResetOutcome, String> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let connection = storage.connection();
    let mut outcome = ResetOutcome {
        deleted: Vec::new(),
        failed: Vec::new(),
    };

    let clear_table = |table: &str, outcome: &mut ResetOutcome| {
        let sql = format!("DELETE FROM {table}");
        match connection.with_connection(|conn| Ok(conn.execute_batch(&sql)?)) {
            Ok(()) => outcome.deleted.push(table.to_string()),
            Err(error) => outcome.failed.push(format!("{table}: {error}")),
        }
    };

    for table in REQUIRED_LOCAL_TABLES {
        clear_table(table, &mut outcome);
    }
    if scope.query_history {
        clear_table("sql_execution_history", &mut outcome);
    }
    if scope.command_history {
        clear_table("terminal_command_history", &mut outcome);
        clear_table("quick_commands", &mut outcome);
    }

    // 密钥相关文件：主密钥本体、验证数据与版本信息。
    // 顺序要紧：先清 pending 标记再删元信息文件，否则删完之后再写标记会凭空
    // 生成一个版本为 1 的元信息文件，重置就不算"等同全新安装"了。
    let _ = crypto::reset_repo_password();
    master_key_meta::set_pending_cloud_reencrypt(false);
    master_key_meta::set_pending_cloud_wipe(false);
    master_key_meta::clear();
    AppSettings::update_and_save(cx, |settings| {
        settings.personal_sync.webdav.password = String::new();
    });

    if scope.notes {
        match NotesStorage::configured_root() {
            Ok(root) if root.exists() => match std::fs::remove_dir_all(&root) {
                Ok(()) => outcome.deleted.push("notes".to_string()),
                Err(error) => outcome.failed.push(format!("notes: {error}")),
            },
            Ok(_) => outcome.deleted.push("notes".to_string()),
            Err(error) => outcome.failed.push(format!("notes: {error}")),
        }
    }

    if scope.cloud_sync {
        personal_sync_runtime::wipe_cloud_data(cx);
    }
    if scope.team_data || scope.cloud_sync {
        wipe_navop_cloud(scope.team_data, cx);
    }

    Ok(outcome)
}

/// 清空 Navop Cloud 侧数据：个人记录按 `cloud_sync` 勾选，团队记录按 `team_data` 勾选
fn wipe_navop_cloud(include_team: bool, cx: &mut App) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        return;
    };
    if AppSettings::global(cx).sync_provider != SyncProvider::NavopCloud {
        return;
    }
    let client = crate::auth::get_auth_service(cx).cloud_client();

    let task = Tokio::spawn(cx, async move {
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
        let empty = new_user_config(&user.id, "", 0);
        client
            .save_user_config(&empty)
            .await
            .map_err(|error| format!("{error:?}"))?;
        Ok::<_, String>(deleted)
    });

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
