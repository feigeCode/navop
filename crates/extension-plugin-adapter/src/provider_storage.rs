//! Universal provider 的宿主侧 KV 存储。
//!
//! 此前 `HostApiProvider::storage_get` / `storage_set` 是桩:get 永远返回
//! `None`,set 永远成功。provider 因此无法区分"从没存过"和"宿主根本不支持
//! 存储",订阅持久化只能永远静默失效。本模块把它做成真的:
//!
//! * 每个扩展一个目录 `<root>/<extension_id>/`,扩展只能读写自己的目录;
//! * 每个命名空间一个 JSON 文件,写走"临时文件 + rename",崩溃不会留下半截文件;
//! * 支持 ttl 与字节预算,超限直接报错而不是静默丢数据;
//! * 文件损坏时挪走并重建,不让一次坏盘永久锁死该扩展的持久化。
//!
//! 读写都在 `parking_lot::Mutex` 保护下完成。文件很小、写入罕见(连接级配置),
//! 所以"一把锁换正确性"是划算的:省掉了跨进程/跨 provider 的丢写。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use extension_host::HostError;
use extension_protocol::error::{ProtocolError, error_codes};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

/// 单个 value 序列化后的字节上限。
pub const DEFAULT_MAX_VALUE_BYTES: usize = 256 * 1024;
/// 单个命名空间的字节预算(key + value 之和)。
pub const DEFAULT_MAX_NAMESPACE_BYTES: usize = 4 * 1024 * 1024;
/// key 的最大字节数。
pub const MAX_STORAGE_KEY_BYTES: usize = 512;
/// 命名空间标识的最大字符数。
pub const MAX_NAMESPACE_CHARS: usize = 64;

const STORE_VERSION: i64 = 1;
const CORRUPT_LABEL: &str = "corrupt";
const TEMP_LABEL: &str = "tmp";

/// 存储在磁盘上的格式。
///
/// ```json
/// { "version": 1, "entries": { "<key>": { "value": …, "expires_at_unix_ms": 123 } } }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ProviderStoreLimits {
    pub max_value_bytes: usize,
    pub max_namespace_bytes: usize,
}

