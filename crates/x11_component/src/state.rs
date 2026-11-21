use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};

use semver::Version;

use crate::{X11ComponentError, X11ForwardingStatus, X11RestartNotification, X11ServerMode};

static CONFIGURED_MODE: AtomicU8 = AtomicU8::new(0);
static RESOLUTION_GENERATION: AtomicU64 = AtomicU64::new(0);
static EMBEDDED_DISABLE_REASON: RwLock<Option<String>> = RwLock::new(None);
static STATUS: OnceLock<RwLock<X11ForwardingStatus>> = OnceLock::new();
static DATA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);
static NAVOP_VERSION: RwLock<Option<Version>> = RwLock::new(None);
static ACTIVE_VERSION: RwLock<Option<String>> = RwLock::new(None);
static RESTART_GENERATION: AtomicU64 = AtomicU64::new(0);
static RESTART_NOTICE: Mutex<Option<X11RestartNotification>> = Mutex::new(None);
static RESTART_NOTICE_CONSUMERS: LazyLock<Mutex<HashSet<u64>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn configured_mode() -> X11ServerMode {
    match CONFIGURED_MODE.load(Ordering::Acquire) {
        1 => X11ServerMode::External,
        2 => X11ServerMode::Embedded,
        _ => X11ServerMode::Auto,
    }
}

pub fn set_configured_mode(mode: X11ServerMode) {
    let value = match mode {
        X11ServerMode::Auto => 0,
        X11ServerMode::External => 1,
        X11ServerMode::Embedded => 2,
    };
    if CONFIGURED_MODE.swap(value, Ordering::AcqRel) != value {
        RESOLUTION_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

/// 模式、组件重启或禁用发生变化时递增，防止新会话复用旧 X11 transport。
pub fn resolution_generation() -> u64 {
    RESOLUTION_GENERATION.load(Ordering::Acquire)
}

/// 注入已限定产品的数据目录，避免再次拼接 navop。
pub fn set_data_dir(path: PathBuf) {
    *DATA_DIR
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(path);
}

/// 应用初始化时注入 apps/navop 自身的 CARGO_PKG_VERSION。
pub fn set_navop_version(version: Version) {
    *NAVOP_VERSION
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(version);
}

pub fn configured_data_dir() -> Result<PathBuf, X11ComponentError> {
    if let Some(path) = DATA_DIR
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
    {
        return Ok(path);
    }
    dirs::data_dir()
        .map(|path| path.join("navop"))
        .ok_or_else(|| X11ComponentError::Config("无法确定用户数据目录".into()))
}

pub fn configured_navop_version() -> Result<Version, X11ComponentError> {
    NAVOP_VERSION
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
        .ok_or_else(|| X11ComponentError::Config("尚未注入 Navop 版本".into()))
}

pub fn current_status() -> X11ForwardingStatus {
    status_lock()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn update_status(status: X11ForwardingStatus) {
    *status_lock()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
}

pub fn active_version() -> Option<String> {
    ACTIVE_VERSION
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

pub fn set_active_version(version: Option<String>) {
    *ACTIVE_VERSION
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = version;
}

pub fn note_restart(reason: String) {
    RESOLUTION_GENERATION.fetch_add(1, Ordering::AcqRel);
    let generation = RESTART_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    *RESTART_NOTICE_CONSUMERS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = HashSet::new();
    *RESTART_NOTICE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) =
        Some(X11RestartNotification { generation, reason });
}

pub fn take_restart_notification() -> Option<X11RestartNotification> {
    take_restart_notification_for(0)
}

/// 每个设置窗口使用自己的 consumer id，避免一个窗口抢走其它窗口的通知。
pub fn take_restart_notification_for(consumer: u64) -> Option<X11RestartNotification> {
    let notification = RESTART_NOTICE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()?;
    let mut consumers = RESTART_NOTICE_CONSUMERS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    consumers.insert(consumer).then_some(notification)
}

#[cfg(test)]
pub fn reset_restart_notifications_for_test() {
    *RESTART_NOTICE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = None;
    RESTART_NOTICE_CONSUMERS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
}

#[cfg(test)]
pub(crate) fn reset_embedded_disable_reason_for_test() {
    *EMBEDDED_DISABLE_REASON
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = None;
}

pub fn embedded_disable_reason() -> Option<String> {
    EMBEDDED_DISABLE_REASON
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

/// 本次运行首次永久禁用的原始原因不被后续修复或回退覆盖。
pub fn disable_embedded(reason: String) -> String {
    let mut current = EMBEDDED_DISABLE_REASON
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    if current.is_none() {
        *current = Some(reason);
        RESOLUTION_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
    current.clone().unwrap_or_else(|| "内置 X11 已禁用".into())
}

#[cfg(test)]
mod tests;

fn status_lock() -> &'static RwLock<X11ForwardingStatus> {
    STATUS.get_or_init(|| {
        RwLock::new(X11ForwardingStatus::Disabled {
            reason: "尚未解析 X11 服务".into(),
        })
    })
}
