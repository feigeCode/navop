//! 密钥存储模块
//!
//! 提供统一的密钥持久化接口，用于「记住主密码」功能。
//!
//! 支持两种后端，由设置项 `master_key_storage` 选择：
//!
//! - 系统凭据库（默认）：macOS 钥匙串 / Windows 凭据管理器，由操作系统按当前
//!   用户隔离，Navop 不自己保管解密密钥。升级前写入的 `key_storage` 文件使用
//!   程序内置固定密钥加密，等效于明文保存；读取到旧文件后会迁移到系统凭据库并
//!   删除旧文件。
//! - 本机文件：写在数据目录的 `key_storage` 里，沿用旧格式（程序内置固定密钥 +
//!   AES-256-GCM）。它随数据目录一起被复制或备份，任何同时拿到程序和数据目录
//!   的人都能恢复主密钥，因此只在用户显式选择时使用，UI 会给出风险提示。
//!
//! 系统凭据库不可用的平台只提供本机文件后端，不会静默改变用户的选择。

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use rand::RngCore;
use rand::rngs::OsRng;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::settings::MasterKeyStorage;

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
use keyring::Entry;

#[cfg(test)]
use std::sync::Mutex;

/// 本地加密密钥文件名
const KEY_STORAGE_FILE: &str = "key_storage";
const NONCE_LENGTH: usize = 12;
const AUTH_TAG_LENGTH: usize = 16;
const MIN_ENCRYPTED_LENGTH: usize = NONCE_LENGTH + AUTH_TAG_LENGTH;

/// 旧版本地文件存储使用的固定加密密钥
///
/// 仅用于读取升级前的 `key_storage` 文件，禁止用于新保存。
const LOCAL_STORAGE_FIXED_KEY: &[u8; 32] = b"onehub-local-dev-key-2025-fixed!";

/// 系统凭据库中的服务名与账户名
#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
const KEYRING_SERVICE: &str = "com.navop.desktop";
#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
const KEYRING_USER: &str = "master-key";

/// 全局密钥存储后端
static KEY_STORAGE: RwLock<Option<Arc<dyn KeyStorage>>> = RwLock::new(None);

// ============================================================================
// KeyStorage trait
// ============================================================================

/// 密钥存储后端 trait
pub trait KeyStorage: Send + Sync {
    /// 存储后端名称，用于日志标识
    fn name(&self) -> &'static str;

    /// 保存主密钥
    fn save(&self, master_key: &str) -> Result<(), String>;

    /// 加载主密钥
    fn load(&self) -> Option<String>;

    /// 删除存储的密钥
    fn delete(&self) -> Result<(), String>;

    /// 检查是否存在已保存的密钥
    fn exists(&self) -> bool;
}

// ============================================================================
// LocalFileStorage 实现
// ============================================================================

/// 本机文件存储实现
///
/// 沿用旧版 `key_storage` 文件格式（内置固定密钥 + AES-256-GCM），写入数据目录。
/// 由于固定密钥随程序分发，任何同时拿到程序和数据目录的攻击者都能解密，
/// 只有在用户在设置里显式选择「本机文件」时才会写入。
pub struct LocalFileStorage;

impl KeyStorage for LocalFileStorage {
    fn name(&self) -> &'static str {
        "本机文件"
    }

    fn save(&self, master_key: &str) -> Result<(), String> {
        if !persistent_storage_allowed() {
            return Err("当前运行模式不允许持久化主密钥".to_string());
        }
        let path = get_key_storage_path().ok_or_else(|| "无法定位数据目录".to_string())?;
        save_master_key_to_path(&path, master_key)
    }

    fn load(&self) -> Option<String> {
        if !persistent_storage_allowed() {
            return None;
        }
        let path = get_key_storage_path()?;
        load_master_key_from_path(&path)
    }

    fn delete(&self) -> Result<(), String> {
        if let Some(path) = get_key_storage_path() {
            delete_master_key_at_path(&path)?;
        }
        Ok(())
    }

    fn exists(&self) -> bool {
        if !persistent_storage_allowed() {
            return false;
        }
        get_key_storage_path().map(|p| p.exists()).unwrap_or(false)
    }
}

// ============================================================================
// SystemKeyStorage 实现（macOS 钥匙串 / Windows 凭据管理器）
// ============================================================================

/// 系统凭据库是否可用
///
/// Linux 等未接入系统凭据库的平台返回 false：设置页只提供「本机文件」选项。
pub fn system_storage_supported() -> bool {
    cfg!(any(test, target_os = "macos", target_os = "windows"))
}