impl Default for ProviderStoreLimits {
    fn default() -> Self {
        Self {
            max_value_bytes: DEFAULT_MAX_VALUE_BYTES,
            max_namespace_bytes: DEFAULT_MAX_NAMESPACE_BYTES,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProviderStoreError {
    #[error("storage key is empty or longer than {MAX_STORAGE_KEY_BYTES} bytes")]
    InvalidKey,
    #[error("storage namespace `{0}` is not a valid scope name")]
    InvalidNamespace(String),
    #[error("storage value is {size} bytes, exceeding the {limit} byte limit")]
    ValueTooLarge { size: usize, limit: usize },
    #[error("storage namespace `{namespace}` would exceed its {limit} byte budget")]
    NamespaceTooLarge { namespace: String, limit: usize },
    #[error("storage i/o failed: {0}")]
    Io(String),
}

impl From<ProviderStoreError> for ProtocolError {
    fn from(error: ProviderStoreError) -> Self {
        // 超限/非法参数都是 provider 自己传错,给它 INVALID_PARAMS 让它能自行收敛;
        // 磁盘问题不是它的错,报 INTERNAL_ERROR。
        let code = match error {
            ProviderStoreError::InvalidKey
            | ProviderStoreError::InvalidNamespace(_)
            | ProviderStoreError::ValueTooLarge { .. }
            | ProviderStoreError::NamespaceTooLarge { .. } => error_codes::INVALID_PARAMS,
            ProviderStoreError::Io(_) => error_codes::INTERNAL_ERROR,
        };
        Self::new(code, error.to_string())
    }
}

impl From<ProviderStoreError> for HostError {
    fn from(error: ProviderStoreError) -> Self {
        HostError::protocol(error.into())
    }
}

/// 扩展维度的共享存储句柄。
///
/// `Clone` 共享同一把锁与同一份 root,因此同一个扩展的多个 provider
/// (多个连接)不会互相覆盖写入。
#[derive(Clone)]
pub struct ProviderStore {
    inner: Arc<ProviderStoreInner>,
}

impl std::fmt::Debug for ProviderStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderStore")
            .field("root", &self.inner.root)
            .finish()
    }
}

struct ProviderStoreInner {
    root: PathBuf,
    limits: ProviderStoreLimits,
    /// 覆盖"读文件 → 改 → 写回"的整个临界区。
    guard: Mutex<()>,
}

impl ProviderStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_limits(root, ProviderStoreLimits::default())
    }

    pub fn with_limits(root: impl Into<PathBuf>, limits: ProviderStoreLimits) -> Self {
        Self {
            inner: Arc::new(ProviderStoreInner {
                root: root.into(),
                limits,
                guard: Mutex::new(()),
            }),
        }
    }

    /// 该扩展的存储根目录(`<root>/<extension_id>` 的中间层由调用方决定)。
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// 读取一个 key。不存在或已过期返回 `None`。
    ///
    /// 返回带类型的错误:调用方(host 边界)才知道该映射成哪个协议 code。
    pub fn get(&self, namespace: &str, key: &str) -> Result<Option<Value>, ProviderStoreError> {
        self.get_inner(namespace, key)
    }

    /// 写入一个 key。`ttl_secs` 为 `None` 表示永不过期。
    pub fn set(
        &self,
        namespace: &str,
        key: &str,
        value: Value,
        ttl_secs: Option<u64>,
    ) -> Result<(), ProviderStoreError> {
        self.set_inner(namespace, key, value, ttl_secs)
    }

    fn get_inner(&self, namespace: &str, key: &str) -> Result<Option<Value>, ProviderStoreError> {
        let path = self.namespace_path(namespace)?;
        validate_key(key)?;
        let _guard = self.inner.guard.lock();
        let mut entries = self.load(&path)?;
        if prune_expired(&mut entries, now_unix_ms()) > 0 {
            self.persist(&path, &entries)?;
        }
        Ok(entries.get(key).map(|entry| entry.value.clone()))
    }

    fn set_inner(
        &self,
        namespace: &str,
        key: &str,
        value: Value,
        ttl_secs: Option<u64>,
    ) -> Result<(), ProviderStoreError> {
        let path = self.namespace_path(namespace)?;
        validate_key(key)?;

        let encoded_len = serde_json::to_vec(&value)
            .map_err(|error| ProviderStoreError::Io(error.to_string()))?
            .len();
        if encoded_len > self.inner.limits.max_value_bytes {
            return Err(ProviderStoreError::ValueTooLarge {
                size: encoded_len,
                limit: self.inner.limits.max_value_bytes,
            });
        }

        let now = now_unix_ms();
        let _guard = self.inner.guard.lock();
        let mut entries = self.load(&path)?;
        prune_expired(&mut entries, now);

        let mut candidate = entries.clone();
        candidate.insert(
            key.to_owned(),
            StoredEntry {
                value,
                expires_at_unix_ms: ttl_secs.map(|secs| {
                    now.saturating_add(
                        i64::try_from(secs)
                            .unwrap_or(i64::MAX / 1000)
                            .saturating_mul(1000),
                    )
                }),
            },
        );

        let total: usize = candidate
            .iter()
            .map(|(key, entry)| entry.approx_bytes(key))
            .sum();
        if total > self.inner.limits.max_namespace_bytes {
            return Err(ProviderStoreError::NamespaceTooLarge {
                namespace: namespace.to_owned(),
                limit: self.inner.limits.max_namespace_bytes,
            });
        }

        self.persist(&path, &candidate)?;
        Ok(())
    }

    fn namespace_path(&self, namespace: &str) -> Result<PathBuf, ProviderStoreError> {
        let scope = sanitize_scope(namespace)
            .ok_or_else(|| ProviderStoreError::InvalidNamespace(namespace.to_owned()))?;
        Ok(self.inner.root.join(format!("{scope}.json")))
    }

    /// 读取命名空间文件;不存在、为空、损坏(挪走后重建)都视为空表。
    fn load(&self, path: &Path) -> Result<BTreeMap<String, StoredEntry>, ProviderStoreError> {
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(ProviderStoreError::Io(format!("{path:?}: {error}"))),
        };
        if raw.is_empty() {
            return Ok(BTreeMap::new());
        }
        match decode_entries(&raw) {
            Some(entries) => Ok(entries),
            None => {
                // 坏文件不能永久锁死这个扩展的持久化:挪到一边留证,然后从空表继续。
                let quarantine = path.with_extension(format!("{CORRUPT_LABEL}-{}", now_unix_ms()));
                match fs::rename(path, &quarantine) {
                    Ok(()) => tracing::warn!(
                        path = ?path,
                        quarantine = ?quarantine,
                        "extension storage file is corrupt; moved aside and starting from empty"
                    ),
                    Err(error) => tracing::warn!(
                        path = ?path,
                        error = %error,
                        "extension storage file is corrupt and could not be moved aside; starting from empty"
                    ),
                }
                Ok(BTreeMap::new())
            }
        }
    }

    /// 原子写:临时文件 + rename。rename 在同一目录内,因此是同文件系统替换。
    fn persist(
        &self,
        path: &Path,
        entries: &BTreeMap<String, StoredEntry>,
    ) -> Result<(), ProviderStoreError> {
        let parent = path.parent().ok_or_else(|| {
            ProviderStoreError::Io(format!("storage path {path:?} has no parent directory"))
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| ProviderStoreError::Io(format!("{parent:?}: {error}")))?;

        let body = encode_entries(entries);
        let temp = parent.join(format!(
            ".{}.{TEMP_LABEL}-{}",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "store".to_owned()),
            std::process::id()
        ));
        {
            use std::io::Write;
            let mut file = fs::File::create(&temp)
                .map_err(|error| ProviderStoreError::Io(format!("{temp:?}: {error}")))?;
            file.write_all(&body)
                .map_err(|error| ProviderStoreError::Io(format!("{temp:?}: {error}")))?;
            // 持久化是这层接口存在的全部理由,sync 后再 replace,避免断电留下空文件。
            file.sync_all()
                .map_err(|error| ProviderStoreError::Io(format!("{temp:?}: {error}")))?;
        }
        fs::rename(&temp, path).map_err(|error| {
            let _ = fs::remove_file(&temp);
            ProviderStoreError::Io(format!("{temp:?} -> {path:?}: {error}"))
        })
    }
}

