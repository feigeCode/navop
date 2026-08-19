//! 密钥存储模块
//!
//! 提供统一的密钥持久化接口。
//! 当前仅保留 `LocalFileStorage`：将主密钥使用程序内置固定 key
//! 进行 AES-256-GCM 加密后写入本地文件。

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

/// 本地加密密钥文件名
const KEY_STORAGE_FILE: &str = "key_storage";
const NONCE_LENGTH: usize = 12;
const AUTH_TAG_LENGTH: usize = 16;
const MIN_ENCRYPTED_LENGTH: usize = NONCE_LENGTH + AUTH_TAG_LENGTH;

/// 本地文件存储使用的固定加密密钥（用于加密本地保存的主密钥）
const LOCAL_STORAGE_FIXED_KEY: &[u8; 32] = b"onehub-local-dev-key-2025-fixed!";

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

/// 本地文件存储实现
///
/// 使用固定密钥对主密钥进行 AES-256-GCM 加密后保存到本地文件。
pub struct LocalFileStorage;

impl KeyStorage for LocalFileStorage {
    fn name(&self) -> &'static str {
        "本地文件"
    }

    fn save(&self, master_key: &str) -> Result<(), String> {
        if !persistent_storage_allowed() {
            return Err("当前运行模式不允许持久化主密钥".to_string());
        }
        let path = get_key_storage_path().ok_or_else(|| "无法获取密钥存储路径".to_string())?;
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
// 全局存储后端管理
// ============================================================================

/// 设置全局密钥存储后端
pub fn set_key_storage(storage: Arc<dyn KeyStorage>) {
    if let Ok(mut guard) = KEY_STORAGE.write() {
        tracing::info!("[密钥存储] 切换到「{}」后端", storage.name());
        *guard = Some(storage);
    }
}

/// 获取当前密钥存储后端
///
/// 如果未设置，默认返回 `LocalFileStorage`。
pub fn get_key_storage() -> Arc<dyn KeyStorage> {
    KEY_STORAGE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| Arc::new(LocalFileStorage))
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 获取数据目录路径
fn get_data_dir() -> Option<PathBuf> {
    crate::app_dirs::data_dir()
}

/// 获取本地密钥存储文件路径
fn get_key_storage_path() -> Option<PathBuf> {
    get_data_dir().map(|data_dir| key_storage_path_for_data_dir(&data_dir))
}

pub(crate) fn key_storage_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(KEY_STORAGE_FILE)
}

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
    use super::{delete_master_key_at_path, load_master_key_from_path, save_master_key_to_path};

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
}