/// 系统凭据库存储实现
///
/// 主密钥交给操作系统按当前用户加密保存：macOS 走钥匙串，Windows 走凭据管理器。
/// 读取到旧版 `key_storage` 文件时会自动迁移并删除旧文件。
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub struct SystemKeyStorage;

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
impl SystemKeyStorage {
    /// 把旧版 `key_storage` 文件迁移进系统凭据库
    ///
    /// 写入成功后才删除旧文件；写入失败时保留旧文件并返回其中的主密钥，
    /// 保证本次启动仍能解锁，同时日志明确记录迁移失败。
    fn migrate_legacy_file(&self) -> Option<String> {
        let path = get_key_storage_path()?;
        self.migrate_legacy_path(&path)
    }

    fn migrate_legacy_path(&self, path: &Path) -> Option<String> {
        if !path.exists() {
            return None;
        }
        let master_key = load_master_key_from_path(path)?;
        match write_system_secret(&master_key) {
            Ok(()) => {
                match delete_master_key_at_path(path) {
                    Ok(()) => tracing::info!(
                        "[密钥存储] 已将旧版 key_storage 迁移到系统凭据库并删除旧文件"
                    ),
                    Err(error) => {
                        tracing::warn!("[密钥存储] 迁移后删除旧 key_storage 文件失败: {error}")
                    }
                }
                Some(master_key)
            }
            Err(error) => {
                tracing::warn!(
                    "[密钥存储] 写入系统凭据库失败，暂时保留旧 key_storage 文件: {error}"
                );
                Some(master_key)
            }
        }
    }

    /// 删除不安全的旧版固定密钥文件
    fn purge_legacy_file(&self) {
        let Some(path) = get_key_storage_path() else {
            return;
        };
        self.purge_legacy_path(&path);
    }

    fn purge_legacy_path(&self, path: &Path) {
        if !path.exists() {
            return;
        }
        match delete_master_key_at_path(path) {
            Ok(()) => tracing::info!("[密钥存储] 已删除不安全的旧版 key_storage 文件"),
            Err(error) => tracing::warn!("[密钥存储] 删除旧版 key_storage 文件失败: {error}"),
        }
    }
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
impl KeyStorage for SystemKeyStorage {
    fn name(&self) -> &'static str {
        "系统凭据库"
    }

    fn save(&self, master_key: &str) -> Result<(), String> {
        if !persistent_storage_allowed() {
            return Err("当前运行模式不允许持久化主密钥".to_string());
        }
        write_system_secret(master_key)?;
        self.purge_legacy_file();
        tracing::info!("[系统凭据库] 主密钥已保存");
        Ok(())
    }

    fn load(&self) -> Option<String> {
        if !persistent_storage_allowed() {
            return None;
        }
        match read_system_secret() {
            Ok(Some(master_key)) => {
                self.purge_legacy_file();
                tracing::info!("[系统凭据库] 成功读取主密钥");
                Some(master_key)
            }
            Ok(None) => self.migrate_legacy_file(),
            Err(error) => {
                // 系统凭据库不可访问（如钥匙串被锁定）时，退回到旧文件只读，
                // 保证老数据可解锁；但绝不会把新的弱加密副本写回去。
                tracing::warn!("[系统凭据库] 读取主密钥失败: {error}");
                get_key_storage_path()
                    .filter(|path| path.exists())
                    .and_then(|path| load_master_key_from_path(&path))
            }
        }
    }