#[derive(Debug, Clone)]
struct StoredEntry {
    value: Value,
    expires_at_unix_ms: Option<i64>,
}

impl StoredEntry {
    fn approx_bytes(&self, key: &str) -> usize {
        key.len()
            + serde_json::to_vec(&self.value)
                .map(|encoded| encoded.len())
                .unwrap_or(0)
    }

    fn is_expired(&self, now_unix_ms: i64) -> bool {
        self.expires_at_unix_ms
            .is_some_and(|deadline| deadline <= now_unix_ms)
    }
}

fn prune_expired(entries: &mut BTreeMap<String, StoredEntry>, now_unix_ms: i64) -> usize {
    let before = entries.len();
    entries.retain(|_, entry| !entry.is_expired(now_unix_ms));
    before - entries.len()
}

fn encode_entries(entries: &BTreeMap<String, StoredEntry>) -> Vec<u8> {
    let mut map = Map::new();
    for (key, entry) in entries {
        let mut record = Map::new();
        record.insert("value".to_owned(), entry.value.clone());
        if let Some(deadline) = entry.expires_at_unix_ms {
            record.insert("expires_at_unix_ms".to_owned(), json!(deadline));
        }
        map.insert(key.clone(), Value::Object(record));
    }
    let body = json!({ "version": STORE_VERSION, "entries": Value::Object(map) });
    serde_json::to_vec_pretty(&body).unwrap_or_else(|_| b"{}".to_vec())
}

fn decode_entries(raw: &[u8]) -> Option<BTreeMap<String, StoredEntry>> {
    let parsed: Value = serde_json::from_slice(raw).ok()?;
    let root = parsed.as_object()?;
    // 版本不认识就当损坏处理:宁可重建也不按错误语义读数据。
    if root.get("version").and_then(Value::as_i64) != Some(STORE_VERSION) {
        return None;
    }
    let entries = root.get("entries")?.as_object()?;
    let mut decoded = BTreeMap::new();
    for (key, record) in entries {
        let record = record.as_object()?;
        let value = record.get("value")?.clone();
        let expires_at_unix_ms = match record.get("expires_at_unix_ms") {
            None | Some(Value::Null) => None,
            Some(deadline) => Some(deadline.as_i64()?),
        };
        decoded.insert(
            key.clone(),
            StoredEntry {
                value,
                expires_at_unix_ms,
            },
        );
    }
    Some(decoded)
}

fn validate_key(key: &str) -> Result<(), ProviderStoreError> {
    if key.is_empty() || key.len() > MAX_STORAGE_KEY_BYTES {
        return Err(ProviderStoreError::InvalidKey);
    }
    Ok(())
}

/// 把任意标识收成一个安全的单层文件名。
///
/// 只允许 `[A-Za-z0-9._-]`,且必须是纯 ASCII、不以 `.` 开头、长度受限 ——
/// 这样 `..`、`/`、空字节、Unicode 反转都不会拼出扩展目录之外的路径。
pub(crate) fn sanitize_scope(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.chars().count() > MAX_NAMESPACE_CHARS || !raw.is_ascii() {
        return None;
    }
    let allowed = raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !allowed || raw.starts_with('.') {
        return None;
    }
    Some(raw.to_owned())
}

/// `<root>/<extension_id>`:扩展只能在自己的目录里读写。
pub fn extension_storage_dir(
    root: &Path,
    extension_id: &str,
) -> Result<PathBuf, ProviderStoreError> {
    let scope = sanitize_scope(extension_id)
        .ok_or_else(|| ProviderStoreError::InvalidNamespace(extension_id.to_owned()))?;
    Ok(root.join(scope))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}
