//! 主密钥修改、KDF 迁移与流程编排（三产品共享）。
//!
//! 这里只放"做什么、按什么顺序做"，界面在 [`super::dialogs`] / [`super::reset`] /
//! [`super::unlock`]。关键顺序约束（原 `crates/navop_app/src/master_key_flow.rs` 文档）：
//!
//! 1. 本地先行：先在一个 SQLite 事务里把连接、钥匙串、团队密钥缓存和冲突快照
//!    全部换成新密钥，成功后才切换验证数据；中途失败本地整体回滚。
//! 2. 云端不回滚：云端重加密依赖网络，失败时记 pending 标记下次补做，而不是把
//!    本地已经自洽的新密钥退回去。
//! 3. 重置等价全新安装（见 [`super::reset`]）。
//!
//! 云端重加密与密钥版本上报依赖宿主产品（云同步 / auth），通过
//! [`crate::vault::VaultHostHooks`] 注入；未注入时只做本地轮换。

use gpui::App;
use one_core::cloud_sync::personal::reseal_webdav_password_with_keys;
use one_core::crypto;
use one_core::master_key_meta;
use one_core::settings::AppSettings;
use one_core::storage::GlobalStorageState;

use crate::vault::host_hooks;

/// 重置确认词。
pub const RESET_CONFIRM_WORD: &str = "RESET";

/// 云端重加密结束后的结果通知：参数是给用户看的提示文案。
pub type CloudReencryptNotifier = Box<dyn Fn(&mut App, String) + 'static>;

/// 修改主密钥的结果。
pub struct ChangeOutcome {
    pub connections: usize,
    pub credentials: usize,
    pub team_key_caches: usize,
    pub key_version: u32,
    /// 本次是否真的启动了云端重加密：启动了才会有"云端重写完成 / 失败"的后续提示
    pub cloud_reencrypt_started: bool,
}

/// 执行主密钥修改：本地轮换 → 切换密钥 → 版本递增 → 启动云端重加密。
pub fn apply_change_master_key(
    old_key: &str,
    new_key: &str,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> Result<ChangeOutcome, String> {
    crypto::validate_master_key_change(old_key, new_key, new_key)
        .map_err(|error| error.to_string())?;

    // 旧版固定盐安装首次改密码时顺带把 KDF 升到 Argon2id：轮换本来就要把每条
    // 密文解出来再加密，不必在改完密码后再跑一轮全量重加密。
    promote_local_kdf(old_key)?;

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
    master_key_meta::set_pending_kdf_upgrade(false);
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

/// 本地验证数据是否仍是旧版固定盐格式。
pub fn local_kdf_upgrade_needed() -> bool {
    crypto::get_current_verification_data()
        .is_some_and(|data| crypto::verification_needs_kdf_upgrade(&data))
}

/// 把本地验证信封切到 Argon2id（先落盘新信封，再激活内存派生规范）。
///
/// 只负责「信封 + 内存派生密钥」的切换，密文轮换由调用方负责。
/// 信封落盘后即使进程崩溃，旧密文仍由 legacy 派生兜底解密。
fn promote_local_kdf(master_key: &str) -> Result<(), String> {
    if !local_kdf_upgrade_needed() {
        return Ok(());
    }
    let upgraded = crypto::create_upgraded_verification(master_key)
        .ok_or_else(|| "生成 Argon2id 验证数据失败".to_string())?;
    let previous = crypto::get_current_verification_data();

    // 先记标记再落盘：留下「信封已切换、密文未轮换」的可恢复状态
    master_key_meta::set_pending_kdf_upgrade(true);
    if !crypto::save_verification(&upgraded) {
        return Err("写入 Argon2id 验证数据失败".to_string());
    }
    crypto::activate_kdf_from_verification(&upgraded);
    if crypto::republish_master_key() {
        return Ok(());
    }

    // 内存里没有主密钥（理论上刚解锁时不会发生）：回滚信封，保持状态一致
    if let Some(previous) = previous {
        crypto::activate_kdf_from_verification(&previous);
        let _ = crypto::save_verification(&previous);
    }
    master_key_meta::set_pending_kdf_upgrade(false);
    Err("刷新主密钥派生状态失败".to_string())
}

/// 解锁后检查并完成 KDF 迁移：SHA-256 + 固定盐 → Argon2id + 随机盐。
///
/// 顺序：升级并落盘验证信封 → 重加密本地密文 → WebDAV 密码重新封存 →
/// 清除待办标记 → 版本号递增 → 云端重加密（失败由既有 pending 机制补做）。
/// 任一步中断，旧密文仍可解，下次解锁凭 `pending_kdf_upgrade` 补做。
pub fn ensure_kdf_upgraded(
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> Result<bool, String> {
    if !local_kdf_upgrade_needed() && !master_key_meta::pending_kdf_upgrade() {
        return Ok(false);
    }
    let master_key =
        crypto::get_raw_master_key().ok_or_else(|| "主密钥未解锁，无法升级 KDF".to_string())?;

    promote_local_kdf(&master_key)?;

    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let connection = storage.connection();
    // 旧密文用 legacy 派生兜底解密，新密文一律使用 Argon2id
    one_core::storage::re_encrypt_secrets(&connection, &master_key, &master_key)
        .map_err(|error| format!("{error:#}"))?;
    reseal_webdav_password(&master_key, &master_key, cx);
    master_key_meta::set_pending_kdf_upgrade(false);

    let key_version = master_key_meta::bump_key_version();
    start_cloud_reencrypt(&master_key, &master_key, key_version, notifier, cx);
    tracing::info!("主密钥 KDF 已升级到 Argon2id，本地密文已重加密（版本 {key_version}）");
    Ok(true)
}

/// 解锁成功后触发 KDF 迁移；失败只记录日志，不阻塞解锁/首次设置。
pub fn run_kdf_upgrade_after_unlock(cx: &mut App) {
    if let Err(error) = ensure_kdf_upgraded(None, cx) {
        tracing::warn!("主密钥 KDF 升级未完成（旧数据仍可读，下次解锁重试）: {error}");
    }
}

/// WebDAV 密码在 settings.json 里是用主密钥封存的，主密钥换了就得重新封存。
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
/// 具体上报由宿主钩子完成；独立产品没有云同步，默认 no-op。
pub fn report_key_version_to_cloud(cx: &mut App) {
    if let Some(hooks) = host_hooks() {
        (hooks.report_key_version)(cx);
    }
}

/// 启动云端重加密。返回是否真的启动了（未注册钩子 / 未登录时为 false）。
fn start_cloud_reencrypt(
    old_key: &str,
    new_key: &str,
    key_version: u32,
    notifier: Option<CloudReencryptNotifier>,
    cx: &mut App,
) -> bool {
    match host_hooks() {
        Some(hooks) => (hooks.reencrypt_cloud)(old_key, new_key, key_version, notifier, cx),
        None => false,
    }
}