    fn delete(&self) -> Result<(), String> {
        let mut first_error = None;
        if let Err(error) = delete_system_secret()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Some(path) = get_key_storage_path()
            && let Err(error) = delete_master_key_at_path(&path)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn exists(&self) -> bool {
        if !persistent_storage_allowed() {
            return false;
        }
        read_system_secret().ok().flatten().is_some()
            || get_key_storage_path().is_some_and(|path| path.exists())
    }
}

/// 读取系统凭据库中的主密钥（测试环境使用内存实现，避免写真实钥匙串）
#[cfg(test)]
fn read_system_secret() -> Result<Option<String>, String> {
    Ok(SYSTEM_SECRET_TEST_STORE
        .lock()
        .map_err(|_| "测试凭据库锁已中毒".to_string())?
        .clone())
}

#[cfg(test)]
fn write_system_secret(master_key: &str) -> Result<(), String> {
    *SYSTEM_SECRET_TEST_STORE
        .lock()
        .map_err(|_| "测试凭据库锁已中毒".to_string())? = Some(master_key.to_string());
    Ok(())
}

#[cfg(test)]
fn delete_system_secret() -> Result<(), String> {
    *SYSTEM_SECRET_TEST_STORE
        .lock()
        .map_err(|_| "测试凭据库锁已中毒".to_string())? = None;
    Ok(())
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
fn read_system_secret() -> Result<Option<String>, String> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
fn write_system_secret(master_key: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())?;
    entry
        .set_password(master_key)
        .map_err(|error| error.to_string())
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
fn delete_system_secret() -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
static SYSTEM_SECRET_TEST_STORE: Mutex<Option<String>> = Mutex::new(None);

// ============================================================================
// 全局存储后端管理
// ============================================================================

/// 设置全局密钥存储后端（测试与迁移脚本可直接指定，绕开设置项）
pub fn set_key_storage(storage: Arc<dyn KeyStorage>) {
    if let Ok(mut guard) = KEY_STORAGE.write() {
        tracing::info!("[密钥存储] 切换到「{}」后端", storage.name());
        *guard = Some(storage);
    }
}

/// 「记住主密码」的保存位置偏好，由设置页写入
static PREFERRED_BACKEND: RwLock<Option<MasterKeyStorage>> = RwLock::new(None);

/// 设置保存位置偏好（只影响之后的读写，不搬运已有副本）
pub fn set_preferred_backend(backend: MasterKeyStorage) {
    if let Ok(mut guard) = PREFERRED_BACKEND.write() {
        tracing::info!("[密钥存储] 保存位置切换为 {backend:?}");
        *guard = Some(backend);
    }
}

/// 当前保存位置偏好
pub fn preferred_backend() -> MasterKeyStorage {
    PREFERRED_BACKEND
        .read()
        .ok()
        .and_then(|guard| *guard)
        .unwrap_or_default()
}

/// 按保存位置构造后端（不修改当前偏好）
pub fn storage_for(backend: MasterKeyStorage) -> Arc<dyn KeyStorage> {
    match backend {
        #[cfg(any(test, target_os = "macos", target_os = "windows"))]
        MasterKeyStorage::SystemCredentialStore => Arc::new(SystemKeyStorage),
        #[cfg(not(any(test, target_os = "macos", target_os = "windows")))]
        MasterKeyStorage::SystemCredentialStore => {
            tracing::warn!("当前平台未接入系统凭据库，改用本机文件保存主密钥");
            Arc::new(LocalFileStorage)
        }
        MasterKeyStorage::LocalFile => Arc::new(LocalFileStorage),
    }
}

/// 获取当前密钥存储后端
///
/// 优先使用显式设置的后端，否则按保存位置偏好（默认系统凭据库）构造。
pub fn get_key_storage() -> Arc<dyn KeyStorage> {
    if let Some(storage) = KEY_STORAGE.read().ok().and_then(|guard| guard.clone()) {
        return storage;
    }
    storage_for(preferred_backend())
}

// ============================================================================
// 辅助函数
// ============================================================================

#[cfg(test)]
static TEST_DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// 测试专用：把数据目录临时指向新建的临时目录，执行 `f` 后恢复
///
/// 供本模块与 crypto 的端到端测试共享，避免它们各自维护一套全局状态。
#[cfg(test)]
pub(crate) fn with_test_data_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    if let Ok(mut store) = SYSTEM_SECRET_TEST_STORE.lock() {
        *store = None;
    }
    if let Ok(mut guard) = TEST_DATA_DIR.lock() {
        *guard = Some(temp.path().to_path_buf());
    }
    let result = f(temp.path());
    if let Ok(mut guard) = TEST_DATA_DIR.lock() {
        *guard = None;
    }
    result
}

/// 测试专用：当前是否把数据目录重定向到了临时目录。
///
/// crypto 的验证文件与 master_key_meta 的元信息同样落在数据目录下，必须
/// 跟随同一个重定向，否则单元测试会覆盖真实用户的落盘状态。
#[cfg(test)]
pub(crate) fn test_data_dir_override() -> Option<PathBuf> {
    TEST_DATA_DIR.lock().ok().and_then(|guard| guard.clone())
}

/// 获取数据目录路径
fn get_data_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        test_data_dir_override().or_else(|| Some(crate::app_dirs::test_data_dir()))
    }
    #[cfg(not(test))]
    {
        crate::app_dirs::data_dir()
    }
}

/// 获取本地密钥存储文件路径
fn get_key_storage_path() -> Option<PathBuf> {
    get_data_dir().map(|data_dir| key_storage_path_for_data_dir(&data_dir))
}

pub(crate) fn key_storage_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(KEY_STORAGE_FILE)
}

/// 本机文件格式写入（含旧版兼容读取的同一格式）
fn save_master_key_to_path(path: &Path, master_key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {e}"))?;
    }

    let cipher = Aes256Gcm::new_from_slice(LOCAL_STORAGE_FIXED_KEY)
        .map_err(|e| format!("创建加密器失败: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), master_key.as_bytes())
        .map_err(|e| format!("加密密钥失败: {e}"))?;
    let data = [nonce_bytes.as_slice(), ciphertext.as_slice()].concat();

    atomic_write_file(path, &data).map_err(|e| format!("写入密钥文件失败: {e}"))?;
    tracing::info!("[本地文件] 主密钥已保存");
    Ok(())
}

fn load_master_key_from_path(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;
    if data.len() < MIN_ENCRYPTED_LENGTH {
        tracing::warn!("[本地文件] 密钥文件格式无效");
        return None;
    }

    let cipher = Aes256Gcm::new_from_slice(LOCAL_STORAGE_FIXED_KEY).ok()?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&data[..NONCE_LENGTH]),
                &data[NONCE_LENGTH..],
            )
            .ok()?,
    );
    let master_key = String::from_utf8(plaintext.to_vec()).ok()?;

    tracing::info!("[本地文件] 成功读取密钥");
    Some(master_key)
}

pub(crate) fn atomic_write_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(path);
    let result = write_and_replace(path, &temporary, data);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("secret");
    path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()))
}

fn write_and_replace(path: &Path, temporary: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(temporary)?;
    file.write_all(data)?;
    file.flush()?;
    file.sync_all()?;
    replace_file(temporary, path)?;
    set_owner_only_permissions(path)?;
    sync_parent_directory(path)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{
        MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source = wide_path(temporary);
    let destination = wide_path(destination);
    let flags = MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0);
    unsafe {
        MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr()), flags)
            .map_err(std::io::Error::other)
    }
}

#[cfg(target_os = "windows")]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn delete_master_key_at_path(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除密钥文件失败: {e}"))?;
    }
    Ok(())
}

fn persistent_storage_allowed() -> bool {
    crate::app_paths::initialized_paths().is_none_or(|paths| paths.allows_persistent_master_key())
}

#[cfg(test)]
mod tests {
    use super::{
        KeyStorage, LocalFileStorage, SystemKeyStorage, delete_master_key_at_path,
        load_master_key_from_path, save_master_key_to_path, with_test_data_dir,
    };

    #[test]
    fn local_file_storage_round_trips_without_writing_plaintext() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");
        let master_key = "portable-master-key";

        save_master_key_to_path(&path, master_key).expect("保存主密钥");

        let stored = std::fs::read(&path).expect("读取密钥文件");
        assert!(
            !stored
                .windows(master_key.len())
                .any(|bytes| bytes == master_key.as_bytes())
        );
        assert_eq!(
            Some(master_key.to_string()),
            load_master_key_from_path(&path)
        );
    }

    #[test]
    fn local_file_storage_rejects_corrupted_data() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");
        std::fs::write(&path, b"corrupted").expect("写入损坏数据");

        assert_eq!(None, load_master_key_from_path(&path));
    }

    #[test]
    fn local_file_storage_rejects_nonce_only_and_truncated_tag() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");

        for length in [12, 27] {
            std::fs::write(&path, vec![0_u8; length]).expect("写入截断数据");
            assert_eq!(None, load_master_key_from_path(&path));
        }
    }

    #[test]
    fn local_file_storage_atomically_replaces_an_existing_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");
        save_master_key_to_path(&path, "old-key").expect("保存旧主密钥");

        save_master_key_to_path(&path, "new-key").expect("覆盖主密钥");

        assert_eq!(
            Some("new-key".to_string()),
            load_master_key_from_path(&path)
        );
        let entries = std::fs::read_dir(temp.path())
            .expect("读取临时目录")
            .collect::<Result<Vec<_>, _>>()
            .expect("读取目录项");
        assert_eq!(entries.len(), 1, "不应遗留临时密钥文件");
    }

    #[cfg(unix)]
    #[test]
    fn local_file_storage_uses_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");
        save_master_key_to_path(&path, "portable-master-key").expect("保存主密钥");

        let mode = std::fs::metadata(path)
            .expect("读取密钥文件元数据")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn local_file_storage_delete_removes_the_persisted_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("key_storage");
        save_master_key_to_path(&path, "portable-master-key").expect("保存主密钥");
        assert!(path.exists());

        delete_master_key_at_path(&path).expect("删除主密钥");
        assert!(!path.exists());

        delete_master_key_at_path(&path).expect("重复删除应保持幂等");
    }

    #[test]
    fn local_file_backend_round_trips_when_selected() {
        with_test_data_dir(|data_dir| {
            let storage = LocalFileStorage;
            storage.save("local-key").expect("写入本机文件");
            assert!(data_dir.join("key_storage").exists());
            assert_eq!(Some("local-key".to_string()), storage.load());
            storage.delete().expect("删除本机文件");
            assert!(!storage.exists());
            assert_eq!(None, storage.load());
        });
    }

    #[test]
    fn storage_for_resolves_the_selected_backend() {
        assert_eq!(
            "本机文件",
            super::storage_for(crate::settings::MasterKeyStorage::LocalFile).name()
        );
        #[cfg(any(test, target_os = "macos", target_os = "windows"))]
        assert_eq!(
            "系统凭据库",
            super::storage_for(crate::settings::MasterKeyStorage::SystemCredentialStore).name()
        );
    }

    #[test]
    fn system_backend_round_trips() {
        with_test_data_dir(|_| {
            let storage = SystemKeyStorage;
            assert_eq!(None, storage.load());
            storage.save("system-key").expect("保存到系统凭据库");
            assert!(storage.exists());
            assert_eq!(Some("system-key".to_string()), storage.load());
            storage.delete().expect("删除系统凭据库条目");
            assert!(!storage.exists());
            assert_eq!(None, storage.load());
        });
    }

    #[test]
    fn system_backend_migrates_legacy_file_and_removes_it() {
        with_test_data_dir(|data_dir| {
            let legacy_path = data_dir.join("key_storage");
            save_master_key_to_path(&legacy_path, "legacy-key").expect("写入旧格式密钥");

            let storage = SystemKeyStorage;
            assert_eq!(Some("legacy-key".to_string()), storage.load());
            assert!(!legacy_path.exists(), "迁移成功后应删除旧的固定密钥文件");
            assert_eq!(Some("legacy-key".to_string()), storage.load());
        });
    }

    #[test]
    fn system_backend_save_removes_legacy_file() {
        with_test_data_dir(|data_dir| {
            let legacy_path = data_dir.join("key_storage");
            save_master_key_to_path(&legacy_path, "legacy-key").expect("写入旧格式密钥");

            let storage = SystemKeyStorage;
            storage.save("new-key").expect("保存到系统凭据库");
            assert!(!legacy_path.exists(), "保存新密钥后不应保留弱加密副本");
            assert_eq!(Some("new-key".to_string()), storage.load());
        });
    }

    #[test]
    fn system_backend_delete_removes_legacy_file() {
        with_test_data_dir(|data_dir| {
            let legacy_path = data_dir.join("key_storage");
            save_master_key_to_path(&legacy_path, "legacy-key").expect("写入旧格式密钥");

            let storage = SystemKeyStorage;
            storage.delete().expect("删除所有副本");
            assert!(!legacy_path.exists());
            assert_eq!(None, storage.load());
        });
    }

    #[test]
    fn crypto_remembers_and_restores_through_the_selected_local_file_backend() {
        let _crypto_guard = crate::crypto::crypto_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = super::preferred_backend();
        with_test_data_dir(|data_dir| {
            super::set_preferred_backend(crate::settings::MasterKeyStorage::LocalFile);
            crate::crypto::reset_repo_password();
            crate::crypto::set_master_key("local-backend-key").expect("设置主密钥");
            assert!(
                data_dir.join("key_verification").exists(),
                "验证文件必须跟随测试数据目录，绝不能覆盖真实安装"
            );
            crate::crypto::remember_master_key_for_future_startups().expect("保存到本机文件");
            assert!(
                data_dir.join("key_storage").exists(),
                "选择本机文件后应写入数据目录"
            );

            // 模拟重启：只清内存，再从本机文件恢复
            crate::crypto::clear_in_memory_master_key_for_tests();
            assert!(!crate::crypto::has_master_key());
            assert!(crate::crypto::try_restore_master_key());
            assert_eq!(
                Some("local-backend-key".to_string()),
                crate::crypto::get_raw_master_key()
            );

            crate::crypto::forget_persisted_master_key().expect("删除本机文件副本");
            assert!(!data_dir.join("key_storage").exists());
            crate::crypto::reset_repo_password();
        });
        super::set_preferred_backend(previous);
    }
}
